#!/usr/bin/env python3
r"""Capture the ARM -> DSP message stream of one or all factory programs (D121).

One worker boots the full machine (`arm_screens.Session`) with the program in the Store record
(`arm_program_load.build_image`), attaches the board-present flag and the UART2 MIDI model
(`arm_keydown`), then logs every DSP message of the phases

* `load`  800 1 ms ticks after the boot (program recall plus the initial push to every DSP);
* `idle`  100 ticks;
* `k36`, `k60`, `k84`  note-on of one key (MIDI 36 / 60 / 84, velocity 100), 40 ticks, note-off,
  40 ticks (`<phase>` and `<phase>_off`).

A message is `phase tick chip path core dm w2 pc` (one text line, gzip file per program under the
output directory, git-ignored). The batch mode runs one worker process per program and skips
programs whose file exists, so a killed batch resumes.

    C=target/fw_unpacked
    uv run --with unicorn python tools/arm_dsp_capture.py batch \
        $C/bank0_CODE.bin $C/bank0_SRAM.bin --eeprom target/oled/eeprom.bin \
        --out target/arm_dsp_all --workers 8 \
        --ns4b "<backup>.ns4b" factory_programs/Program
    # one program: replace `batch` by `one` and the last argument by the `.ns4p` path

Summary and verdict: `tools/arm_dsp_summary.py`, `docs/research/arm-dsp-capture.md`. A program
takes about 70 s of CPU (boot, recall, 800 + 100 + 3 x 80 ticks). `--ns4b` binds a library sample
(default `Carina Accordion 1`) to every synth layer whose recall set the SAMPLES flag, because the
harness has no sample library and the recall alone leaves the layer without a sample.
"""

from __future__ import annotations

import argparse
import gzip
import subprocess
import sys
import time
from collections.abc import Iterable, Iterator
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

# ASSUMED (same as `arm_org_stream`): one tick is 1 ms of ARM time, 200,000 instructions.
LOAD_TICKS, IDLE_TICKS, KEY_TICKS = 800, 100, 40
# Three keys spread over the range: low, middle, high (sample pitch should differ per key).
KEYS = (36, 60, 84)
VELOCITY = 100
SAMPLE_MEMBER = "Samp Lib/Accordion_Harm/Carina Accordion 1_PS 4.1.nsmp4"
# EMPIRICAL (D154, `sample-gaps.md`): the 4 MiB default serves only the head of a library sample;
# strokes behind it are missing for the ARM, so voices of those zones are never prepared. A
# library sample is at most 15.5 MB in the factory backup: 16 MiB serves all of it.
MEMBER_BYTES = 0x400000
FIELDS = ("phase", "tick", "chip", "path", "core", "dm", "w2", "pc")


def out_name(program: Path) -> str:
    """File stem of a program capture: `Bank A__Name` (bank directory and program name)."""
    return f"{program.parent.name}__{program.stem}".replace(" ", "_")


def program_files(root: Path) -> list[Path]:
    """Every `.ns4p` under `root`, sorted by path."""
    return sorted(root.rglob("*.ns4p")) if root.is_dir() else [root]


def format_row(phase: str, tick: int, m) -> str:  # noqa: ANN001 - arm_emu.Message
    """One message line: `phase tick chip path core dm w2 pc` (hex words)."""
    return f"{phase} {tick} {m.chip} {m.path} {m.core} {m.dm_word:#x} {m.word2:#010x} {m.pc:#x}"


def parse_row(line: str) -> dict[str, object]:
    """Inverse of `format_row`: `{phase, tick, chip, path, core, dm, w2, pc}` with ints."""
    p = line.split()
    if len(p) != len(FIELDS):
        raise ValueError(f"bad capture row: {line!r}")
    return {
        "phase": p[0],
        "tick": int(p[1]),
        "chip": p[2],
        "path": int(p[3]),
        "core": int(p[4]),
        "dm": int(p[5], 16),
        "w2": int(p[6], 16),
        "pc": int(p[7], 16),
    }


