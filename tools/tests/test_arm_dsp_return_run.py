"""Unit tests for tools/arm_dsp_return_run.py (the pure summary helpers)."""

from __future__ import annotations

import arm_dsp_return as rt
import arm_dsp_return_run as run

W = rt.STATE_BASE  # state message address of voice 0
W1 = rt.STATE_BASE + rt.STATE_STRIDE  # voice 1


def row(tick: int, w1: int, w2: int, chip: str = "SY", path: int = 1) -> tuple:
    return (tick, chip, path, w1, w2)


def test_is_sy_state_needs_chip_path_and_voice_address() -> None:
    assert run.is_sy_state(row(0, W, rt.STATE_PREPARE), (rt.STATE_PREPARE,))
    assert not run.is_sy_state(row(0, W, rt.STATE_GO), (rt.STATE_PREPARE,))
    assert not run.is_sy_state(row(0, W, rt.STATE_PREPARE, chip="ORG"), (rt.STATE_PREPARE,))
    assert not run.is_sy_state(row(0, 0x1234, rt.STATE_PREPARE), (rt.STATE_PREPARE,))
    assert not run.is_sy_state(row(0, W, rt.STATE_PREPARE, path=2), (rt.STATE_PREPARE,))


def test_key_down_delay_is_limited_to_the_note_window() -> None:
    sy = [row(5, W, rt.STATE_PREPARE), row(27, W1, rt.STATE_PREPARE)]
    assert run.key_down_delay(sy, 0, 20) == 5
    assert run.key_down_delay(sy, 20, None) == 7
    assert run.key_down_delay(sy, 0, 3) is None  # nothing before the next note
    assert run.key_down_delay([], 0, None) is None


def test_summarize_counts_states_delays_and_the_first_free() -> None:
    rows = [
        row(3, W, rt.STATE_PREPARE),
        row(4, W, rt.STATE_GO),
        row(24, W1, rt.STATE_PREPARE),
        row(26, 0x93000, 1),  # another SY message (not a state)
        row(130, W, rt.STATE_RESET),
        row(140, W1, rt.STATE_IDLE),
        row(150, 0x100, 5, chip="ORG", path=2),
    ]
    s = run.summarize(rows, [0, 20], 100)
    assert s["sy_messages"] == 6
    assert s["messages_by_chip"] == {"ORG": 1, "SY": 6}
    assert s["key_down_delay_ticks"] == [3, 4]
    assert (s["prepare"], s["go"]) == (2, 1)
    assert s["free_after_off_ticks"] == 30 and s["frees"] == 2


def test_summarize_without_a_free_reports_none() -> None:
    s = run.summarize([row(3, W, rt.STATE_PREPARE)], [0], 100)
    assert s["free_after_off_ticks"] is None and s["frees"] == 0
