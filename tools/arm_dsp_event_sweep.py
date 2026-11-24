#!/usr/bin/env python3
r"""ARM -> DSP event sweep: per-layer voice and gate events of every factory program.

For each program a worker boots the full machine (`arm_dsp_capture` setup, program in the Store
record), settles it, holds the chord 60, 64, 67 (velocity 100) for 1.0 s and releases it at 0.7 s,
and logs every ARM -> DSP message with its time in ms since the note-on (24 kHz timer counter
`0x80001DC0`, D130). The Master Clock position is fed as in `arm_arp_events.Recorder` (120 BPM,
ASSUMED scale, so arp step times are inputs; the event structure is the measurement).

`classify` reduces the messages to derived events per layer (A/B/C):

* `voice_on_ms` / `voice_off_ms`  key words `DM(0xC1DCC + 3 voice + layer)` (path 2): on when a
  lane bit (`0x03` of the flag byte) is set, off otherwise (`arm_key_words.is_on`);
* `gate_edges`  float gate levels `DM(0xC1396/7)`, `0xC13E6/7`, `0xC1436/7` (layer A, B, C; Gate
  arp mode toggles them, 1.0 / 0.0, instead of retriggering voices): a record on every change;
* `other`  every other DM address, `{dm: count}`.

Only derived numbers are written (no firmware bytes, no audio): one gz message log per program in
`--work` (git-ignored) so a killed batch resumes, and `merge` builds the committed JSON.

    C=target/fw_unpacked
    uv run --with unicorn python tools/arm_dsp_event_sweep.py batch $C/bank0_CODE.bin \
        $C/bank0_SRAM.bin --eeprom target/oled/eeprom.bin --work target/arm_dsp_events \
        --workers 8 --only-csv docs/research/data/arp-morph-flip-sweep.csv \
        factory_programs/Program
    uv run python tools/arm_dsp_event_sweep.py merge --work target/arm_dsp_events \
        --out docs/research/data/arm-dsp-events.json
"""

from __future__ import annotations

import argparse
import csv
import json
import struct
import subprocess
import sys
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

TICK_HZ = 24_000  # VERIFIED (D130): the 24 kHz timer counter `0x80001DC0`
VOICE_BASE = 0xC1DCC  # VERIFIED (`arm_key_words`, `keydown-zones.md` s7): key words, 3 per voice
VOICE_SPAN = 16
LANE_BITS = 0x03
# EMPIRICAL (PR 633): Gate arp mode toggles these float levels, 2/3 duty; 0x50 words per layer.
GATE_BASES = {"A": 0xC1396, "B": 0xC13E6, "C": 0xC1436}
GATE_WORDS = {base + i: layer for layer, base in GATE_BASES.items() for i in (0, 1)}
BLOCK_WORDS = 0x50  # ASSUMED: gate bases are 0x50 words apart, one block per layer
LAYERS = ("A", "B", "C")
SECONDS, RELEASE_AT, TEMPO = 1.0, 0.7, 120.0  # ASSUMED tempo scale, see `arm_arp_events`
KEYS = (60, 64, 67)
VELOCITY = 100


def out_name(program: Path) -> str:
    """File stem of a program log: `Bank A__Name` with underscores."""
    return f"{program.parent.name}__{program.stem}".replace(" ", "_")


def program_id(program: Path) -> str:
    """Program name as the committed JSON writes it: `Bank A/Name`."""
    return f"{program.parent.name}/{program.stem}"


def as_float(word: int) -> float:
    """The IEEE-754 single of a 32-bit word."""
    return struct.unpack("<f", struct.pack("<I", word & 0xFFFFFFFF))[0]


def key_word_layer(dm: int) -> str | None:
    """Layer of a voice key word address, or `None` outside the key word range."""
    if not VOICE_BASE <= dm < VOICE_BASE + VOICE_SPAN:
        return None
    return LAYERS[(dm - VOICE_BASE) % 3]


def is_on(word: int) -> bool:
    """A key word turns a voice on when a lane bit of its flag byte is set (`arm_key_words`)."""
    return bool((word >> 24) & LANE_BITS)


def block_layer(dm: int) -> str | None:
    """Layer whose 0x50-word block (starting at its gate base) holds `dm`, ASSUMED stride."""
    for layer, base in GATE_BASES.items():
        if base <= dm < base + BLOCK_WORDS:
            return layer
    return None


