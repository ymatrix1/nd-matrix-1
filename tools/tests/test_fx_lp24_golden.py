"""Tests for the LP24 / HP24 Rust generators (no firmware needed)."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import fx_lp24_golden as g  # noqa: E402

LOG = "8 Edt_Effects_kAmpMid -5 {'0xc1a0b': '0x3e380bf0', '0xc1a0c': '0x3f000000'}\nnoise\n"


def test_parse_log_reads_words_and_skips_noise() -> None:
    rows = g.parse_log(LOG)
    assert rows == [("Edt_Effects_kAmpMid", -5, {0xC1A0B: 0x3E380BF0, 0xC1A0C: 0x3F000000})]


def test_csv_lists_only_the_words_that_matter() -> None:
    text = g.to_csv(g.parse_log(LOG))
    assert text.splitlines()[0] == "id,value,dm_word,word"
    assert "Edt_Effects_kAmpMid,-5,0xc1a0b,0x3e380bf0" in text


def test_committed_tables_are_complete() -> None:
    freq, res, gain = g.read_tables()
    assert (len(freq), len(res), len(gain)) == (128, 121, 121)
    assert freq[0] == 0x38824000 and freq[127] == 0x3F7FFFFE


def test_generated_rust_has_the_expected_items() -> None:
    assert "pub(super) const FREQ: [u32; 128]" in g.tables_rs()
    golden = g.golden_rs()
    assert "const CASES: [Case; 8]" in golden and "Lp24Mode::High" in golden
    assert g.golden_rs() == golden
