#!/usr/bin/env python3
"""Turn an `arm_dsp_capture.py` file into a `sharc-sys` message script for one DSP core.

A capture row is `phase tick chip path core dm w2 pc`. The script line is `PASS DM W2`, the
format of `--script-flat` / `--script-at` (`crates/nord-sharc/src/runner/msg.rs`). `PASS` is 0 for
every row (flat replay in file order) unless `--pass-per-tick N` maps one ARM tick to N lane passes.

    python tools/arm_capture_script.py CAPTURE.txt.gz --chip PNO --core 1 --phase load > load.script

Numbers only. Evidence: `docs/research/leslie-locate.md` s3.
"""

from __future__ import annotations

import argparse
import sys
from collections.abc import Iterable, Iterator
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
import arm_dsp_capture as cap


def select(
    rows: Iterable[dict[str, Any]], chip: str, core: int, phases: set[str] | None
) -> Iterator[dict[str, Any]]:
    """Rows of `chip` / `core` (and of the listed `phases`, any when `None`), in order."""
    for row in rows:
        if row["chip"] != chip or row["core"] != core:
            continue
        if phases is not None and row["phase"] not in phases:
            continue
        yield row


def script_lines(rows: Iterable[dict[str, Any]], pass_per_tick: int = 0) -> list[str]:
    """`PASS DM W2` lines; PASS = `tick * pass_per_tick` (0 when the factor is 0)."""
    return [
        f"{int(row['tick']) * pass_per_tick} {int(row['dm']):#x} {int(row['w2']):#010x}"
        for row in rows
    ]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("capture", type=Path)
    ap.add_argument("--chip", required=True)
    ap.add_argument("--core", type=int, required=True)
    ap.add_argument("--phase", action="append", help="phase name (repeatable); default all")
    ap.add_argument("--pass-per-tick", type=int, default=0)
    ns = ap.parse_args(argv)
    phases = set(ns.phase) if ns.phase else None
    rows = select(cap.read_rows(ns.capture), ns.chip, ns.core, phases)
    for line in script_lines(rows, ns.pass_per_tick):
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