def classify(rows: list[tuple[int, int, int, int]]) -> dict:
    """Per-layer events of `(t_ms, path, dm, w2)` messages (time order).

    Gate words keep their last value per address, so only changes are edges. Other addresses
    count under their layer block (`block_layer`) or, outside all blocks, under `unassigned`.
    """
    layers = {
        n: {"voice_on_ms": [], "voice_off_ms": [], "gate_edges": [], "other": {}} for n in LAYERS
    }
    unassigned: Counter[int] = Counter()
    per_layer: dict[str, Counter[int]] = {n: Counter() for n in LAYERS}
    last: dict[int, int] = {}
    for t_ms, _path, dm, w2 in rows:
        if lay := key_word_layer(dm):
            layers[lay]["voice_on_ms" if is_on(w2) else "voice_off_ms"].append(t_ms)
        elif dm in GATE_WORDS:
            if last.get(dm) != w2:
                edge = {"t_ms": t_ms, "dm": f"{dm:#x}", "value": as_float(w2)}
                layers[GATE_WORDS[dm]]["gate_edges"].append(edge)
            last[dm] = w2
        elif lay := block_layer(dm):
            per_layer[lay][dm] += 1
        else:
            unassigned[dm] += 1
    for n in LAYERS:
        layers[n]["other"] = hex_counts(per_layer[n])
    return {"layers": layers, "unassigned": hex_counts(unassigned)}


def hex_counts(counts: Counter[int]) -> dict[str, int]:
    """`{dm_hex: count}` sorted by address."""
    return {f"{dm:#x}": n for dm, n in sorted(counts.items())}


def period_ms(times: list[int]) -> float | None:
    """Mean interval of consecutive times, `None` below two events."""
    if len(times) < 2:
        return None
    return (times[-1] - times[0]) / (len(times) - 1)


def entry(program: str, rows: list[tuple[int, int, int, int]]) -> dict:
    """JSON entry of one program: `{program, layers: {A: {...}}, unassigned: {dm: count}}`.

    `unassigned` is an extra key beside the agreed schema: repeating DM addresses outside every
    layer block (consumers that read only `program` and `layers` ignore it).
    """
    res = classify(rows)
    return {"program": program, "layers": res["layers"], "unassigned": res["unassigned"]}


def parse_log(lines) -> list[tuple[int, int, int, int]]:  # noqa: ANN001
    """Rows `t_ms path dm w2` (the worker log) as ints, skipping blank lines."""
    rows = []
    for line in lines:
        p = line.split()
        if p:
            rows.append((int(p[0]), int(p[1]), int(p[2], 16), int(p[3], 16)))
    return rows


def arp_programs(csv_path: Path) -> list[str]:
    """Program ids (`Bank A/Name`) of the sweep CSV rows with arp layers."""
    with csv_path.open(newline="", encoding="utf-8") as fh:
        return [r["program"] for r in csv.DictReader(fh) if r["arp_layers"].strip()]


def order_programs(programs: list[Path], first: list[str]) -> list[Path]:
    """`first` ids lead (in the given order), the rest follow sorted."""
    rank = {p: i for i, p in enumerate(first)}
    return sorted(programs, key=lambda p: (rank.get(program_id(p), len(rank)), str(p)))


def pending(programs: list[Path], work: Path) -> list[Path]:
    """Programs without a finished log (resume)."""
    return [p for p in programs if not (work / f"{out_name(p)}.txt.gz").exists()]


def program_files(root: Path) -> list[Path]:
    """Every `.ns4p` under `root`."""
    return sorted(root.rglob("*.ns4p")) if root.is_dir() else [root]


def merge(work: Path) -> list[dict]:
    """Entries of all finished logs in `work`, sorted by program id."""
    import gzip

    entries = []
    for path in sorted(work.glob("*.txt.gz")):
        with gzip.open(path, "rt", encoding="utf-8") as fh:
            head, *body = fh.read().splitlines()
        entries.append(entry(head.removeprefix("# program ").strip(), parse_log(body)))
    return sorted(entries, key=lambda e: e["program"])


