"""Unit tests for tools/arm_const_track.py (synthetic ARM words only)."""

from __future__ import annotations

import struct

import arm_const_track as t

BASE = t.LOAD_BASE


def _bl(pc: int, target: int) -> int:
    return 0xEB00_0000 | (((target - (pc + 8)) >> 2) & 0x00FF_FFFF)


def _code(*words: int) -> bytes:
    return struct.pack(f"<{len(words)}I", *words)


def test_ror_and_imm16() -> None:
    assert t.ror(0xFF, 8) == 0xFF00_0000
    assert t.ror(0x1234, 0) == 0x1234
    assert t.imm16(0xE304_1213) == 0x4213


def test_bl_target_decodes_forward_and_backward() -> None:
    assert t.bl_target(_bl(BASE + 8, 0xC006_2E10), BASE + 8) == 0xC006_2E10
    assert t.bl_target(_bl(BASE + 0x100, BASE + 0x20), BASE + 0x100) == BASE + 0x20
    assert t.bl_target(0xE3A0_0004, BASE) is None  # mov, not a bl
    assert t.bl_target(0xFB00_0000, BASE) is None  # blx (cond 0xF)


def test_mov_ldr_literal_and_add_are_tracked() -> None:
    code = _code(
        0xE59F_1004,  # ldr r1, [pc, #4]  (literal at +12)
        0xE3A0_0004,  # mov r0, #4
        0xE281_2010,  # add r2, r1, #0x10
        0x0009_4213,  # literal
    )
    states = list(t.iter_states(code))
    assert states[0][3] == (1, 0x9_4213)
    assert states[1][3] == (0, 4)
    assert states[2][3] == (2, 0x9_4223)
    assert states[2][2] == {0: 4, 1: 0x9_4213, 2: 0x9_4223}


def test_branch_is_reported_before_the_reset() -> None:
    code = _code(0xE3A0_0003, _bl(BASE + 4, 0xC006_2E10), 0xE1A0_1000)
    states = list(t.iter_states(code))
    assert states[1][2] == {0: 3}  # the bl sees r0
    assert states[1][3] is None
    assert states[2][3] == (1, None)  # `mov r1, r0` after the reset: r0 is unknown again


def test_movw_movt_and_unknown_overwrite() -> None:
    code = _code(
        0xE304_1213,  # movw r1, #0x4213
        0xE340_1009,  # movt r1, #9
        0xE591_0000,  # ldr r0, [r1]  (r0 unknown)
        0xE3A0_0001,  # mov r0, #1
        0xE590_0000,  # ldr r0, [r0] -> r0 overwritten with an unknown value
    )
    states = list(t.iter_states(code))
    assert states[1][3] == (1, 0x9_4213)
    assert states[4][3] == (0, None)
    assert 0 not in states[4][2]


def test_skip_span_clears_state() -> None:
    code = _code(0xE3A0_0004, 0xE3A0_1005, 0xE3A0_2006)
    states = list(t.iter_states(code, skip=((BASE + 4, BASE + 8),)))
    assert [s[0] for s in states] == [BASE, BASE + 8]
    assert states[1][2] == {2: 6}
