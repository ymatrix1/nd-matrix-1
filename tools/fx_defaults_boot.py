#!/usr/bin/env python3
"""Does the ARM start program send the firmware default words to the FX chips? (D101, OQ34)

    python tools/fx_defaults_boot.py target/bulk/boot_log.jsonl [--install fx_live_install]

The boot log (`tools/arm_bulk_tables.py capture`, local only) holds every message the booted ARM
sends to the DSPs. The words of a knob are a function of its UI value, so the value the ARM used
is recovered by looking for the words of each candidate value in the FX writes:

* compressor `kCmpAmt`: makeup `1 + 5.72 (v/127)^2` and threshold `-36 v/127` (`fw_map.rs`),
  inverted from the compressor rows' `c[-0x1C]`, `c[-0x1B]` writes;
* reverb `kRvbAmt`: the wet word of `fx-reverb-amt.csv` (written by a compressor-free search);
* Mod 1 / Mod 2 amount: the words `fx_live_install` exports for the candidate amount.

Prints one line per word. Evidence and limits: `docs/research/fx-defaults-flip.md`.
"""

from __future__ import annotations

import argparse
import csv
import json
import struct
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REVERB = ROOT / "docs/research/data/fx-reverb-amt.csv"
#: compressor instance marker word (attack pair `c[-0x20]`), see `fx_compressor_row.py scan`
CMP_MARK = "0x3dd10ac8"


def f32(w: int) -> float:
    return struct.unpack("<f", struct.pack("<I", w))[0]


def load_fx(path: Path) -> list[dict]:
    """FX-chip rows of the boot log."""
    rows = (json.loads(ln) for ln in path.read_text().splitlines() if ln.strip())
    return [r for r in rows if r.get("chip") == "FX"]


def comp_amounts(fx: list[dict]) -> set[int]:
    """`kCmpAmt` values the boot sent: makeup / threshold pairs of every compressor instance."""
    inst = {(r["core"], int(r["dm"], 16) + 0x20) for r in fx if r["w2"] == CMP_MARK}
    last: dict[tuple[int, int, int], float] = {}
    for r in fx:
        a = int(r["dm"], 16)
        for core, i1 in inst:
            if r["core"] == core and a - i1 in (-0x1C, -0x1B):
                last[(core, i1, a - i1)] = f32(int(r["w2"], 16))
    out = set()
    for (core, i1, off), thr in last.items():
        if off == -0x1B:
            out.add(round(-thr * 127.0 / 36.0))
    return out


def reverb_amounts(fx: list[dict], table: Path = REVERB) -> set[int]:
    """`kRvbAmt` values whose wet word appears in the boot writes (value 0 and 1 excluded: the
    words of the first rows are tiny and also occur as other data)."""
    seen = {int(r["w2"], 16) for r in fx}
    out = set()
    for row in csv.DictReader(table.open()):
        if int(row["value"]) > 1 and int(row["wet_word"]) in seen:
            out.add(int(row["value"]))
    return out


def install_words(exe: Path, slot: int, ty: int, amount: int) -> dict[int, int] | None:
    p = subprocess.run(
        [str(exe), str(slot), str(ty), "64", str(amount)], capture_output=True, text=True
    )
    if p.returncode:
        return None
    return {int(a, 16): int(w, 16) for a, w in (ln.split() for ln in p.stdout.splitlines()[1:])}


def mod_hits(
    fx: list[dict], exe: Path, slot: int, ty: int, amounts: list[int]
) -> dict[int, tuple[int, int]]:
    """Per candidate amount: (words that depend on the amount, how many of them the boot wrote)."""
    seen = {int(r["w2"], 16) for r in fx}
    ref = install_words(exe, slot, ty, 0)
    out: dict[int, tuple[int, int]] = {}
    for amt in amounts:
        d = install_words(exe, slot, ty, amt)
        if ref is None or d is None:
            continue
        uniq = [w for a, w in d.items() if ref.get(a) != w and w != 0]
        out[amt] = (len(uniq), sum(w in seen for w in uniq))
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("log", type=Path)
    ap.add_argument("--install", type=Path, default=ROOT / "target/release/fx_live_install")
    args = ap.parse_args(argv)
    fx = load_fx(args.log)
    print(f"{len(fx)} FX writes")
    print("kCmpAmt sent by the boot:", sorted(comp_amounts(fx)), "(firmware default word 95)")
    print(
        "kRvbAmt wet words in the boot:", sorted(reverb_amounts(fx)), "(firmware default word 38)"
    )
    if args.install.exists() or args.install.with_suffix(".exe").exists():
        for slot, ty in ((1, 0), (1, 1), (2, 0), (2, 4)):
            hits = mod_hits(fx, args.install, slot, ty, [44, 51, 64])
            print(f"Mod {slot} type {ty}: (amount-words, found in boot) {hits} (default word 64)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