def run_one(ns) -> int:  # noqa: ANN001  # pragma: no cover - needs the local firmware
    """Worker: boot, hold the chord, write the message log."""
    import arm_arp_events as ev
    import arm_dsp_capture as dc
    import arm_keydown as kd
    import arm_program_load as apl
    import arm_screens as sc
    import arm_steady as ast
    import arm_sy_voice as sv
    import arm_voice_gate as vg

    program = ns.program
    session = sc.Session(ns.image, ns.sram)
    session.load_eeproms(
        apl.build_image(ns.eeprom.read_bytes(), program.read_bytes(), program.stem[:23])
    )
    kd.install_board_present(session.emu)
    session.boot()
    emu = session.emu
    rig = kd.Rig(emu, kd.Uart2().install(emu))
    emu.install_dsp_ports()
    dc.install_dsp_ack(emu)
    vg.install_spi_neutral(emu)
    rec = ev.Recorder(emu, TEMPO)
    dc.run_phase(rig, dc.LOAD_TICKS, "load")
    dc.run_phase(rig, dc.IDLE_TICKS, "idle")
    session.load_state(session.state_bytes())
    session.reset()
    rec.restart_masters()
    ast.run_ticking(emu, ev.INSNS_PER_TICK * 4, per_tick=ev.INSNS_PER_TICK)
    for key in KEYS:
        rig.send(kd.note_on(key, VELOCITY))
    t_on = rec.now()
    lines = [f"# program {program_id(program)}"]
    for tick in range(int(SECONDS * ev.TICK_HZ)):
        if tick == int(RELEASE_AT * ev.TICK_HZ):
            for key in KEYS:
                rig.send(kd.note_off(key))
        emu.clear_messages()
        ast.run_ticking(emu, ev.INSNS_PER_TICK, per_tick=ev.INSNS_PER_TICK)
        sv.pump(emu)
        ms = (rec.now() - t_on) * 1000 // TICK_HZ
        lines += [f"{ms} {m.path} {m.dm_word:#x} {m.word2:#010x}" for m in emu.messages]
    dc.write_rows(ns.work / f"{out_name(program)}.txt.gz", lines)
    print(f"{program.stem}: {len(lines) - 1} messages", flush=True)
    return 0


def worker_command(ns, program: Path) -> list[str]:  # noqa: ANN001
    """Command line of the worker process for one program."""
    return [
        sys.executable, "-I", str(Path(__file__).resolve()), "one",
        str(ns.image), str(ns.sram), "--eeprom", str(ns.eeprom), "--work", str(ns.work),
        str(program),
    ]  # fmt: skip


def run_batch(ns) -> int:  # noqa: ANN001  # pragma: no cover - spawns firmware workers
    programs = program_files(ns.programs)
    first = arp_programs(ns.only_csv) if ns.only_csv else []
    programs = pending(order_programs(programs, first), ns.work)
    if ns.limit:
        programs = programs[: ns.limit]
    print(f"{len(programs)} programs, {ns.workers} workers", flush=True)
    with ThreadPoolExecutor(max_workers=ns.workers) as pool:
        codes = list(pool.map(lambda p: subprocess.call(worker_command(ns, p)), programs))
    failed = [p.stem for p, c in zip(programs, codes, strict=True) if c]
    print(f"done, {len(failed)} failed: {failed}", flush=True)
    return int(bool(failed))


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local firmware
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("one", "batch"):
        p = sub.add_parser(name)
        p.add_argument("image", type=Path)
        p.add_argument("sram", type=Path)
        p.add_argument("--eeprom", type=Path, required=True)
        p.add_argument("--work", type=Path, required=True)
        p.add_argument("programs" if name == "batch" else "program", type=Path)
        if name == "batch":
            p.add_argument("--workers", type=int, default=8)
            p.add_argument("--limit", type=int, default=0)
            p.add_argument("--only-csv", type=Path, help="arp programs of this CSV run first")
    mp = sub.add_parser("merge")
    mp.add_argument("--work", type=Path, required=True)
    mp.add_argument("--out", type=Path, required=True)
    ns = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    if ns.cmd == "one":
        return run_one(ns)
    if ns.cmd == "batch":
        return run_batch(ns)
    entries = merge(ns.work)
    ns.out.write_text(json.dumps(entries, indent=1) + "\n", encoding="utf-8")
    print(f"{len(entries)} programs -> {ns.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