def read_rows(path: Path) -> Iterator[dict[str, object]]:
    """Messages of one capture file."""
    with gzip.open(path, "rt", encoding="utf-8") as fh:
        for line in fh:
            if line.strip():
                yield parse_row(line)


def write_rows(path: Path, lines: Iterable[str]) -> None:
    """Write a capture file atomically (temp name, then rename) so a killed run leaves none."""
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    with gzip.open(tmp, "wt", encoding="utf-8", compresslevel=6) as fh:
        for line in lines:
            fh.write(line + "\n")
    tmp.replace(path)


def pending(programs: list[Path], out: Path) -> list[Path]:
    """Programs without a finished capture file (resume)."""
    return [p for p in programs if not (out / f"{out_name(p)}.txt.gz").exists()]


def install_dsp_ack(emu) -> None:  # noqa: ANN001  # pragma: no cover - firmware
    """Set the DSP status bit the voice update waits for (`arm_sy_voice.play_note`, ASSUMED)."""
    import arm_sy_voice as sv

    def ack(_uc, _a, _s, _u) -> None:  # noqa: ANN001
        address, mask = sv.ack_address(emu.reg(5), emu.reg(6), emu.read32)
        emu.write32(address, emu.read32(address) | mask)

    for pc in sv.ACK_PCS:
        emu.uc.hook_add(emu._u.UC_HOOK_CODE, ack, begin=pc, end=pc)


def run_phase(rig, ticks: int, phase: str, action=None, pump: bool = True) -> list[str]:  # noqa: ANN001
    """Run `ticks` ticks (`action()` first); returns the formatted message rows.

    With `pump` the handler event queue is drained after every tick (`arm_sy_voice.pump`): the
    harness runs no consumer task, so queued synth voice events (the SY traffic) would never run.
    """
    import arm_steady as ast
    import arm_sy_voice as sv
    from arm_org_stream import TICK_INSNS

    rows: list[str] = []
    for n in range(ticks):
        if n == 0 and action:
            action()
        rig.emu.clear_messages()
        ast.run_ticking(rig.emu, TICK_INSNS, per_tick=TICK_INSNS)
        if pump:
            sv.pump(rig.emu)
        rows += [format_row(phase, n, m) for m in rig.emu.messages]
    return rows


def sample_layers_flagged(emu) -> list[int]:  # noqa: ANN001  # pragma: no cover - firmware
    """Synth layer table indexes whose sample flag `[obj + 0x4BC]` the recall set (SAMPLES mode)."""
    import arm_sample_probe as sp

    return [n for n in range(5) if sp.synth_gate(emu, n).status == 1]


def bind_samples(emu, data: bytes) -> list[int]:  # noqa: ANN001  # pragma: no cover - firmware
    """Load one library sample with the real loader and bind it to every flagged layer.

    The recall sets the sample flag and the (hash) sample id but never loads a sample, because the
    harness has no sample library. `data` (any `.nsmp4`; the hash to file map is unknown) is served
    as library entry 1, so the synth voice path can run: `arm_sample_probe` steps, ASSUMED id 1.
    """
    import arm_sample_probe as sp

    layers = sample_layers_flagged(emu)
    if not layers:
        return []
    sp.serve_sample(emu, sp.sample_device_image(data, sp.sample_name(data)))
    if sp.load_sample(emu) != 0:
        raise RuntimeError("sample loader failed")
    for layer in layers:
        sp.bind_layer(emu, layer)
    return layers


