#!/usr/bin/env python3
"""Sixth hunt for the SY pitch entry `e` writer: the ARM claimed flash service (D182).

D176 left one path open: the checksum service `0x1C09A9` DMAs flash words to the DM address held in
`DM(0xB650F)`, so an ARM that sets that word to the pitch window would fill `e`. This tool checks
the three things that decide it, from local files only (no firmware word, sample or audio value is
printed, only addresses and counts):

1. `image_defaults`: the DM words the SY image itself seeds for the service mailboxes. The 260 word
   data block at `0xB650F` starts with `DM(0xB650F)` (buffer pointer), `0xB6510` (count),
   `0xB6511` (table pointer), `0xB6512` (list pointer) and is followed by the 256 entry CRC-32
   table, so the buffer pointer has a firmware default.
2. `dsp_writers`: DSP listing lines that store to one of those words (the service only reads them).
3. `arm_const_hits`: a linear constant tracker over the ARM code (literal loads, `mov` / `mvn` /
   `movw` / `movt`, `add` / `sub` / `orr` with an immediate) that reports every place where a
   register holds one of the service words, including values built from a descriptor value plus an
   offset, which the literal scan of `sy_flash_census.py` cannot see.

    python tools/sy_flash_service_hunt.py target/fw_unpacked/bank0_CODE.bin \
        target/DSP/dsp_image_3_SY1SY2.bin target/oracle/sy2.lst

Provenance tags follow `.agents/rules/provenance-in-code.md`.
"""

from __future__ import annotations

import argparse
import re
import struct
import sys
from pathlib import Path

import ns4_dsp_raw as raw
from arm_const_track import iter_states
from sy_flash_census import LOAD_BASE, SERVICE_WORDS, SKIP_SPANS  # noqa: F401 (re-exported)

# VERIFIED: pitch window of the SY audio ISR step 3 (`i5 = 0x9354A`), D141 / D176.
PITCH_WINDOW = (0x93500, 0x935A0)
# VERIFIED (listing `0x1C0925`, `0x1C0A12`, `0x1C0AB6`): the read service stages flash data at
# `0x93FF7`; the checksum service reads its destination from `DM(0xB650F)`.
STAGING_BASE = 0x93FF7
# VERIFIED (`image_defaults` on `dsp_image_3_SY1SY2.bin`, block at header offset 29300): the image
# default of `DM(0xB650F)` is the staging base, not the pitch window.
CRC_BUFFER_WORD = 0xB650F


def image_words(img: bytes, address: int, count: int) -> list[int]:
    """DM words `address .. address + count` as the SY image initialises them (zero fill gives 0,
    an address no block covers also gives 0, the memory reset value)."""
    out = [0] * count
    for b in raw.parse_blocks(img)[0]:
        if b.address >= 0x10_0000 or b.tag != raw.DATA:
            continue
        lo, hi = max(b.address, address), min(b.address + b.count, address + count)
        for a in range(lo, hi):
            off = b.offset + raw.HEADER + 4 * (a - b.address)
            out[a - address] = struct.unpack_from("<I", img, off)[0]
    return out


def image_defaults(img: bytes) -> dict[int, int]:
    """Image values of the checksum mailbox words `0xB650F .. 0xB6512`."""
    words = image_words(img, CRC_BUFFER_WORD, 4)
    return {CRC_BUFFER_WORD + i: w for i, w in enumerate(words)}


def in_window(value: int) -> bool:
    return PITCH_WINDOW[0] <= value < PITCH_WINDOW[1]


_STORE = re.compile(r"dm\(0x([0-9a-f]+)\)\s*=", re.IGNORECASE)


def dsp_writers(listing: str, addresses: set[int]) -> list[tuple[str, int]]:
    """Listing lines that store to `DM(address)` for an address in `addresses`: `(line, address)`.
    Only direct `dm(0x...) =` forms (an indexed store is not visible to a text scan)."""
    hits = []
    for line in listing.splitlines():
        m = _STORE.search(line)
        if m and int(m.group(1), 16) in addresses:
            hits.append((line.strip(), int(m.group(1), 16)))
    return hits


def arm_const_hits(
    code: bytes, targets: dict[int, str], skip: tuple[tuple[int, int], ...] = SKIP_SPANS
) -> list[tuple[int, int, int, str]]:
    """Linear constant tracker over ARM code: `(address, register, value, name)` for every
    instruction after which a register holds one of `targets` (tracker: `arm_const_track`, which
    resets at any branch; ASSUMED approximation: values built across basic blocks are not seen)."""
    hits: list[tuple[int, int, int, str]] = []
    for pc, _w, _regs, written in iter_states(code, skip):
        if written is not None and written[1] in targets:
            hits.append((pc, written[0], written[1], targets[written[1]]))
    return hits


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("code", type=Path, help="bank0_CODE.bin")
    ap.add_argument("sy_image", type=Path, help="dsp_image_3_SY1SY2.bin")
    ap.add_argument("listing", type=Path, help="sy2.lst (tools/adi_dis.py over the SY image)")
    a = ap.parse_args(argv)
    for addr, val in image_defaults(a.sy_image.read_bytes()).items():
        print(f"image DM(0x{addr:X}) = 0x{val:X}  in pitch window: {in_window(val)}")
    stores = dsp_writers(a.listing.read_text(), set(SERVICE_WORDS))
    print(f"DSP direct stores to service words: {len(stores)}")
    for line, addr in stores:
        print(f"  0x{addr:X}: {line}")
    hits = arm_const_hits(a.code.read_bytes(), SERVICE_WORDS)
    print(f"ARM constant tracker hits: {len(hits)}")
    for pc, rd, val, name in hits:
        print(f"  0x{pc:08X} r{rd} = 0x{val:X} ({name})")
    window = {addr: "pitch window" for addr in range(*PITCH_WINDOW)}
    whits = arm_const_hits(a.code.read_bytes(), window)
    print(f"ARM constants naming a pitch window address: {len(whits)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
