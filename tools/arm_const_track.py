#!/usr/bin/env python3
"""Linear constant tracker over ARM-mode code (shared by the SY pitch entry hunts).

`iter_states` walks a code image word by word and keeps the registers whose value is a known
constant: literal loads (`ldr rd, [pc, #x]`), `mov`, `mvn`, `movw`, `movt`, register copies and
`add` / `sub` / `orr` with an immediate. It resets at any branch, `bx`, `ldm` and unconditional
(`0xF`) space instruction, so values built across basic blocks are not seen (ASSUMED approximation,
the same one `sy_flash_service_hunt.arm_const_hits` used before it was moved here).

Branch words are reported before the reset, so a caller can read the argument registers at a `bl`.
`bl_target` decodes the destination of a `bl` word. Pure Python, no firmware is read here.
"""

from __future__ import annotations

import struct
from collections.abc import Iterator

LOAD_BASE = 0xC000_0000


def ror(v: int, n: int) -> int:
    """Rotate a 32 bit value right by `n` bits."""
    n %= 32
    return ((v >> n) | (v << (32 - n))) & 0xFFFF_FFFF if n else v


def imm16(w: int) -> int:
    """The 16 bit immediate of a `movw` / `movt` word."""
    return ((w >> 4) & 0xF000) | (w & 0xFFF)


def is_branch(w: int) -> bool:
    """`b` / `bl`, `bx`, or `ldm` (any form that may move the pc or reload registers)."""
    bx = (w & 0x0FFF_FFF0) == 0x012F_FF10
    return (w >> 25) & 7 == 5 or bx or (w & 0x0E50_0000) == 0x0810_0000


def bl_target(w: int, pc: int) -> int | None:
    """Destination of a `bl` word at `pc`, or `None` when `w` is not an ARM `bl`."""
    if (w >> 28) == 0xF or (w >> 24) & 0xF != 0xB:
        return None
    off = (w & 0x00FF_FFFF) << 2
    if off & 0x0200_0000:
        off -= 0x0400_0000
    return (pc + 8 + off) & 0xFFFF_FFFF


def _value(w: int, off: int, code: bytes, regs: dict[int, int]) -> tuple[int, int | None] | None:
    """`(rd, value)` written by `w` (value `None` = overwritten with an unknown), or `None`."""
    kind = (w >> 25) & 7
    rd = (w >> 12) & 0xF
    rn = (w >> 16) & 0xF
    value: int | None = None
    if (w & 0x0F7F_0000) == 0x051F_0000:  # ldr rd, [pc, #+-imm12]
        lit = off + 8 + (w & 0xFFF if w & (1 << 23) else -(w & 0xFFF))
        if 0 <= lit < len(code) - 3:
            value = struct.unpack_from("<I", code, lit)[0]
    elif (w & 0x0FF0_0000) == 0x0300_0000:  # movw
        value = imm16(w)
    elif (w & 0x0FF0_0000) == 0x0340_0000:  # movt
        if rd in regs:
            value = (imm16(w) << 16) | (regs[rd] & 0xFFFF)
    elif kind == 1:  # data processing, immediate operand
        op = (w >> 21) & 0xF
        imm = ror(w & 0xFF, 2 * ((w >> 8) & 0xF))
        if op == 0xD:
            value = imm
        elif op == 0xF:
            value = ~imm & 0xFFFF_FFFF
        elif rn in regs and op in (0x4, 0x2, 0xC):
            base = regs[rn]
            value = {0x4: base + imm, 0x2: base - imm, 0xC: base | imm}[op] & 0xFFFF_FFFF
    elif kind == 0 and ((w >> 21) & 0xF) == 0xD and (w & 0xFF0) == 0 and (w & 0xF) in regs:
        value = regs[w & 0xF]  # mov rd, rm
    elif (w & 0x0C10_0000) == 0x0410_0000 or (kind == 0 and (w & 0x0010_0000) == 0):
        pass  # other load or register data processing: rd is overwritten (value stays None)
    else:
        return None
    return rd, value


def iter_states(
    code: bytes, skip: tuple[tuple[int, int], ...] = ()
) -> Iterator[tuple[int, int, dict[int, int], tuple[int, int | None] | None]]:
    """Yield `(pc, word, regs, written)` for every word of `code` (loaded at `LOAD_BASE`).

    `regs` is a copy of the constant map after the word (before the reset for a branch word),
    `written` is `(rd, value)` when the word wrote a register (`value` `None` when unknown).
    `skip` spans (absolute pc ranges) clear the state and are not reported."""
    words = struct.unpack_from(f"<{len(code) // 4}I", code)
    regs: dict[int, int] = {}
    for i, w in enumerate(words):
        off = 4 * i
        pc = LOAD_BASE + off
        if any(lo <= pc < hi for lo, hi in skip) or (w >> 28) == 0xF:
            regs.clear()
            continue
        if is_branch(w):
            yield pc, w, dict(regs), None
            regs.clear()
            continue
        written = _value(w, off, code, regs)
        if written is None:
            yield pc, w, dict(regs), None
            continue
        rd, value = written
        if value is None:
            regs.pop(rd, None)
        else:
            regs[rd] = value
        yield pc, w, dict(regs), written
