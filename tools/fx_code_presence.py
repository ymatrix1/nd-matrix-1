#!/usr/bin/env python3
"""Is a routine of one SHARC image present as code in another image? Numbers only.

A routine is a window of `LEN` bytes at `2 * pc` of a flat SRAM image (`ns4_dsp_ldr.py --sram`,
byte offset = 2 x short-word address). The window is cut into overlapping `W`-byte pieces (step 2,
one VISA parcel); a piece is *present* when the same bytes occur anywhere in the target image.
Absolute addresses inside the routine break only the pieces that contain them, so a copy of the
routine linked at another address still scores close to 1.0, while code that is not in the target
scores close to 0.0.

    python tools/fx_code_presence.py PNO_core1_sram.bin 0x1256AD:leslie 0x1237F8:cabB \
        --target FX_core1_sram.bin --target FX_core2_sram.bin

Prints `routine target present_fraction` rows. No instruction, constant or sample is printed.
Evidence: `docs/research/b3-leslie-fx-chip.md`.
"""

from __future__ import annotations

import argparse
from collections.abc import Iterable
from pathlib import Path

W = 12  # bytes per piece (6 parcels)
LEN = 0x300  # bytes of routine body compared


def pieces(ref: bytes, pc: int, length: int = LEN, width: int = W) -> list[bytes]:
    """Overlapping `width`-byte pieces (step 2) of `length` bytes starting at `2 * pc`."""
    start = 2 * pc
    body = ref[start : start + length]
    return [body[i : i + width] for i in range(0, len(body) - width + 1, 2)]


def presence(ref: bytes, pc: int, target: bytes, length: int = LEN, width: int = W) -> float:
    """Fraction of the routine's pieces that occur in `target` (0.0 when the window is empty)."""
    ps = pieces(ref, pc, length, width)
    if not ps:
        return 0.0
    return sum(1 for p in ps if target.find(p) >= 0) / len(ps)


def parse_routine(text: str) -> tuple[int, str]:
    """`PC[:name]` -> (pc, name)."""
    pc, _, name = text.partition(":")
    return int(pc, 0), name or pc


def rows(ref: bytes, routines: Iterable[tuple[int, str]], targets: dict[str, bytes]):
    """`(name, target, fraction)` for every routine and target image."""
    for pc, name in routines:
        for tname, data in targets.items():
            yield name, tname, presence(ref, pc, data)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("reference", help="flat SRAM image that holds the routines")
    ap.add_argument("routines", nargs="+", help="PC[:name], hex or decimal short-word address")
    ap.add_argument("--target", action="append", required=True, help="flat SRAM image to search")
    args = ap.parse_args()
    ref = Path(args.reference).read_bytes()
    targets = {Path(t).parent.name + "/" + Path(t).name: Path(t).read_bytes() for t in args.target}
    for name, tname, frac in rows(ref, [parse_routine(r) for r in args.routines], targets):
        print(f"{name} {tname} {frac:.2f}")


if __name__ == "__main__":
    main()
