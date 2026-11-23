#!/usr/bin/env python3
"""Static census of every ARM call site of the path 1 message writers (SY and SMP link).

The pitch entry hunts (docs/research/sy-pitch-entry-writer.md, D196, D219) drove the ARM only
through the paths a harness reaches. This tool asks the opposite question: which call sites of the
shared path 1 writers `0xC0062E10` and `0xC0062E78` `(comm, word1, word2)` exist at all, which
chip does each name, and which of them carry a `word1` that no table or constant explains. The
comm index of a call is the constant in `r0` at the `bl` (`arm_const_track`, resets at branches),
`word1` the constant in `r1` when known.

    python tools/arm_sender_sites.py target/fw_unpacked/bank0_CODE.bin

Only addresses and counts are printed. Provenance tags follow `.agents/rules/provenance-in-code.md`.
"""

from __future__ import annotations

import argparse
import sys
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

from arm_const_track import bl_target, iter_states

# VERIFIED: docs/research/arm-param-messages.md s2 (`arm_dsp_link`, disassembly of both bodies):
# the two copies take `(comm, word1, word2)`, differ only in the register that keeps `word2`.
PATH1_WRITERS = (0xC006_2E10, 0xC006_2E78)
# ASSUMED: comm index to chip, tools/arm_emu.py `COMM_NAMES` (same order as the download index).
COMM_NAMES = ("FX", "PNO", "ORG", "SY", "SMP")
# VERIFIED: pitch window of the SY audio ISR step 3 (`i5 = 0x9354A`), D141 / D176.
PITCH_WINDOW = (0x93500, 0x935A0)


@dataclass(frozen=True)
class Site:
    """One `bl` to a path 1 writer: the call address and the constants known at the call."""

    pc: int
    writer: int
    comm: int | None
    word1: int | None
    word2: int | None

    @property
    def chip(self) -> str:
        if self.comm is None:
            return "unknown"
        return COMM_NAMES[self.comm] if 0 <= self.comm < len(COMM_NAMES) else f"comm{self.comm}"


def find_sites(code: bytes, writers: tuple[int, ...] = PATH1_WRITERS) -> list[Site]:
    """Every `bl` to one of `writers` with the `r0`, `r1`, `r2` constants known at the call."""
    sites = []
    for pc, w, regs, _written in iter_states(code):
        target = bl_target(w, pc)
        if target in writers:
            sites.append(Site(pc, target, regs.get(0), regs.get(1), regs.get(2)))
    return sites


def in_window(word1: int | None) -> bool:
    """True when a constant `word1` addresses the pitch window (DM space, opcode bits ignored)."""
    return word1 is not None and PITCH_WINDOW[0] <= (word1 & 0x0FFF_FFFF) < PITCH_WINDOW[1]


def summarise(sites: list[Site]) -> dict[str, object]:
    """Counts per chip, constant `word1` sites, window hits and the sites with an unknown comm."""
    by_chip = Counter(s.chip for s in sites)
    const_w1 = [s for s in sites if s.word1 is not None]
    return {
        "sites": len(sites),
        "by_chip": dict(sorted(by_chip.items())),
        "const_word1": len(const_w1),
        "window_hits": [s.pc for s in sites if in_window(s.word1)],
        "unknown_comm": [s.pc for s in sites if s.comm is None],
    }


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("code", type=Path, help="bank0_CODE.bin (loaded at 0xC0000000)")
    ap.add_argument("--list", action="store_true", help="print every site")
    a = ap.parse_args(argv)
    sites = find_sites(a.code.read_bytes())
    info = summarise(sites)
    print(f"path 1 writer call sites: {info['sites']}  by chip: {info['by_chip']}")
    print(f"constant word1: {info['const_word1']}  pitch window hits: {info['window_hits']}")
    print(f"unknown comm: {len(info['unknown_comm'])}")  # type: ignore[arg-type]
    if a.list:
        for s in sites:
            w1 = "?" if s.word1 is None else f"0x{s.word1:X}"
            print(f"0x{s.pc:08X} -> 0x{s.writer:X} {s.chip:7} word1={w1}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
