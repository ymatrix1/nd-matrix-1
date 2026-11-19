#!/usr/bin/env python3
"""Capture all SAMPLE programs with the whole library sample served to the ARM (D154).

`arm_dsp_capture.py one` serves the first 4 MiB of a library sample (`MEMBER_BYTES`); strokes behind
that offset are missing for the ARM and their voices are never prepared. This launcher resolves the
first SAMPLE member of every program (`smp_stream resolve`) and captures it with `--member-bytes
0x1000000` (all of it), `--workers` processes at a time, resuming finished programs.

    uv run --with unicorn python tools/sample_gaps_cap.py --backup B.ns4b --out target/sg/cap \
        --smp-stream target/release/smp_stream.exe --workers 6 factory_programs/Program
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

FULL = "0x1000000"


def stem_of(program: Path) -> str:
    """Capture stem `Bank_A__Name` of a program file (as `arm_dsp_capture.out_name`)."""
    return f"{program.parent.name}__{program.stem}".replace(" ", "_")


def first_member(resolved: dict[str, str | None]) -> str | None:
    """First SAMPLE layer member (layer order A, B, C)."""
    return next((resolved[k] for k in ("synth_a", "synth_b", "synth_c") if resolved.get(k)), None)


def done(out: Path, program: Path) -> bool:
    """Whether the capture file of `program` exists (resume)."""
    stem = stem_of(program)
    return (out / stem / f"{stem}.txt.gz").exists()


def command(ns: argparse.Namespace, program: Path, member: str) -> list[str]:
    """`arm_dsp_capture.py one` command line for one program."""
    fw = ns.fw_dir
    return [
        sys.executable, "-I", str(Path(__file__).resolve().with_name("arm_dsp_capture.py")), "one",
        str(fw / "bank0_CODE.bin"), str(fw / "bank0_SRAM.bin"), "--eeprom", str(ns.eeprom),
        "--out", str(ns.out / stem_of(program)), "--ns4b", str(ns.backup), "--member", member,
        "--member-bytes", FULL, str(program),
    ]  # fmt: skip


def run_one(ns: argparse.Namespace, program: Path) -> int:  # pragma: no cover - spawns firmware
    if done(ns.out, program):
        return 0
    entry = f"Program/{program.parent.name}/{program.stem}.ns4p"
    r = subprocess.run(
        [str(ns.smp_stream), "resolve", str(ns.backup), entry],
        capture_output=True, text=True, check=False,
    )  # fmt: skip
    member = first_member(json.loads(r.stdout)) if r.returncode == 0 else None
    if member is None:
        return 0
    log = ns.out / f"{stem_of(program)}.log"
    ns.out.mkdir(parents=True, exist_ok=True)
    with log.open("w") as fh:
        return subprocess.call(command(ns, program, member), stdout=fh, stderr=subprocess.STDOUT)


def main(argv: list[str] | None = None) -> int:  # pragma: no cover
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--backup", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--smp-stream", type=Path, required=True)
    ap.add_argument("--fw-dir", type=Path, default=Path("target/fw_unpacked"))
    ap.add_argument("--eeprom", type=Path, default=Path("target/oled/eeprom.bin"))
    ap.add_argument("--workers", type=int, default=6)
    ap.add_argument("programs", type=Path)
    ns = ap.parse_args(argv)
    ns.fw_dir, ns.eeprom = ns.fw_dir.resolve(), ns.eeprom.resolve()
    ns.out = ns.out.resolve()
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    cov = Path(__file__).resolve().parent.parent / "docs/research/data/sample-stream-coverage.json"
    names = {r["program"] for r in json.loads(cov.read_text(encoding="utf-8"))["programs"]}
    files = [p for p in sorted(ns.programs.rglob("*.ns4p")) if stem_of(p) in names]
    with ThreadPoolExecutor(ns.workers) as pool:
        codes = list(pool.map(lambda p: run_one(ns, p), files))
    print(f"{len(files)} programs, {sum(1 for c in codes if c)} failed", flush=True)
    return int(any(codes))


if __name__ == "__main__":
    sys.exit(main())
