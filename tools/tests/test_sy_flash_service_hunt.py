"""Unit tests for tools/sy_flash_service_hunt.py (synthetic images and ARM code only)."""

from __future__ import annotations

import struct

import ns4_dsp_raw as raw
import sy_flash_service_hunt as h


def _image(blocks: list[tuple[int, int, int, bytes]]) -> bytes:
    out = bytearray(raw.KERNEL_BYTES)
    for tag, count, address, payload in blocks:
        out += struct.pack("<3I", tag, count, address) + payload
    return bytes(out)


def _words(*vals: int) -> bytes:
    return struct.pack(f"<{len(vals)}I", *vals)


def test_image_defaults_reads_the_data_block_and_zero_fill() -> None:
    img = _image([(raw.DATA, 4, 0xB650F, _words(0x93FF7, 0, 0x941F7, 0x96000))])
    assert h.image_defaults(img) == {
        0xB650F: 0x93FF7,
        0xB6510: 0,
        0xB6511: 0x941F7,
        0xB6512: 0x96000,
    }
    zero = _image([(raw.ZERO, 16, 0xB6500, b"")])
    assert h.image_defaults(zero) == {0xB650F: 0, 0xB6510: 0, 0xB6511: 0, 0xB6512: 0}


def test_image_default_buffer_is_outside_the_pitch_window() -> None:
    img = _image([(raw.DATA, 1, 0xB650F, _words(h.STAGING_BASE))])
    assert not h.in_window(h.image_defaults(img)[0xB650F])
    assert h.in_window(0x9354A) and not h.in_window(0x935A0)


def test_dsp_writers_finds_direct_stores_only() -> None:
    lst = "1C09BF 1120 000b 6510  dm(0xb6510)=m0\n1C0A12 1011 000b 650f  i1=dm(0xb650f)\n"
    assert h.dsp_writers(lst, {0xB6510, 0xB650F}) == [(lst.splitlines()[0].strip(), 0xB6510)]


def _code(*words: int) -> bytes:
    return _words(*words)


def test_const_tracker_sees_mov_add_and_sub_forms() -> None:
    targets = {0xB6000: "a", 0xB650F: "b"}
    # mov r0,#0xB6000 is not encodable; build it as mov r6,#0xB0000 ; add r6,r6,#0x6000
    code = _code(
        0xE3A06A0B,  # mov r6, #0xB000 (imm8 0x0B ror 20)
        0xE2866A06,  # add r6, r6, #0x6000 (imm8 0x06 ror 20)
    )
    # 0xB000 + 0x6000 = 0x11000, not a target: no hit
    assert h.arm_const_hits(code, targets, skip=()) == []
    code = _code(0xE3A06A0B, 0xE2866A0B)  # 0xB000 + 0xB000
    assert h.arm_const_hits(code, {0x16000: "x"}, skip=()) == [(h.LOAD_BASE + 4, 6, 0x16000, "x")]


def test_const_tracker_literal_load_then_sub_one() -> None:
    # ldr r0, [pc, #0] -> literal two words ahead; sub r0, r0, #1
    code = _code(0xE59F0000, 0xE2400001, 0xB6510)
    hits = h.arm_const_hits(code, {0xB650F: "crc_buffer", 0xB6510: "crc_count"}, skip=())
    assert hits == [
        (h.LOAD_BASE, 0, 0xB6510, "crc_count"),
        (h.LOAD_BASE + 4, 0, 0xB650F, "crc_buffer"),
    ]


def test_const_tracker_movw_movt_and_branch_reset() -> None:
    code = _code(0xE3000510 | (0xB << 16), 0xE3400009)  # movw r0,#0xB510 ; movt r0,#9
    hits = h.arm_const_hits(code, {0x9B510: "x"}, skip=())
    assert hits == [(h.LOAD_BASE + 4, 0, 0x9B510, "x")]
    # a branch clears the tracker: the sub after it must not hit
    code = _code(0xE59F0004, 0xEA000000, 0xE2400001, 0, 0xB6510)
    assert h.arm_const_hits(code, {0xB650F: "x"}, skip=()) == []


def test_skip_span_is_ignored() -> None:
    code = _code(0xE3A00001)
    span = ((h.LOAD_BASE, h.LOAD_BASE + 4),)
    assert h.arm_const_hits(code, {1: "one"}, skip=span) == []
    assert len(h.arm_const_hits(code, {1: "one"}, skip=())) == 1
