#!/usr/bin/env python3
r"""Why the synth voice update does not run for half of the SAMPLE programs (D143).

The voice event `0xC01EC690 -> 0xC01E72C4` queues an event that the drain `0xC01E623C` hands to
`0xC01DEAFC` (event scheduler). `0xC01DEAFC` returns 0 (event deferred, voice update never runs)
when the slot counter `[0x90639A50 + 4]` is above 1. That counter is `|signed byte|` of the first
half-word of the table `0xC05A4AB0`, which `0xC01B1840` refills on every note-on from the SPI1
controller (`0x01E12040` data register, SPI read routine `0xC00119A4`, store at `0xC0011AEC`). The
harness maps SPI1 as plain RAM, so the read returns whatever test pattern the firmware wrote
into the neighbourhood last (0, 3, 1, 0x3F, 0x10001, ...), which depends on the run length. This
tool measures that: per program it runs key 60 twice from the same saved state, once as the
harness is (`gated`), once with the SPI read neutralised (`neutral`, `r12 = 0` at the store).

    uv run --with unicorn python tools/arm_voice_gate.py one image sram --eeprom e.bin \
        --out target/arm_voice_gate --ns4b backup.ns4b "<program>.ns4p"
    uv run --with unicorn python tools/arm_voice_gate.py batch ... <program dir>
    python tools/arm_voice_gate.py summary target/arm_voice_gate
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from collections import Counter
from collections.abc import Iterable
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

# VERIFIED (listing, probe run): the two copies of the SPI read loop store `r12` (two half-words
# of the data register) at these pcs; `0xC0011AEC` is the one the note-on path uses.
SPI_FILL_PCS = (0xC0011AEC, 0xC0011930)
# VERIFIED (listing `0xC01DEAFC..0xC01DED2C`): defer when the slot counter is above this.
DEFER_ABOVE = 1
VOICE_UPDATE = 0xC01E00AC
KEY, VELOCITY, KEY_TICKS = 60, 100, 40


def entry_magnitude(word: int) -> int:
    """`|signed byte|` the firmware keeps from the low byte of a table word (`0xC01B19C0..19D0`)."""
    byte = word & 0xFF
    return (256 - byte) if byte >= 0x80 else byte


def defers(word: int) -> bool:
    """True when a table word of the SPI refill makes `0xC01DEAFC` defer the voice event."""
    return entry_magnitude(word) > DEFER_ABOVE


def mode_row(updates: int, sy: int, spi_word: int | None) -> dict[str, object]:
    """One run result: voice update calls, SY messages, the SPI word the refill stored."""
    return {"voice_updates": updates, "sy_msgs": sy, "spi_word": spi_word}


def program_row(name: str, gated: dict, neutral: dict) -> dict[str, object]:
    """Both runs of one program plus the prediction check (`defers` of the stored word)."""
    word = gated["spi_word"]
    return {
        "program": name,
        "gated": gated,
        "neutral": neutral,
        "predicted_stop": None if word is None else defers(word),
        "stopped": gated["voice_updates"] == 0,
        "fixed": gated["voice_updates"] == 0 and neutral["voice_updates"] > 0,
    }


def summarize(rows: Iterable[dict]) -> dict[str, object]:
    """Counts only: stopped, fixed, prediction agreement, SPI word histogram."""
    rows = list(rows)
    stopped = [r for r in rows if r["stopped"]]
    return {
        "programs": len(rows),
        "gated_voiced": sum(not r["stopped"] for r in rows),
        "gated_stopped": len(stopped),
        "neutral_voiced": sum(r["neutral"]["voice_updates"] > 0 for r in rows),
        "fixed": sum(r["fixed"] for r in rows),
        "prediction_agrees": sum(r["predicted_stop"] == r["stopped"] for r in rows),
        "spi_words": dict(Counter(hex(r["gated"]["spi_word"] or 0) for r in rows)),
    }


def read_rows(path: Path) -> list[dict]:
    """Result rows of a batch directory (one JSON file per program) or of one JSON list file."""
    if path.is_file():
        return list(json.loads(path.read_text()))
    return [json.loads(p.read_text()) for p in sorted(path.glob("*.json"))]


def install_spi_neutral(emu, state: dict | None = None) -> dict:  # noqa: ANN001  # pragma: no cover
    """Hook the SPI refill stores: remember the first word, zero it while `state["neutral"]`."""
    state = state if state is not None else {"neutral": True}
    state.setdefault("spi", None)

    def fill(_uc, _a, _s, _u) -> None:  # noqa: ANN001
        if state["spi"] is None:
            state["spi"] = emu.reg(12)
        if state["neutral"]:
            emu.set_reg(12, 0)

    for pc in SPI_FILL_PCS:
        emu.uc.hook_add(emu._u.UC_HOOK_CODE, fill, begin=pc, end=pc)
    return state


def probe(  # pragma: no cover
    code: Path, sram: Path, eeprom: Path, program: Path, sample: bytes
) -> dict:
    """Boot, recall, bind the stand-in sample, play key 60 gated and neutral from one state."""
    import arm_dsp_capture as dc
    import arm_keydown as kd
    import arm_program_load as apl
    import arm_screens as sc

    session = sc.Session(code, sram)
    session.load_eeproms(
        apl.build_image(eeprom.read_bytes(), program.read_bytes(), program.stem[:23])
    )
    kd.install_board_present(session.emu)
    session.boot()
    emu = session.emu
    rig = kd.Rig(emu, kd.Uart2().install(emu))
    emu.install_dsp_ports()
    dc.install_dsp_ack(emu)
    dc.run_phase(rig, dc.LOAD_TICKS, "load")
    dc.run_phase(rig, dc.IDLE_TICKS, "idle")
    dc.bind_samples(emu, sample)
    state = install_spi_neutral(emu, {"neutral": False, "updates": 0, "spi": None})

    def update(_uc, _a, _s, _u) -> None:  # noqa: ANN001
        state["updates"] += 1

    emu.uc.hook_add(emu._u.UC_HOOK_CODE, update, begin=VOICE_UPDATE, end=VOICE_UPDATE)
    session.load_state(session.state_bytes())
    out = {}
    for mode in ("gated", "neutral"):
        session.reset()
        state.update(neutral=mode == "neutral", updates=0, spi=None)
        rows = dc.run_phase(rig, KEY_TICKS, "k", lambda: rig.send(kd.note_on(KEY, VELOCITY)))
        sy = sum(1 for r in rows if " SY " in r)
        out[mode] = mode_row(state["updates"], sy, state["spi"])
    return out


def run_one(ns) -> int:  # noqa: ANN001  # pragma: no cover
    import arm_dsp_capture as dc
    import arm_piano_load as pl

    sample = pl.read_npno(ns.ns4b, dc.SAMPLE_MEMBER, 0x400000)
    t0 = time.perf_counter()
    res = probe(ns.image, ns.sram, ns.eeprom, ns.program, sample)
    row = program_row(dc.out_name(ns.program), res["gated"], res["neutral"])
    ns.out.mkdir(parents=True, exist_ok=True)
    (ns.out / f"{row['program']}.json").write_text(json.dumps(row))
    print(f"{row['program']}: {time.perf_counter() - t0:.0f} s {row}", flush=True)
    return 0


def run_batch(ns) -> int:  # noqa: ANN001  # pragma: no cover
    import arm_dsp_capture as dc

    programs = sorted(ns.programs.rglob("*.ns4p"))
    if ns.only:
        keep = set(ns.only.read_text().split())
        programs = [p for p in programs if dc.out_name(p) in keep]
    done = {p.stem for p in ns.out.glob("*.json")} if ns.out.exists() else set()

    programs = [p for p in programs if dc.out_name(p) not in done]
    cmd = [sys.executable, "-I", str(Path(__file__).resolve()), "one"]

    def work(p: Path) -> int:
        args = [str(ns.image), str(ns.sram), "--eeprom", str(ns.eeprom), "--out", str(ns.out)]
        return subprocess.call([*cmd, *args, "--ns4b", str(ns.ns4b), str(p)])

    with ThreadPoolExecutor(max_workers=ns.workers) as pool:
        codes = list(pool.map(work, programs))
    print(f"done, {sum(bool(c) for c in codes)} failed of {len(programs)}", flush=True)
    return int(any(codes))


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local firmware
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("one", "batch"):
        p = sub.add_parser(name)
        p.add_argument("image", type=Path)
        p.add_argument("sram", type=Path)
        p.add_argument("--eeprom", type=Path, required=True)
        p.add_argument("--out", type=Path, required=True)
        p.add_argument("--ns4b", type=Path, required=True)
        p.add_argument("programs" if name == "batch" else "program", type=Path)
        if name == "batch":
            p.add_argument("--workers", type=int, default=4)
            p.add_argument("--only", type=Path, help="text file of program stems")
    s = sub.add_parser("summary")
    s.add_argument("directory", type=Path)
    ns = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    if ns.cmd == "summary":
        print(json.dumps(summarize(read_rows(ns.directory)), indent=1))
        return 0
    return run_one(ns) if ns.cmd == "one" else run_batch(ns)


if __name__ == "__main__":
    sys.exit(main())
