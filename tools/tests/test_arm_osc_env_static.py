"""Tests for `tools/arm_osc_env_static.py` (pure helpers, no firmware)."""

from __future__ import annotations

import struct
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import arm_osc_env_static as st  # noqa: E402


def test_patch_dst_sets_and_clears_only_the_one_bit() -> None:
    blob = bytes([0xA5] * 700)
    on = st.patch_dst(blob, 1)
    off = st.patch_dst(blob, 0)
    assert st.dst_of(on) == 1 and st.dst_of(off) == 0
    diff = [i for i in range(len(blob)) if on[i] != off[i]]
    assert diff == [st.DST_BIT // 8]
    assert on[st.DST_BIT // 8] ^ off[st.DST_BIT // 8] == 1 << (7 - st.DST_BIT % 8)
    assert st.patch_dst(on, 1) == on, "idempotent"


def test_patch_dst_rejects_other_values() -> None:
    with pytest.raises(ValueError, match="not 0 or 1"):
        st.patch_dst(bytes(700), 2)


def test_dsp_address_is_the_common_block_word_12() -> None:
    assert st.dsp_address(0) == 0xE02F4
    assert st.dsp_address(2) == 0xE0334


def test_word_steps_reads_a_float32_word_in_steps() -> None:
    word = struct.unpack("<I", struct.pack("<f", -0.575))[0]
    assert st.word_steps(word) == pytest.approx(-34.5, abs=1e-4)
    assert st.word_steps(0) == 0.0


def test_q16_word_is_twos_complement() -> None:
    assert st.q16_word(24) == 24 << 16
    assert st.q16_word(-48) == (-48 << 16) & 0xFFFFFFFF
    assert st.q16_word(-1) == 0xFFFF0000


def test_rows_csv_has_a_header_and_one_line_per_row() -> None:
    text = st.rows_csv([("Time Warp Saw", 1, -48, "0xbf133336", "-34.5000")])
    assert text.splitlines() == [
        "program,dst,x,dsp_word,dsp_steps",
        "Time Warp Saw,1,-48,0xbf133336,-34.5000",
    ]
