"""Unit tests for tools/msg_script_scan.py (message script scan, D190)."""

from __future__ import annotations

from pathlib import Path

import msg_script_scan as ms

LINES = [
    "1670 0xb2f26 0x00026000  # tick 77 pc 0xc020d9fc",
    "0 0x954ee 0x000b33c2",
    "5 0xc2e90 0x001256ad",
    "5 0xc01e6 0x00121e2b",
    "6 0xc01ea 0x28495ab4",
    "# comment only",
    "bad line",
    "7 0xzz 0x1",
]
ROWS = ms.parse_rows(LINES)


def test_parse_rows_skips_comments_and_bad_lines() -> None:
    assert len(ROWS) == 5
    assert ROWS[0] == (1670, 0xB2F26, 0x26000)


def test_region_hits_is_half_open() -> None:
    assert ms.region_hits(ROWS, 0xC2E00, 0xC3100) == [(5, 0xC2E90, 0x1256AD)]
    assert ms.region_hits(ROWS, 0xC2E00, 0xC2E90) == []


def test_value_hits_match_raw_and_code_record() -> None:
    hits = ms.value_hits(ROWS, 0x1256AD)
    assert [h[1] for h in hits] == [0xC2E90, 0xC01EA]
    assert ms.value_hits(ROWS, 0x1237F8) == []


def test_code_census_lists_window_values_and_addresses() -> None:
    census = ms.code_census(ROWS)
    assert census == {0x121E2B: [0xC01E6], 0x1256AD: [0xC2E90]}


def test_main_prints_counts(tmp_path: Path, capsys) -> None:  # noqa: ANN001
    script = tmp_path / "m.script"
    script.write_text("\n".join(LINES), encoding="utf-8")
    assert ms.main([str(script), "region", "0xC2E00", "0xC3100"]) == 0
    out = capsys.readouterr().out
    assert "1 stores in [0xc2e00, 0xc3100)" in out
    assert ms.main([str(script), "codes"]) == 0
    assert "2 distinct code-window values" in capsys.readouterr().out
    assert ms.main([str(script), "value", "0x1256AD"]) == 0
    assert "2 stores of 0x1256AD" in capsys.readouterr().out
