"""Unit tests for tools/arm_flash_boot.py (message feeding only; the boot needs the firmware)."""

from __future__ import annotations

import arm_flash_boot as b
import arm_flash_model as fm
from arm_emu import Message


def test_in_window_is_bank1_only() -> None:
    assert b.in_window(0x0400_0040)
    assert not b.in_window(0x0800_0140)
    assert not b.in_window(0x9354A)


def test_feed_routes_messages_by_comm_and_resumes() -> None:
    chips = {3: fm.FlashChip("SY"), 4: fm.FlashChip("SMP")}
    msgs = [
        Message(1, 3, fm.BANK1 + 0x145, fm.CMD_QUERY),
        Message(1, 4, fm.BANK1 + 0x285, fm.CMD_QUERY),
        Message(1, 0, fm.BANK1 + 1, fm.CMD_QUERY),  # FX: ignored
    ]
    assert b.feed(chips, msgs[:1], 0) == 1
    assert chips[3].mode == "query" and chips[4].mode == "array"
    assert b.feed(chips, msgs, 1) == 3
    assert chips[4].mode == "query"
    assert len(chips[3].log) == 1
