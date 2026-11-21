"""Unit tests for tools/sy_flash_census.py (synthetic images and logs, no firmware needed)."""

from __future__ import annotations

import json
import struct

import ns4_dsp_raw as raw
import sy_flash_census as c


def _image(blocks: list[tuple[int, int, int, bytes]]) -> bytes:
    """Raw DSP image: a kernel stub of KERNEL_BYTES zero bytes, then `(tag, count, address,
    payload)` blocks."""
    out = bytearray(raw.KERNEL_BYTES)
    for tag, count, address, payload in blocks:
        out += struct.pack("<3I", tag, count, address) + payload
    return bytes(out)


def test_message_space_classifies_dm_and_ami_banks() -> None:
    assert c.message_space(0x0009_354A) == "dm"
    assert c.message_space(0x0400_0145) == "bank1"
    assert c.message_space(0x0800_0140) == "bank2"
    assert c.message_space(0x0C00_0010) == "bank3"
    assert c.message_space(0x2000_0000 | 5) == "dm"  # the opcode nibble is not part of the address
    assert c.message_space(0x0F00_0000) == "bank3"
    assert c.message_space(0x0F00_0000 + 0x0200_0000) == "other"


def test_census_log_counts_spaces_opcodes_and_window_hits() -> None:
    def row(chip: str, w1: int) -> str:
        return json.dumps({"chip": chip, "w1": f"0x{w1:08x}", "w2": "0x1", "pc": "0xc0000000"})

    lines = [
        row("SY", 0x921A6),
        row("SY", 0x9354A),
        row("SY", 0xB2005),
        row("SY", 0x0400_0145),
        row("SMP", 0x0800_0280),
        row("SY", 0x0109_3550),  # address 0x1093550: neither DM nor an AMI bank
    ]
    r = c.census_log(lines, "SY")
    assert r["total"] == 5
    assert r["pitch_hits"] == 1
    assert r["offset_hits"] == 1
    assert r["spaces"] == {"dm": 3, "bank1": 1, "other": 1}
    assert c.census_log(lines, "SMP")["spaces"] == {"bank2": 1}
    assert c.census_log(lines, "FX")["total"] == 0


def test_zero_fill_covers_distinguishes_fill_from_data() -> None:
    fill = _image([(raw.ZERO, 0x4003, 0xB2000, b"")])
    assert c.zero_fill_covers(fill, c.OFFSET_TABLE) == (True, False)
    data = _image([(raw.ZERO, 0x4003, 0xB2000, b""), (raw.DATA, 1, 0xB2010, b"\x01\x00\x00\x00")])
    assert c.zero_fill_covers(data, c.OFFSET_TABLE) == (True, True)
    gap = _image([(raw.ZERO, 0x10, 0xB2000, b"")])
    assert c.zero_fill_covers(gap, c.OFFSET_TABLE) == (False, False)


def test_zero_fill_covers_reports_partial_fill_and_data_in_the_window() -> None:
    img = _image([(raw.DATA, 1, 0x93580, b"\x00\x80\x00\x80"), (raw.ZERO, 0x60, 0x93500, b"")])
    assert c.zero_fill_covers(img, c.PITCH_WINDOW) == (False, True)
    ok = _image([(raw.ZERO, 0xA0, 0x93500, b"")])
    assert c.zero_fill_covers(ok, c.PITCH_WINDOW) == (True, False)


def test_literal_hits_finds_words_and_movw_movt_pairs_and_skips_images() -> None:
    size = c.SKIP_SPANS[0][1] - c.LOAD_BASE
    code = bytearray(size)
    struct.pack_into("<I", code, 0x100, 0xB6001)  # plain literal
    movw = 0xE300_0000 | (0x6510 >> 12 << 16) | (1 << 12) | (0x6510 & 0xFFF)
    movt = 0xE340_0000 | (0xB >> 12 << 16) | (1 << 12) | (0xB & 0xFFF)
    struct.pack_into("<2I", code, 0x200, movw, movt)
    struct.pack_into("<I", code, c.SKIP_SPANS[0][0] - c.LOAD_BASE + 8, 0x93DF6)  # inside an image
    hits = c.literal_hits(bytes(code), c.SERVICE_WORDS)
    assert (c.LOAD_BASE + 0x100, 0xB6001, "read_count", "lit") in hits
    assert (c.LOAD_BASE + 0x200, 0xB6510, "crc_count", "movw") in hits
    assert all(h[1] != 0x93DF6 for h in hits)


def test_main_prints_a_report(tmp_path, capsys) -> None:  # noqa: ANN001
    code = tmp_path / "code.bin"
    code.write_bytes(bytes(c.SKIP_SPANS[0][1] - c.LOAD_BASE))
    img = tmp_path / "sy.bin"
    img.write_bytes(_image([(raw.ZERO, 0x4003, 0xB2000, b""), (raw.ZERO, 0xA0, 0x93500, b"")]))
    log = tmp_path / "log.jsonl"
    log.write_text(json.dumps({"chip": "SY", "w1": "0x000921a6", "w2": "0x0", "pc": "0x0"}) + "\n")
    assert c.main([str(code), str(img), str(log)]) == 0
    out = capsys.readouterr().out
    assert "offset table" in out and "covers=True" in out
    assert "'total': 1" in out
