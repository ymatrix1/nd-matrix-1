"""Unit tests for tools/arm_table_watch.py (range and edit parsing, access summary)."""

from __future__ import annotations

import arm_table_watch as w
import pytest


def test_parse_range_hex_and_decimal() -> None:
    assert w.parse_range("0xC0545128:0xC0545148") == (0xC0545128, 0xC0545148)
    assert w.parse_range("16:32") == (16, 32)


@pytest.mark.parametrize("text", ["0x10:0x10", "0x20:0x10"])
def test_parse_range_rejects_empty(text: str) -> None:
    with pytest.raises(ValueError):
        w.parse_range(text)


def test_parse_edit() -> None:
    assert w.parse_edit("0x189=1") == (0x189, 1)
    assert w.parse_edit("394=0x64") == (394, 100)
    with pytest.raises(ValueError):
        w.parse_edit("0x189")


def test_summarise_counts_and_keeps_first_address() -> None:
    events = [
        ("read", 0x100, 0x200, 0xA0),
        ("read", 0x100, 0x200, 0xA8),
        ("write", 0x104, 0x200, 0xB0),
        ("read", 0x100, 0x300, 0xA0),
    ]
    rows = w.summarise(events)
    assert rows == [
        ("read", 0x100, 0x200, 0xA0, 2),
        ("read", 0x100, 0x300, 0xA0, 1),
        ("write", 0x104, 0x200, 0xB0, 1),
    ]


def test_summarise_empty() -> None:
    assert w.summarise([]) == []


def test_format_line() -> None:
    line = w.format_line(("read", 0xC0013CA8, 0, 0xC0545128, 2))
    assert line == "read  pc 0xc0013ca8 lr 0x00000000 first 0xc0545128 x2"
