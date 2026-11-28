#!/usr/bin/env python3
"""Map ARM addresses of one OS image to the equivalent address in another OS version.

A new OS is a recompile: code moves by a few bytes per function, so every hard-coded ARM0
address changes. This tool finds the equivalent address by matching instruction windows with the
position-dependent bits masked (the 24-bit offset of `B`/`BL`), and, for data cells, by following
the `ldr rX, [pc, #imm]` that loads the address from a literal pool.

    python tools/fw_addr_map.py OLD_bank0_CODE.bin NEW_bank0_CODE.bin 0xC006CA48 0xC055B150 ...

Prints `old -> new (kind, support)` per address. Code addresses are matched on a growing window
(8 to 256 words) until the match is unique; data addresses vote over every literal load of the
old value. `unmapped` means no unique match: handle that address by hand.
Derived values only: nothing of either image is written out.
"""

from __future__ import annotations

import argparse
import struct
import sys
from collections import Counter
from pathlib import Path

LOAD_BASE = 0xC000_0000
WINDOWS = (8, 16, 32, 64, 128, 256)


def words(code: bytes) -> tuple[int, ...]:
    """Little-endian 32-bit words of `code` (a trailing partial word is ignored)."""
    return struct.unpack_from(f"<{len(code) // 4}I", code)


# Address-like constants (literal pool words) move with the image; they match as wildcards.
WILDCARDS = ((0xC000_0000, 0xC060_0000), (0x8000_0000, 0x8010_0000))
ANY_ADDRESS = 1 << 32  # outside the 32-bit word range: no real constant equals it


def mask(word: int) -> int:
    """`word` with the position dependent parts cleared.

    The 24-bit offset of `B`/`BL` (cond, bits 27-25 = 101) and any word that looks like an image
    or SRAM address (literal pool entries) are replaced, so a moved function still matches.
    """
    if (word >> 25) & 7 == 0b101:
        return word & 0xFF00_0000
    if any(lo <= word < hi for lo, hi in WILDCARDS):
        return ANY_ADDRESS
    return word


def masked(ws: tuple[int, ...]) -> list[int]:
    return [mask(w) for w in ws]


class Image:
    """One image: raw words plus a lazily built index of masked word -> positions."""

    def __init__(self, code: bytes) -> None:
        self.words = words(code)
        self.masked = masked(self.words)
        self.index: dict[int, list[int]] = {}
        for i, w in enumerate(self.masked):
            self.index.setdefault(w, []).append(i)

    def find(self, pattern: list[int]) -> list[int]:
        """Word positions where the masked `pattern` occurs."""
        first = self.index.get(pattern[0], [])
        hits = []
        for p in first:
            if self.masked[p : p + len(pattern)] == pattern:
                hits.append(p)
        return hits


def local_delta(old: Image, new: Image, pos: int) -> int | None:
    """Word shift (new - old) of the nearest uniquely matching 16-word window around `pos`."""
    for step in range(0, 4096, 32):
        for p in (pos + step, pos - step):
            if 0 <= p and p + 16 <= len(old.masked):
                hits = new.find(old.masked[p : p + 16])
                if len(hits) == 1:
                    return hits[0] - p
    return None


def map_code(old: Image, new: Image, addr: int, size: int = 256) -> tuple[int, int] | None:
    """New address of the instruction at `addr`, with the window size that made it unique.

    Support 0 means the code is a repeated stub (several equal matches): the one within 0x200
    bytes of where the neighbouring code moved to is taken when it is the only one there.
    """
    pos = (addr - LOAD_BASE) // 4
    hits: list[int] = []
    for n in WINDOWS:
        if n > size or pos + n > len(old.masked):
            break
        found = new.find(old.masked[pos : pos + n])
        if len(found) == 1:
            return LOAD_BASE + 4 * found[0], n
        if not found:
            break
        hits = found
    delta = local_delta(old, new, pos) if len(hits) > 1 else None
    if delta is not None:
        near = [h for h in hits if abs(h - (pos + delta)) <= 0x80]
        if len(near) == 1:
            return LOAD_BASE + 4 * near[0], 0
    return None


def literal_loads(image: Image, value: int) -> list[int]:
    """Word positions of `ldr rX, [pc, #+imm]` instructions whose literal equals `value`."""
    pool = {i for i, w in enumerate(image.words) if w == value}
    out = []
    for i, w in enumerate(image.words):
        # LDR (immediate, offset addressing, Rn = pc, add): cond 0101 U=1 0 0 1 1111
        if (w >> 16) & 0xFFF == 0x59F:
            lit = i + 2 + (w & 0xFFF) // 4
            if lit in pool:
                out.append(i)
    return out


def map_data(old: Image, new: Image, addr: int) -> tuple[int, int] | None:
    """New value of the address constant `addr`, by majority over its literal loads."""
    votes: Counter[int] = Counter()
    for pos in literal_loads(old, addr):
        hit = map_code(old, new, LOAD_BASE + 4 * pos)
        if hit is None:
            continue
        npos = (hit[0] - LOAD_BASE) // 4
        w = new.words[npos]
        if (w >> 16) & 0xFFF == 0x59F:
            lit = npos + 2 + (w & 0xFFF) // 4
            if lit < len(new.words):
                votes[new.words[lit]] += 1
    if not votes:
        return None
    (value, n), *rest = votes.most_common(2)
    if rest and rest[0][1] == n:
        return None
    return value, n


def map_address(old: Image, new: Image, addr: int) -> tuple[str, int, int] | None:
    """`(kind, new_address, support)`; code in the old image first, else a data cell."""
    end = LOAD_BASE + 4 * len(old.words)
    if LOAD_BASE <= addr < end:
        code = map_code(old, new, addr)
        if code:
            return "code", code[0], code[1]
    data = map_data(old, new, addr)
    if data:
        return "data", data[0], data[1]
    return None


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("old", type=Path)
    ap.add_argument("new", type=Path)
    ap.add_argument("addresses", nargs="+", help="addresses in the old image (hex)")
    args = ap.parse_args(argv)
    old, new = Image(args.old.read_bytes()), Image(args.new.read_bytes())
    for text in args.addresses:
        a = int(text, 16)
        r = map_address(old, new, a)
        if r is None:
            print(f"0x{a:08X} -> unmapped")
        else:
            print(f"0x{a:08X} -> 0x{r[1]:08X} ({r[0]}, support {r[2]})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
