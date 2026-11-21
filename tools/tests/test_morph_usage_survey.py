"""Tests for `tools/morph_usage_survey.py` (synthetic program payloads, no firmware)."""

from __future__ import annotations

import sys
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import morph_usage_survey as ms  # noqa: E402

PAYLOAD_LEN = 820


def set_bits(buf: bytearray, pos: int, width: int, value: int) -> None:
    """Writes `value` MSB first into `buf` at bit `pos`."""
    for i in range(width):
        bit = (value >> (width - 1 - i)) & 1
        p = pos + i
        shift = 7 - (p & 7)
        buf[p >> 3] = (buf[p >> 3] & ~(1 << shift)) | (bit << shift)


def zero_depth_payload() -> bytearray:
    """A payload whose every morph word reads depth 0 (stored `max_val`)."""
    buf = bytearray(PAYLOAD_LEN)
    org = ms.ORGAN[0] * 8
    for base in ms.ORGAN_LAYER_BASES:
        for i in range(ms.DRAWBAR_COUNT):
            for k in range(3):
                set_bits(buf, org + (base + 40 + 19 * i + 4 + 5 * k), 5, 8)
    set_bits_all(buf, org + ms.LESLIE_SPEED[0] + 2, 3, 3)
    syn = ms.SYNTH[0] * 8
    for base in ms.SYNTH_LAYER_BASES:
        for off, width, max_val in (ms.ARP_RATE, ms.ARP_RANGE):
            set_words(buf, syn + base + off + width, width + 1, max_val)
    for base in ms.SOUND_BASES:
        set_words(buf, syn + base + ms.OSC_ENV_AMOUNT[0] + 7, 8, ms.OSC_ENV_AMOUNT[2])
    for section, offsets in ms.FX_BLOCKS.items():
        start = {"organ": ms.ORGAN, "piano": ms.PIANO, "synth": ms.SYNTH}[section][0] * 8
        for off in offsets:
            set_words(buf, start + off * 8 + ms.FX_DELAY_RATE[0] + 14, 15, ms.FX_DELAY_RATE[2])
    return buf


def set_words(buf: bytearray, pos: int, width: int, value: int) -> None:
    """Three consecutive `width`-bit words holding `value`."""
    for k in range(3):
        set_bits(buf, pos + k * width, width, value)


def set_bits_all(buf: bytearray, pos: int, width: int, value: int) -> None:
    set_words(buf, pos, width, value)


def test_bits_at_reads_msb_first() -> None:
    assert ms.bits_at(bytes([0b1010_0000, 0xFF]), 0, 4) == 0b1010
    assert ms.bits_at(bytes([0b0000_0001, 0b1000_0000]), 7, 2) == 0b11


def test_morph_depths_subtract_the_stored_zero() -> None:
    buf = bytearray(8)
    set_bits(buf, 3, 7, 40)  # base
    set_bits(buf, 10, 8, 127 + 22)  # wheel +22
    set_bits(buf, 18, 8, 127)
    set_bits(buf, 26, 8, 127 - 5)  # pedal -5
    assert ms.morph_depths(bytes(buf), 3, 7, 127) == (22, 0, -5)
    assert ms.used((22, 0, -5)) == ["wheel", "pedal"]


def test_a_program_without_depths_uses_nothing() -> None:
    assert ms.survey_program(bytes(zero_depth_payload())) == {}


def test_each_target_is_found_at_its_offset() -> None:
    buf = zero_depth_payload()
    org = ms.ORGAN[0] * 8
    set_bits(buf, org + ms.ORGAN_LAYER_BASES[1] + 40 + 19 * 3 + 4, 5, 8 + 2)
    syn = ms.SYNTH[0] * 8
    set_bits(buf, syn + ms.SYNTH_LAYER_BASES[2] + ms.ARP_RANGE[0] + 7 + 8, 8, 127 - 9)
    set_bits(buf, syn + ms.SOUND_BASES[0] + ms.OSC_ENV_AMOUNT[0] + 7 + 16, 8, 120 + 1)
    fx = ms.PIANO[0] * 8 + ms.FX_BLOCKS["piano"][1] * 8 + ms.FX_DELAY_RATE[0] + 14 + 15
    set_bits(buf, fx, 15, 16383 - 4096)
    assert ms.survey_program(bytes(buf)) == {
        "drawbar": ["wheel"],
        "arp_range": ["at"],
        "osc_env_amount": ["pedal"],
        "delay_rate": ["at"],
    }


def test_leslie_speed_word_is_read() -> None:
    buf = zero_depth_payload()
    set_bits(buf, ms.ORGAN[0] * 8 + ms.LESLIE_SPEED[0] + 2, 3, 3 + 1)
    assert ms.survey_program(bytes(buf)) == {"leslie_speed": ["wheel"]}


def test_summary_and_archive(tmp_path: Path) -> None:
    buf = zero_depth_payload()
    set_bits(buf, ms.SYNTH[0] * 8 + ms.SYNTH_LAYER_BASES[0] + ms.ARP_RATE[0] + 7, 8, 127 + 3)
    path = tmp_path / "b.ns4b"
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("Program/Bank A/Test.ns4p", bytes(ms.HEADER) + bytes(buf))
        archive.writestr("Synth/Other.ns4y", b"x")
    rows = ms.survey_archive(path)
    assert rows == {"Program/Bank A/Test.ns4p": {"arp_rate": ["wheel"]}}
    lines = ms.summary(rows)
    assert lines[0] == "1 programs"
    assert any(line.startswith("arp_rate") and " 1 programs" in line for line in lines)