def capture(
    code: Path,
    sram: Path,
    eeprom: Path,
    program: Path,
    sample: bytes | None = None,
    spi_neutral: bool = True,
) -> list[str]:  # pragma: no cover
    """Boot with `program` in the Store record and run all phases; returns every row.

    With `sample` (an `.nsmp4` image) it is bound to the flagged synth layers after `load`.
    """
    import arm_keydown as kd
    import arm_program_load as apl
    import arm_screens as sc

    session = sc.Session(code, sram)
    session.load_eeproms(
        apl.build_image(eeprom.read_bytes(), program.read_bytes(), program.stem[:23])
    )
    kd.install_board_present(session.emu)
    session.boot()
    rig = kd.Rig(session.emu, kd.Uart2().install(session.emu))
    session.emu.install_dsp_ports()
    install_dsp_ack(session.emu)
    if spi_neutral:
        # D143: the harness maps SPI1 as RAM, the note-on refill reads test-pattern leftovers.
        import arm_voice_gate as vg

        vg.install_spi_neutral(session.emu)
    rows = run_phase(rig, LOAD_TICKS, "load")
    rows += run_phase(rig, IDLE_TICKS, "idle")
    if sample is not None:
        rows.append(f"bind 0 SY 0 0 {len(bind_samples(session.emu, sample)):#x} 0x0 0x0")
    # Every key starts from the same settled state (as `arm_sy_voice`): a second note-on in one run
    # finds the first voice still held and sends nothing.
    session.load_state(session.state_bytes())
    for key in KEYS:
        session.reset()
        rows += run_phase(
            rig, KEY_TICKS, f"k{key}", lambda k=key: rig.send(kd.note_on(k, VELOCITY))
        )
        rows += run_phase(rig, KEY_TICKS, f"k{key}_off", lambda k=key: rig.send(kd.note_off(k)))
    return rows


def run_one(ns) -> int:  # noqa: ANN001  # pragma: no cover
    program, out = ns.program, ns.out
    sample = None
    if ns.ns4b:
        import arm_piano_load as pl

        sample = pl.read_npno(ns.ns4b, ns.member, ns.member_bytes)
    t0 = time.perf_counter()
    rows = capture(ns.image, ns.sram, ns.eeprom, program, sample, ns.spi_neutral)
    write_rows(out / f"{out_name(program)}.txt.gz", rows)
    print(f"{program.stem}: {len(rows)} messages in {time.perf_counter() - t0:.0f} s", flush=True)
    return 0


def worker_command(ns, program: Path) -> list[str]:
    """Command line of the worker process for one program."""
    return [
        sys.executable, "-I", str(Path(__file__).resolve()), "one",
        str(ns.image), str(ns.sram), "--eeprom", str(ns.eeprom), "--out", str(ns.out),
        *(["--ns4b", str(ns.ns4b), "--member", ns.member] if ns.ns4b else []),
        "--member-bytes", hex(ns.member_bytes),
        *([] if ns.spi_neutral else ["--no-spi-neutral"]),
        str(program),
    ]  # fmt: skip


def run_batch(ns) -> int:  # noqa: ANN001  # pragma: no cover - spawns firmware workers
    programs = pending(program_files(ns.programs), ns.out)
    if ns.limit:
        programs = programs[: ns.limit]
    print(f"{len(programs)} programs to capture, {ns.workers} workers", flush=True)
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
        p.add_argument("--out", type=Path, required=True)
        p.add_argument("--ns4b", type=Path, help="backup archive: bind a library sample (see docs)")
        p.add_argument("--member", default=SAMPLE_MEMBER)
        p.add_argument(
            "--member-bytes",
            type=lambda v: int(v, 0),
            default=MEMBER_BYTES,
            help="bytes of the member served to the ARM (default 4 MiB; 0x1000000 serves all)",
        )
        p.add_argument(
            "--spi-neutral",
            action=argparse.BooleanOptionalAction,
            default=True,
            help="zero the SPI1 refill read the voice scheduler gates on (D143)",
        )
        p.add_argument("programs" if name == "batch" else "program", type=Path)
        if name == "batch":
            p.add_argument("--workers", type=int, default=4)
            p.add_argument("--limit", type=int, default=0)
    ns = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    if ns.cmd == "one":
        return run_one(ns)
    return run_batch(ns)


if __name__ == "__main__":
    sys.exit(main())
