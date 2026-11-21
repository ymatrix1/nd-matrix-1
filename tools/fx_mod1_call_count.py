#!/usr/bin/env python3
"""Frames per call of the Mod 1 routines in the running instrument (OQ38, D186,
`docs/research/fx-mod1-call-count.md`).

    python tools/fx_mod1_call_count.py counts target/bulk
    python tools/fx_mod1_call_count.py probe FX/core1_sram.bin 4 -16

`counts` reads the block-count word (the routine's loop counter) of every Mod 1 row the ARM writes,
from the boot message scripts (`python tools/arm_bulk_tables.py capture ... --out target/bulk`:
`fx_core1.script` for the synth and piano rows, `pno_core1.script` for the organ rows), at the row
addresses of `docs/research/data/fx-type-words.csv`.

`probe` installs one Mod 1 routine in the live FX chain of the oracle (`sharc-fxlive`, 4-frame
pass) with the block-count word at 2 and at 1 and prints, per frame position of the pass, the
relative deviation of the output from the pass-through (0.1866 x input). A count of 1 processes
frame positions 0 and 1 only; positions 2 and 3 stay at 0: one call processes `2 x count` frames.
Needs the built `sharc-fxlive`, `fx_live_install` and the local FX image; only numbers are printed.
"""

from __future__ import annotations

import argparse
import csv
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fx_oracle_live as live  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
#: kFx1Type -> (routine, offset of the block-count word from the row's I1)
ROUTINES = {0: ("A-PAN", -3), 1: ("A-PAN", -3), 2: ("TREM", 4), 3: ("TREM", 4),
            4: ("RM", -16), 5: ("RM", -16), 6: ("A-WAH", 4), 7: ("A-WAH", 4), 8: ("WAH", 7),
            9: ("WAH", 7), 10: ("PUMP", -8), 11: ("PUMP", -8)}  # fmt: skip
#: kind -> script that holds its rows (organ rows live in the PNO image, the rest in the FX image)
SCRIPT = {"synth": "fx_core1.script", "piano": "fx_core1.script", "organ": "pno_core1.script"}
CHAIN_GAIN = 0.1866  # live chain pass-through gain (docs/research/fx-oracle-live.md)


def read_script(path: Path) -> dict[int, int]:
    """`PASS ADDR VALUE` lines of an ARM boot script, last value per address."""
    out: dict[int, int] = {}
    for ln in path.read_text().splitlines():
        p = ln.split()
        if len(p) >= 3:
            out[int(p[1], 16)] = int(p[2], 16)
    return out


def block_counts(
    words_csv: Path, bulk: Path
) -> dict[tuple[str, str], list[tuple[int, int | None]]]:
    """(routine, kind) -> [(row I1, block count the ARM wrote or None)] for types 0..11."""
    scripts = {k: read_script(bulk / v) for k, v in SCRIPT.items()}
    rows: dict[tuple[str, str], set[int]] = {}
    with open(words_csv, newline="") as f:
        for r in csv.DictReader(f):
            if r["slot"] == "Fx1" and r["type"].isdigit() and int(r["type"]) in ROUTINES:
                name, _ = ROUTINES[int(r["type"])]
                rows.setdefault((name, r["kinds"]), set()).add(
                    int(r["dm_word"], 16) - int(r["ctx"])
                )
    out: dict[tuple[str, str], list[tuple[int, int | None]]] = {}
    for (name, kind), i1s in sorted(rows.items()):
        off = next(o for n, o in ROUTINES.values() if n == name)
        out[(name, kind)] = [(i1, scripts[kind].get(i1 + off)) for i1 in sorted(i1s)]
    return out


def cmd_counts(args: argparse.Namespace) -> int:
    words = REPO / "docs/research/data/fx-type-words.csv"
    print("routine kind   row I1 : block count written by the ARM (None = row of another routine)")
    for (name, kind), rows in block_counts(words, Path(args.bulk)).items():
        print(f"{name:6} {kind:6}", "  ".join(f"{i1:#x}: {c}" for i1, c in rows))
    return 0


def deviation_by_position(y: np.ndarray, x: np.ndarray) -> list[float]:
    """RMS of `y - 0.1866 x` over RMS of `0.1866 x`, for each frame position 0..3 of a pass."""
    return [
        float(np.sqrt(np.mean((y[k::4] - CHAIN_GAIN * x[k::4]) ** 2)
                      / np.mean((CHAIN_GAIN * x[k::4]) ** 2)))
        for k in range(4)
    ]  # fmt: skip


def cmd_probe(args: argparse.Namespace) -> int:
    rel = REPO / "target" / "release"
    ty, off = int(args.type), int(args.offset)
    lines = subprocess.run([str(rel / "fx_live_install"), "1", str(ty), "64", "64"],
                           capture_output=True, text=True,
                           check=True).stdout.splitlines()  # fmt: skip
    pc = int(lines[0].split()[1], 16)
    words = [(int(a, 16), int(w, 16)) for a, w in (ln.split() for ln in lines[1:])]
    words += live.install(pc, live.CHOR_I1)
    assert any(a == live.CHOR_I1 + off for a, _ in words), "block-count word not in the install"
    work = Path(tempfile.mkdtemp(prefix="m1cnt_"))
    lead, frames = 4 * 3000, 4 * 6000
    x = (0.2 * np.random.default_rng(3).standard_normal((lead + frames, 2))).astype(np.float32)
    x[:lead] = 0
    live.write_wav(work / "in.wav", x)
    for count in (2, 1):
        w = [(a, count if a == live.CHOR_I1 + off else v) for a, v in words]
        (work / "w.script").write_text(live.script_text(w))
        subprocess.run([str(rel / "sharc-fxlive"), args.image, "--core", "1", "--passes",
                        str((lead + frames) // 4), "--in-wav", str(work / "in.wav"),
                        "--script-flat", str(work / "w.script"), "--wav", str(work / "o.wav")],
                       check=True, capture_output=True)  # fmt: skip
        y = live.read_wav(work / "o.wav")[lead:, 0]
        dev = deviation_by_position(y, x[lead:, 0])
        print(f"type {ty} block count {count}: deviation at frame position 0..3 =",
              " ".join(f"{d:.4f}" for d in dev))  # fmt: skip
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("counts")
    c.add_argument("bulk")
    c.set_defaults(fn=cmd_counts)
    p = sub.add_parser("probe")
    p.add_argument("image")
    p.add_argument("type", help="kFx1Type 0..11")
    p.add_argument(
        "offset", help="block-count word offset from I1 (RM -16, A-WAH 4, WAH 7, PUMP -8)"
    )
    p.set_defaults(fn=cmd_probe)
    args = ap.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
