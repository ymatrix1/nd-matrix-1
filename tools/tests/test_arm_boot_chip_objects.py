"""Unit tests for tools/arm_boot_chip_objects.py (formatting only; the run needs the firmware)."""

from __future__ import annotations

import arm_boot_chip_objects as m


def test_format_row_names_known_chips() -> None:
    assert m.format_row(3, 0xC0444E4C, 0, 0) == (
        "SY: descriptor 0xC0444E4C, 7 byte records 0, 9 byte records 0"
    )
    assert m.format_row(4, 0xC0444E68, 2, 5).startswith("SMP: ")


def test_format_row_unknown_chip_and_hook_address() -> None:
    assert m.format_row(9, 0, 1, 1).startswith("chip9: ")
    assert m.HOOK_PC == 0xC0248744
