"""Unit tests for tools/sy_script_window_census.py (synthetic scripts only)."""

from __future__ import annotations

import json

import pytest
import sy_script_window_census as c

SCRIPT = "0 0x93df6 0x1\n# note\n0 0x9354a 0x3d4ccccd\n\n0 0x921a6 0x0\n"


def test_parse_script_skips_comments_and_blanks() -> None:
    assert c.parse_script(SCRIPT) == [(0, 0x93DF6, 1), (0, 0x9354A, 0x3D4CCCCD), (0, 0x921A6, 0)]


def test_parse_script_rejects_a_short_line() -> None:
    with pytest.raises(ValueError, match="bad script line"):
        c.parse_script("0 0x93df6\n")


def test_window_hits_ignore_opcode_bits_and_edges() -> None:
    rows = [(0, 0x93500, 0), (0, 0x935A0, 0), (0, 0x1009_3510, 0), (0, 0x934FF, 0)]
    assert c.window_hits(rows) == [0, 2]


def test_summarise_counts_and_range() -> None:
    s = c.summarise(c.parse_script(SCRIPT))
    assert s["messages"] == 3
    assert s["distinct_addresses"] == 3
    assert (s["min_address"], s["max_address"]) == (0x921A6, 0x93DF6)
    assert s["window_hits"] == [1]


def test_summarise_empty_script() -> None:
    assert c.summarise([])["min_address"] is None


def test_compare_equal_different_and_length() -> None:
    a = [(0, 1, 2), (0, 3, 4)]
    assert c.compare(a, list(a))["equal"] is True
    assert c.compare(a, [(0, 1, 2), (0, 3, 5)])["first_difference"] == 1
    r = c.compare(a, a[:1])
    assert (r["equal"], r["first_difference"]) == (False, 1)


def test_watch_changes_counts_changed_words() -> None:
    result = {
        "watch": {"SY0": {"0x1": {"first_change_pass": None}, "0x2": {"first_change_pass": 7}}}
    }
    assert c.watch_changes(result) == {"watched": 2, "changed": 1}
    assert c.watch_changes({}) == {"watched": 0, "changed": 0}


def test_main_prints_json(tmp_path, capsys) -> None:  # noqa: ANN001
    a = tmp_path / "a.script"
    a.write_text(SCRIPT)
    j = tmp_path / "a.json"
    j.write_text(json.dumps({"watch": {"SY0": {"0x1": {"first_change_pass": None}}}}))
    assert c.main([str(a), str(a), "--json-a", str(j)]) == 0
    out = json.loads(capsys.readouterr().out)
    assert out["compare"]["equal"] is True
    assert out["watch_a"] == {"watched": 1, "changed": 0}
