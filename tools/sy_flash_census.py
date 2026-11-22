#!/usr/bin/env python3
"""Census for the SY pitch entry hunt, round 5 (docs/research/sy-flash-bus-hunt.md, D176).

Answers four narrow questions from local files only (no firmware word is printed):

1. Does the SY image fill the per-sample offset table `DM(0xB2000 .. 0xB5FFF)` or the pitch window
   `DM(0x93500 .. 0x9359F)` with data, or only with zero fill? (`zero_fill_covers`)
2. Which address spaces do the ARM to SY / SMP port messages of the boot log address, and does any
   hit one of the two windows? (`census_log`) A message `w1` is `opcode << 28 | address`; opcode 0
   stores `w2` at `address`, and the SY audio ISR (`0x1C0635` loop) treats a 28 bit address that
   lies in an AMI bank as an external memory store.
3. Which ARM literals (32 bit words and `movw` / `movt` pairs) name the flash service mailbox words
   of the SY (`literal_hits`)?

    python tools/sy_flash_census.py target/fw_unpacked/bank0_CODE.bin \
        target/DSP/dsp_image_3_SY1SY2.bin target/bulk/boot_log.jsonl

Provenance tags follow `.agents/rules/provenance-in-code.md`.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
from collections import Counter
from collections.abc import Iterable
from pathlib import Path

import ns4_dsp_raw as raw

LOAD_BASE = 0xC000_0000
# VERIFIED: pitch window of the SY audio ISR step 3 (`i5 = 0x9354A` at 0x1C0679, table
# 0x9354A..0x9358D), docs/research/sy-pitch-entry-writer.md s2.
PITCH_WINDOW = (0x93500, 0x935A0)
# VERIFIED: the only SY reader is `i0 = 0xB2000` at 0x1C0C46 (`r2 = DM(0xB2000 + m3)`); the image
# zero fills 0xB2000 .. 0xB6002 (one tag 1 block, 16,387 words, `zero_fill_covers` below).
OFFSET_TABLE = (0xB2000, 0xB6000)
# VERIFIED (listing `sy2.lst` 0x1C08DB, 0x1C09A9, 0x1C0A67 and the ARM literal pool at 0xC024C370,
# 0xC0444E4C): the words of the ARM claimed flash service. `DM(0x93DF6) != 0` sends the audio ISR
# to its service loop `0x1C0857`; `DM(0xB6000)` is the flash address and `DM(0xB6001)` the read
# count (read service `0x1C08DB`); `DM(0xB6510..0xB6512)` drive the checksum service `0x1C09A9`
# whose buffer pointer is `DM(0xB650F)`.
SERVICE_WORDS = {
    0x93DF6: "claim",
    0x93DF7: "busy",
    0xB6000: "read_addr",
    0xB6001: "read_count",
    0xB650F: "crc_buffer",
    0xB6510: "crc_count",
    0xB6511: "crc_table",
    0xB6512: "crc_extra",
}
# VERIFIED (DSP image stream): the SY image block stream sits at 0xC044667C .. 0xC05263A8 and the
# ARM initialisation script at 0xC0503600 .. 0xC05051E8 contains the same words; both are skipped
# when counting ARM literals, they are DSP side data, not ARM code.
SKIP_SPANS = ((0xC044_667C, 0xC052_63A8),)

BANK_BASE = {1: 0x0400_0000, 2: 0x0800_0000, 3: 0x0C00_0000}


def message_space(w1: int) -> str:
    """Address space of a port message: `dm`, `bank1`..`bank3` (AMI) or `other`."""
    a = w1 & 0x0FFF_FFFF
    if a < 0x10_0000:
        return "dm"
    for bank, base in BANK_BASE.items():
        if base <= a < base + 0x0400_0000:
            return f"bank{bank}"
    return "other"


def in_window(addr: int, window: tuple[int, int]) -> bool:
    return window[0] <= addr < window[1]


def census_log(lines: Iterable[str], chip: str) -> dict[str, object]:
    """Messages of `chip` in a boot log (`jsonl`, keys `chip`, `w1`, `w2`, `pc`)."""
    spaces: Counter[str] = Counter()
    opcodes: Counter[int] = Counter()
    pitch = offset = total = 0
    for line in lines:
        d = json.loads(line)
        if d["chip"] != chip:
            continue
        w1 = int(d["w1"], 16)
        total += 1
        spaces[message_space(w1)] += 1
        opcodes[w1 >> 28] += 1
        a = w1 & 0x0FFF_FFFF
        pitch += in_window(a, PITCH_WINDOW)
        offset += in_window(a, OFFSET_TABLE)
    return {
        "total": total,
        "spaces": dict(spaces),
        "opcodes": dict(opcodes),
        "pitch_hits": pitch,
        "offset_hits": offset,
    }


def zero_fill_covers(img: bytes, window: tuple[int, int]) -> tuple[bool, bool]:
    """`(covered by zero fill blocks, touched by a data or code block)` for a DM window of an
    ADI raw DSP image (`ns4_dsp_raw` block stream; addresses below 0x100000 are DM words)."""
    covered = bytearray(window[1] - window[0])
    touched = False
    for b in raw.parse_blocks(img)[0]:
        if b.address >= 0x10_0000:
            continue
        lo, hi = max(b.address, window[0]), min(b.address + b.count, window[1])
        if lo >= hi:
            continue
        if b.tag == raw.ZERO:
            covered[lo - window[0] : hi - window[0]] = b"\x01" * (hi - lo)
        elif b.tag == raw.DATA:
            touched = True
    return all(covered), touched


def _imm16(word: int) -> int:
    return ((word >> 4) & 0xF000) | (word & 0xFFF)


def literal_hits(code: bytes, values: dict[int, str]) -> list[tuple[int, int, str, str]]:
    """ARM literals naming one of `values`: `(address, value, name, kind)`, `kind` is `lit` for a
    32 bit word or `movw` for a `movw` / `movt` pair within 8 words. Words inside `SKIP_SPANS`
    (DSP images) are ignored."""
    words = struct.unpack_from(f"<{len(code) // 4}I", code)
    hits: list[tuple[int, int, str, str]] = []

    def skipped(off: int) -> bool:
        return any(lo - LOAD_BASE <= off < hi - LOAD_BASE for lo, hi in SKIP_SPANS)

    for i, v in enumerate(words):
        off = 4 * i
        if skipped(off):
            continue
        if v in values:
            hits.append((LOAD_BASE + off, v, values[v], "lit"))
        if (v & 0xFFF0_0000) == 0xE300_0000:  # movw
            rd = (v >> 12) & 0xF
            for u in words[i + 1 : i + 8]:
                if (u & 0xFFF0_0000) == 0xE340_0000 and ((u >> 12) & 0xF) == rd:
                    val = (_imm16(u) << 16) | _imm16(v)
                    if val in values:
                        hits.append((LOAD_BASE + off, val, values[val], "movw"))
                    break
    return hits


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("code", type=Path, help="bank0_CODE.bin")
    ap.add_argument("sy_image", type=Path, help="dsp_image_3_SY1SY2.bin")
    ap.add_argument("boot_log", type=Path, help="boot_log.jsonl of arm_bulk_tables")
    a = ap.parse_args(argv)
    img = a.sy_image.read_bytes()
    for name, win in (("offset table", OFFSET_TABLE), ("pitch window", PITCH_WINDOW)):
        cov, touched = zero_fill_covers(img, win)
        print(f"{name} 0x{win[0]:X}..0x{win[1]:X}: zero fill covers={cov} data block={touched}")
    lines = a.boot_log.read_text().splitlines()
    for chip in ("SY", "SMP"):
        print(chip, census_log(lines, chip))
    for hit in literal_hits(a.code.read_bytes(), SERVICE_WORDS):
        print(f"literal 0x{hit[0]:08X} 0x{hit[1]:X} {hit[2]} ({hit[3]})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
