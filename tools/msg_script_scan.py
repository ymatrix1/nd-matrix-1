#!/usr/bin/env python3
r"""Scan `sharc-sys --msg-script` / `--script-flat` ARM to DSP message files (D190).

A message line is `PASS 0xDMADDR 0xVALUE [# comment]` (`arm_dsp_link.script_text`; boot logs use
the same layout with a core column). Three questions, numbers only (no firmware content):

* `region LO HI`: every store whose DM address is in `[LO, HI)` (the B3 Leslie chain region of the
  PNO core 1 image is `0xC2E00..0xC3100`).
* `codes`: the distinct values in the PNO core 1 code window `0x120000..0x128000` with the DM
  addresses they are written to (a routine pointer the ARM installs is one of these).
* `value V`: every store of the value `V` (raw routine pc or `0x28000000 + 4 pc` code record).

    python tools/msg_script_scan.py docs/research/data/org-system-load-pno1.script codes
    python tools/msg_script_scan.py docs/research/data/org-system-load-pno1.script \
        region 0xC2E00 0xC3100

Evidence: `docs/research/b3-leslie-install-hunt.md`.
"""

from __future__ import annotations

import argparse
import sys
from collections import defaultdict
from collections.abc import Iterable
from pathlib import Path

# VERIFIED: PNO core 1 instruction window of the B3 chain and slot routines (pc words, D139).
CODE_LO, CODE_HI = 0x120000, 0x128000
CODE_RECORD_BASE = 0x28000000

Row = tuple[int, int, int]  # (first column, DM word address, value)


def parse_rows(lines: Iterable[str]) -> list[Row]:
    """Lines `N 0xADDR 0xVALUE` to `(N, addr, value)`; comments and bad lines are skipped."""
    rows: list[Row] = []
    for line in lines:
        parts = line.split("#", 1)[0].split()
        if len(parts) < 3 or not parts[0].isdigit():
            continue
        try:
            rows.append((int(parts[0]), int(parts[1], 0), int(parts[2], 0)))
        except ValueError:
            continue
    return rows


def region_hits(rows: Iterable[Row], lo: int, hi: int) -> list[Row]:
    """Stores with `lo <= address < hi`."""
    return [r for r in rows if lo <= r[1] < hi]


def value_hits(rows: Iterable[Row], value: int) -> list[Row]:
    """Stores of `value` (raw pc) or of its code record `0x28000000 + 4 * value`."""
    forms = {value, CODE_RECORD_BASE + 4 * value}
    return [r for r in rows if r[2] in forms]


def code_census(rows: Iterable[Row]) -> dict[int, list[int]]:
    """Values in the code window mapped to the sorted distinct addresses they are stored to."""
    seen: dict[int, set[int]] = defaultdict(set)
    for _, addr, value in rows:
        if CODE_LO <= value < CODE_HI:
            seen[value].add(addr)
    return {v: sorted(a) for v, a in sorted(seen.items())}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("script", type=Path)
    ap.add_argument("mode", choices=["region", "codes", "value"])
    ap.add_argument("args", nargs="*", help="region: LO HI; value: V")
    ns = ap.parse_args(argv)
    rows = parse_rows(ns.script.read_text(encoding="utf-8", errors="replace").splitlines())
    print(f"{len(rows)} messages")
    if ns.mode == "region":
        lo, hi = (int(x, 0) for x in ns.args[:2])
        hits = region_hits(rows, lo, hi)
        print(f"{len(hits)} stores in [{lo:#x}, {hi:#x})")
        for n, addr, value in hits[:40]:
            print(f"  {n} {addr:#x} {value:#x}")
    elif ns.mode == "value":
        hits = value_hits(rows, int(ns.args[0], 0))
        print(f"{len(hits)} stores of {ns.args[0]} (raw or code record)")
        for n, addr, value in hits[:40]:
            print(f"  {n} {addr:#x} {value:#x}")
    else:
        census = code_census(rows)
        print(f"{len(census)} distinct code-window values")
        for value, addrs in census.items():
            print(f"  {value:#x} x{len(addrs)} at {', '.join(f'{a:#x}' for a in addrs[:6])}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
