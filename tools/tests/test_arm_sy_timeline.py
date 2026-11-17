"""Tests of `arm_sy_timeline` (D153): consumed source frames, voice busy and free ticks."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import arm_dsp_return as rtn  # noqa: E402
import arm_sy_timeline as tl  # noqa: E402


def sample(tick: int, idle: int = 1, state: int = 0, **kw) -> tl.Sample:  # noqa: ANN003
    return tl.Sample(tick, idle, 0, state, kw.pop("step", 0), 0, kw.pop("passes", 1), **kw)


def test_consumed_frames_count_only_the_playing_state() -> None:
    t = tl.SyTimeline()
    t.sample(sample(0, state=tl.STATE_WAITING, step=0xA00000))
    t.sample(sample(1, state=tl.STATE_PLAYING, step=0xA00000, passes=2))
    t.sample(sample(2, state=tl.STATE_PLAYING, step=0x2000000, passes=1))
    # 2 passes x 2 frames x 0.625 = 2.5, then 1 pass x 2 frames x 2.0 = 4.0
    assert t.consumed() == [0.0, 2.5, 6.5]


def test_summary_reports_busy_playing_free_and_the_first_ring() -> None:
    t = tl.SyTimeline()
    prep = (3, "SY", 1, rtn.STATE_BASE, rtn.STATE_PREPARE)
    t.message(*prep)
    t.message(12, "SY", 1, rtn.STATE_BASE, rtn.STATE_RESET)
    t.sample(sample(2, idle=1))
    t.sample(sample(3, idle=0, state=tl.STATE_WAITING))
    # 40 passes x 2 frames x 2.0 = 160 source frames: past the 128 frame ring
    t.sample(sample(4, idle=0, state=tl.STATE_PLAYING, step=0x2000000, passes=40, peak=0.5))
    t.sample(sample(10, idle=0, state=tl.STATE_PLAYING, step=0x2000000, passes=1, peak=0.25))
    t.sample(sample(11, idle=0, state=tl.STATE_RELEASE))
    t.sample(sample(13, idle=1))
    out = t.summary([3], 10)
    assert (out["voice0_busy_tick"], out["voice0_playing_tick"], out["voice0_release_tick"]) == (
        3,
        4,
        11,
    )
    assert out["voice0_free_after_off_ticks"] == 3 and out["first_audio_tick"] == 4
    assert out["source_frames"] == 164.0 and out["past_first_ring"] is True
    assert out["audio_ticks_after_first_ring"] == 2 and out["peak"] == 0.5
    assert out["key_down_delay_ticks"] == [0] and out["free_after_off_ticks"] == 2
    # one row per change of (idle bit, release bit, state, step); the repeated playing sample of
    # tick 10 and the equal state at tick 13 do not add rows
    assert [row[0] for row in out["transitions"]] == [2, 3, 4, 11, 13]
    assert out["transitions"][2] == [4, 0, 0, hex(tl.STATE_PLAYING), hex(0x2000000)]


def test_an_empty_timeline_reports_none_not_zero() -> None:
    out = tl.SyTimeline().summary([0], 5)
    assert out["voice0_busy_tick"] is None and out["voice0_free_after_off_ticks"] is None
    assert out["past_first_ring"] is False and out["first_audio_tick"] is None


class FakeLink:
    def __init__(self, words: dict[int, int]) -> None:
        self.words = words

    def dump(self, addr: int, count: int) -> list[int]:
        return [self.words.get(addr, 0)] * count


class Report:
    cnt, peak, sumsq = 4, 0.5, 0.25


def test_the_probe_reads_a_live_chip_and_falls_back_to_the_message_mask() -> None:
    t = tl.SyTimeline()
    link = FakeLink({rtn.IDLE_MASK: 0xFFFFFFFE, tl.STATE: tl.STATE_PLAYING, tl.STEP: 0xA00000})
    s = tl.SyProbe(t, link=link).take(7, 2, Report())
    assert (s.tick, s.idle, s.state, s.step, s.passes, s.peak) == (
        7,
        0xFFFFFFFE,
        tl.STATE_PLAYING,
        0xA00000,
        2,
        0.5,
    )
    src = rtn.ReturnSource()
    src.observe(3, 0, 1, rtn.STATE_BASE, rtn.STATE_PREPARE)
    stub = tl.SyProbe(t, source=src).take(8, 1, None)
    assert (stub.idle & 1, stub.state, stub.peak) == (0, 0, 0.0)
    assert len(t.samples) == 2


def test_a_cold_chip_sample_before_the_first_pass_is_not_a_busy_voice() -> None:
    t = tl.SyTimeline()
    t.sample(sample(0, idle=0))  # DM(0x92196) is still 0 before the first pass
    t.sample(sample(1, idle=1))
    t.sample(sample(5, idle=0, state=tl.STATE_WAITING))
    assert t.summary([5], 9)["voice0_busy_tick"] == 5


def test_state_messages_keep_only_the_voice_state_stores() -> None:
    t = tl.SyTimeline()
    t.message(3, "SY", 1, rtn.STATE_BASE, rtn.STATE_PREPARE)
    t.message(4, "SY", 1, 0x92500, 7)  # not a state word
    t.message(5, "SY", 1, rtn.STATE_BASE + rtn.STATE_STRIDE, rtn.STATE_GO)
    t.message(6, "PNO", 1, rtn.STATE_BASE, 1)  # another chip
    assert t.state_messages() == [
        [3, hex(rtn.STATE_BASE), hex(rtn.STATE_PREPARE)],
        [5, hex(rtn.STATE_BASE + rtn.STATE_STRIDE), hex(rtn.STATE_GO)],
    ]
