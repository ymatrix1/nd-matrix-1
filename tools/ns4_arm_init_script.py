#!/usr/bin/env python3
"""Decode the ARM DSP initialisation script (`bank0_CODE@0xC0503600 ..`) into SY DM words.

Record layout (docs/research/sy-pitch-entry-writer.md s7, read from the data, no ARM function
traced): type 4 is `[4][n][address][n values]` (a block write into the SY1SY2 DM), type 1 is
`[1][value][address]` (a single word, addresses below 0x100000 are DM, others other spaces).
The SY part of the script is the span `0xC0503740 .. 0xC05038B8`: it sets the scan pointer
`DM(0x95C69)`, the width indexed unpacker pointer table `DM(0x95C7B..)` and some constants.

    python tools/ns4_arm_init_script.py target/fw_unpacked/bank0_CODE.bin [--out poke.scn]

The output holds firmware words: keep it under `target/`, never commit it.
"""

from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path

LOAD_BASE = 0xC000_0000
# VERIFIED: span of the SY records (type 4 records at 0xC0503740 .. 0xC05038B0, one further type 4
# record at 0xC050388C); the next type 4 record at 0xC051BDE8 belongs to the SMP image script.
SY_SPAN = (0xC050_3740, 0xC050_38B8)
DM_LIMIT = 0xA0000
# VERIFIED: the external-port TCB table is one type 4 record, `[4][1224][0xB6003]` + 1224 words,
# at `0xC05038BC` (68 slots of 3 TCBs, 18 words per slot, first slot `II` words at 0xB6008).
TCB_SPAN = (0xC050_38BC, 0xC050_38BC + (3 + 1224) * 4)
TCB_BASE = 0xB6003
SLOTS = 68
SLOT_WORDS = 18
# VERIFIED: the ISR sets the B TCB count `r4 = 8` (sy2.lst 0x1C0AF6); the static table holds 0.
B_COUNT = 8
# Pitch table window of the SY audio ISR (docs/research/sy-pitch-entry-writer.md).
PITCH_WINDOW = (0x93500, 0x935A0)


def read_words(code: bytes, lo: int, hi: int) -> list[int]:
    """Little endian words of `code` between the load addresses `lo` and `hi`."""
    a, b = lo - LOAD_BASE, hi - LOAD_BASE
    return list(struct.unpack_from(f"<{(b - a) // 4}I", code, a))


def parse_records(words: list[int]) -> list[tuple[int, list[int]]]:
    """Type 4 block records `(address, values)`; stray words (other record types) are skipped."""
    out: list[tuple[int, list[int]]] = []
    i = 0
    while i + 3 <= len(words):
        n, addr = words[i + 1], words[i + 2]
        if (
            words[i] == 4
            and 0 < n <= 0x400
            and 0x90000 <= addr < DM_LIMIT
            and i + 3 + n <= len(words)
        ):
            out.append((addr, words[i + 3 : i + 3 + n]))
            i += 3 + n
        else:
            i += 1
    return out


def sy_records(code: bytes) -> list[tuple[int, list[int]]]:
    return parse_records(read_words(code, *SY_SPAN))


def tcb_records(code: bytes) -> list[tuple[int, list[int]]]:
    """The external-port TCB table record (`parse_records` accepts it: 1224 words > 0x400 is
    checked here instead)."""
    if len(code) < TCB_SPAN[1] - LOAD_BASE:
        raise ValueError("code image shorter than TCB_SPAN")
    w = read_words(code, *TCB_SPAN)
    if w[0] != 4 or w[2] != TCB_BASE or w[1] != len(w) - 3:
        raise ValueError("TCB record not found at TCB_SPAN")
    return [(w[2], w[3:])]


def tcb_nodes(vals: list[int]) -> list[dict[str, int]]:
    """Decode the 3 TCBs of each slot. A TCB is 6 words in the order CP, EM, EI, C, IM, II and its
    pointer value is the address of II (HRM listing 4-7); slot `k` holds A at `0xB6008 + 18 k`,
    X at `+6` and B at `+12`."""
    out = []
    for k in range(SLOTS):
        for name, off in (("A", 0), ("X", 6), ("B", 12)):
            p = TCB_BASE + 5 + SLOT_WORDS * k + off
            i = p - TCB_BASE
            cp, em, ei, c, im, ii = (vals[i - 5 + j] for j in range(6))
            out.append(
                {
                    "slot": k,
                    "kind": ord(name),
                    "p": p,
                    "cp": cp,
                    "em": em,
                    "ei": ei,
                    "c": c,
                    "im": im,
                    "ii": ii,
                }
            )
    return out


def tcb_summary(nodes: list[dict[str, int]]) -> list[str]:
    """One line per TCB kind: count, internal destination range, how many touch the pitch window.

    The B TCBs hold count 0 in the static table (the ISR writes 8), so they are checked with
    `B_COUNT` words; A reads DM (its internal address is a source, listed for completeness)."""
    lines = []
    for kind in "AXB":
        ns = [n for n in nodes if n["kind"] == ord(kind)]
        eff = [n["c"] or (B_COUNT if kind == "B" else 0) for n in ns]
        lo = min(n["ii"] for n in ns)
        hi = max(n["ii"] + n["im"] * (c - 1) for n, c in zip(ns, eff, strict=True))
        hits = sum(
            1
            for n, c in zip(ns, eff, strict=True)
            for k in range(c)
            if PITCH_WINDOW[0] <= n["ii"] + n["im"] * k < PITCH_WINDOW[1]
        )
        lines.append(
            f"{kind}: {len(ns)} TCBs, internal 0x{lo:X}..0x{hi:X}, "
            f"counts {sorted(set(eff))}, pitch window hits {hits}"
        )
    return lines


def poke_lines(records: list[tuple[int, list[int]]]) -> list[str]:
    """`poke ADDR VALUE` scenario lines, one per word."""
    return [f"poke 0x{a + k:X} 0x{v:X}" for a, vals in records for k, v in enumerate(vals)]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("code", type=Path, help="bank0_CODE.bin")
    ap.add_argument("--out", type=Path, help="write the poke lines here (default stdout)")
    ap.add_argument("--tcb", action="store_true", help="emit the external-port TCB table instead")
    ap.add_argument("--tcb-summary", action="store_true", help="print the TCB table summary only")
    args = ap.parse_args(argv)
    code = args.code.read_bytes()
    if args.tcb_summary:
        lines = tcb_summary(tcb_nodes(tcb_records(code)[0][1]))
        sys.stdout.write("\n".join(lines) + "\n")
        return 0
    lines = poke_lines(tcb_records(code) if args.tcb else sy_records(code))
    text = "\n".join(lines) + "\n"
    if args.out:
        args.out.write_text(text)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
