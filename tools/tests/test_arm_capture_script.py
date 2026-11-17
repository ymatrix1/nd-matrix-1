"""Unit tests for tools/arm_capture_script.py (row selection and script lines)."""

from __future__ import annotations

import arm_capture_script as s


def row(phase: str, tick: int, chip: str, core: int, dm: int, w2: int) -> dict[str, object]:
    return {
        "phase": phase,
        "tick": tick,
        "chip": chip,
        "path": 2,
        "core": core,
        "dm": dm,
        "w2": w2,
        "pc": 0,
    }


ROWS = [
    row("load", 3, "PNO", 1, 0xB2F26, 0x26000),
    row("load", 4, "PNO", 2, 0xE02F8, 0),
    row("load", 5, "FX", 1, 0xB0473, 0xB0471),
    row("k60", 9, "PNO", 1, 0xC01D6, 0x122A71),
]


def test_select_filters_chip_core_and_phase() -> None:
    assert [r["dm"] for r in s.select(ROWS, "PNO", 1, None)] == [0xB2F26, 0xC01D6]
    assert [r["dm"] for r in s.select(ROWS, "PNO", 1, {"k60"})] == [0xC01D6]
    assert list(s.select(ROWS, "ORG", 1, None)) == []


def test_script_lines_flat_and_per_tick() -> None:
    picked = list(s.select(ROWS, "PNO", 1, None))
    assert s.script_lines(picked) == ["0 0xb2f26 0x00026000", "0 0xc01d6 0x00122a71"]
    assert s.script_lines(picked, pass_per_tick=2) == [
        "6 0xb2f26 0x00026000",
        "18 0xc01d6 0x00122a71",
    ]
