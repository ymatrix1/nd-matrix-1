#!/usr/bin/env python3
"""Context words `c[o]` (`DM(o, I1)` / `DM(o, I3)`) the multi-tap reverb module reads and writes.

Scans the `sharc-disasm` listing of the three routines of module `0xC02BB44C` (core 2 pcs, the
library copies are identical in core 1): the wrapper `0x120FDB`, the packed core `0x120D8A` and the
second routine `0x1211C1`. Only immediate offsets from `I1` (the context) and `I3` (`I3 = I1` in the
core) are listed; other index registers (`I2`, `I4`, `I9`, ...) address buffers, not context words.

    python tools/ns4_dsp_ldr.py target/DSP/dsp_image_0_FX.bin --sram DIR
    sharc-disasm DIR/core2_sram.bin 0x241B14 0x2423C4 > rvbm.lst
    python tools/fx_rvbm_ctx_access.py rvbm.lst --out docs/research/data/fx-rvbm-ctx-access.csv

The CSV holds routine names, pcs and offsets only. Evidence: `docs/research/reverb-stage-gain.md`.
"""

from __future__ import annotations

import argparse
import csv
import re
import sys
from pathlib import Path

#: VERIFIED (ARM routine directory, `docs/research/fx-chain-install.md` s2 run 39): byte address
#: ranges of the listing (`pc * 2`) per routine.
ROUTINES = (("core_0x120d8a", 0x241B14, 0x241FB6), ("wrapper_0x120fdb", 0x241FB6, 0x242382),
            ("second_0x1211c1", 0x242382, 0x2423C4))  # fmt: skip
FIELDS = ["routine", "pc", "kind", "base", "offset"]
_LINE = re.compile(r"^0x([0-9a-f]+)\s+\S+\s+(?:[0-9a-f]{4}\s+)+(.*)$")
_LOAD = re.compile(r"=\s*DM\((-?\d+),\s*(I[13])\)")
_STORE = re.compile(r"DM\((-?\d+),\s*(I[13])\)\s*=")


def routine_of(addr: int) -> str | None:
    """Name of the routine whose byte range holds `addr`."""
    for name, lo, hi in ROUTINES:
        if lo <= addr < hi:
            return name
    return None


def scan(text: str) -> list[dict[str, object]]:
    """Context accesses of every listing line: one row per `DM(o, I1|I3)` load or store."""
    rows: list[dict[str, object]] = []
    for line in text.splitlines():
        m = _LINE.match(line)
        if not m:
            continue
        addr, body = int(m.group(1), 16), m.group(2)
        name = routine_of(addr)
        if name is None:
            continue
        for kind, rx in (("store", _STORE), ("load", _LOAD)):
            for off, base in rx.findall(body):
                rows.append({"routine": name, "pc": f"{addr // 2:#x}", "kind": kind,
                             "base": base, "offset": int(off)})  # fmt: skip
    return rows


def offsets(rows: list[dict[str, object]], routine: str, kind: str) -> set[int]:
    """Context offsets that `routine` loads (`kind` `load`) or stores (`store`)."""
    return {int(r["offset"]) for r in rows if r["routine"] == routine and r["kind"] == kind}


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - thin CLI
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("listing", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args(argv)
    rows = scan(a.listing.read_text())
    with a.out.open("w", encoding="utf-8", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=FIELDS)
        w.writeheader()
        w.writerows(rows)
    print(len(rows), "accesses")
    return 0


if __name__ == "__main__":
    sys.exit(main())
