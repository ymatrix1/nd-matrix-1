"""Unit tests for tools/arm_arp_events.py (pure helpers; the Unicorn runs need local firmware)."""

from __future__ import annotations

import arm_arp_events as ev
import pytest


def test_phase_units_follow_the_tempo():
    # 100 BPM: 100 / 60 firmware beats per second of 2**19 units, 24,000 ticks per second.
    assert ev.phase_units(24_000, 100.0) == pytest.approx(100 / 60 * 2**19)
    assert ev.phase_units(0, 100.0) == 0.0


def test_a_quarter_beat_at_100_bpm_is_150_ms():
    # Grid index 6 (32 points per 4 beats, a step = 2 points) is a quarter of a beat.
    step_units = 2**19 / 4
    ticks = step_units / ev.phase_units(1, 100.0)
    assert ev.ticks_to_ms(ticks) == pytest.approx(150.0)


def test_position_word_is_a_24_bit_value_in_the_upper_bits():
    assert ev.position_word(0.0) == 0
    assert ev.position_word(1.9) == 1 << 8
    assert ev.position_word(0x1000001) == 1 << 8  # wraps at 24 bits
    assert ev.position_word(0xFFFFFF) == 0xFFFFFF << 8


def test_constants_name_the_documented_firmware_sites():
    assert ev.PLAYER == 0xC023B3EC
    assert ev.POSITION_SITES == (0xC023DED8, 0xC023DFD8)
    assert ev.LAYER_OF_POSITION == {1: "A", 3: "B", 5: "C"}
    assert ev.INSNS_PER_TICK == 25_000
