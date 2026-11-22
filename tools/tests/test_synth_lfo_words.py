"""Unit tests for tools/synth_lfo_words.py (word law, last-word reduction, CSV text)."""

from __future__ import annotations

import csv
import io
from pathlib import Path

import synth_lfo_words as w

CSV_PATH = Path(__file__).resolve().parents[2] / "docs/research/data/synth-lfo-arm-words.csv"


def test_last_words_keeps_the_last_watched_word_only() -> None:
    msgs = [(0xE02EB, 1), (0x1234, 9), (0xE02EB, 2), (0xC13A9, 0x3F800000)]
    assert w.last_words(msgs) == {0xE02EB: 2, 0xC13A9: 0x3F800000}


def test_cutoff_depth_word_matches_the_captured_arm_words() -> None:
    with CSV_PATH.open(encoding="utf-8") as f:
        data = [r for r in csv.DictReader(f) if r["dst"] == "3" and r["dm_word"] == "0xe02eb"]
    assert [int(r["amt"]) for r in data] == [0, 32, 64, 127]
    for r in data:
        assert abs(w.cutoff_depth_word(int(r["amt"])) - int(r["word"], 16)) <= 4


def test_other_destinations_do_not_write_the_cutoff_depth_word() -> None:
    with CSV_PATH.open(encoding="utf-8") as f:
        data = list(csv.DictReader(f))
    checked = 0
    for r in data:
        if r["dst"] != "3" and r["dm_word"] == "0xe02eb":
            assert int(r["word"], 16) == 0
            checked += 1
    assert checked == 12


def test_rows_csv_has_header_and_rows() -> None:
    text = w.rows_csv([{"dst": 3, "amt": 127, "dm_word": "0xe02eb", "word": "0x40000000"}])
    got = list(csv.reader(io.StringIO(text)))
    assert got[0] == list(w.FIELDS) and got[1][3] == "0x40000000"
