"""Unit tests for tools/arm_flash_model.py (the model only; no firmware needed)."""

from __future__ import annotations

import arm_flash_model as m


def test_query_mode_returns_qry_at_the_probe_addresses() -> None:
    sy = m.FlashChip("SY")
    sy.message(m.BANK1 + 0x145, m.CMD_QUERY)
    assert [sy.read(m.BANK1 + a) for a in (0x40, 0x41, 0x42)] == [0x51, 0x52, 0x59]
    smp = m.FlashChip("SMP")
    smp.message(m.BANK1 + 0x285, m.CMD_QUERY)
    assert [smp.read(0x80 + k) for k in range(3)] == [0x51, 0x52, 0x59]


def test_reset_returns_to_array_mode_and_erased_words() -> None:
    f = m.FlashChip("SY")
    f.message(m.BANK1 + 5, m.CMD_QUERY)
    f.message(m.BANK1, m.CMD_RESET)
    assert f.mode == "array"
    assert f.read(m.BANK1 + 0x40) == m.ERASED


def test_program_writes_the_next_word_under_the_latch() -> None:
    f = m.FlashChip("SY")
    f.message(m.BANK2 + 0x140, 0x8001)
    f.message(m.BANK1 + 0x555, m.CMD_PROGRAM)
    f.message(m.BANK1 + 0x10, 0x1234)
    assert f.read(m.BANK1 + 0x10) == 0x1234
    f.message(m.BANK2, 0x8000)  # another latch page does not see it
    assert f.read(m.BANK1 + 0x10) == m.ERASED


def test_autoselect_and_size_word() -> None:
    f = m.FlashChip("SY", size_log2=27)
    f.message(m.BANK1, m.CMD_AUTOSELECT)
    assert f.read(0) == 0x0001
    assert m.cfi_word(0x27, 27) == 27
    assert m.cfi_word(0x7F, 27) == 0


def test_messages_outside_the_banks_are_ignored() -> None:
    f = m.FlashChip("SY")
    f.message(0x93580, 0x80008000)
    assert f.log == []
