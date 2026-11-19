"""Unit tests for tools/sample_gaps_probe.py (row summaries; the run needs firmware)."""

from __future__ import annotations

import sample_gaps_probe as sp

ROWS = [
    "key 1 SY 1 0 0x92414 0x001c0794 0xc01d66d4",
    "key 1 SY 1 0 0x92414 0x001c07cd 0xc01e11ac",
    "key 2 PNO 2 2 0xe0000 0x00000001 0xc0000000",
]


def test_summarize_counts_chip_and_path() -> None:
    assert sp.summarize(ROWS) == {"SY:1": 2, "PNO:2": 1}
    assert sp.summarize([]) == {}


def test_sy_rows_keeps_only_sy() -> None:
    assert sp.sy_rows(ROWS) == ROWS[:2]
