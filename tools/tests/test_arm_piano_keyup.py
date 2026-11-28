"""Unit tests for tools/arm_piano_keyup.py on synthetic ARM messages (no firmware, no unicorn)."""

from __future__ import annotations

import math
import struct
from dataclasses import dataclass

import arm_piano_keyup as ku
import arm_smp_slew_init as si


@dataclass(frozen=True)
class Msg:
    path: int
    word1: int
    word2: int


def fl(x: float) -> int:
    return struct.unpack("<I", struct.pack("<f", x))[0]


def rec(record: int, field: int, word: int, tick: int = 3, op: int = 0) -> tuple[int, Msg]:
    return tick, Msg(1, (op << 28) | si.addr(record, field), word)


def release(record: int, f16: float, tick: int = 3) -> list[tuple[int, Msg]]:
    """The four-word release write of one record."""
    return [
        rec(record, 14, fl(1.0), tick),
        rec(record, 12, 0, tick),
        rec(record, 11, 0, tick),
        rec(record, 16, fl(f16), tick),
    ]


def start(record: int, block: int, level: float, rate: float, ramp: float) -> list:
    """A voice start: pointer word, block words and the stroke descriptor write."""
    return [
        (40, Msg(1, block - 2, si.addr(record, 0))),
        (40, Msg(1, block + 2, fl(level))),
        (40, Msg(1, block + 4, fl(rate))),
        (40, Msg(1, block + 5, fl(ramp))),
        rec(record, -18, 0x9404E, 40),
    ]


def test_tick_is_the_24_khz_timer_interrupt():
    assert abs(ku.TICK_US - 41.6667) < 1e-3


def test_decode_writes_maps_records_and_drops_batch_framing():
    msgs = [
        (3, Msg(1, (9 << 28), 0x03992772)),
        (3, Msg(1, (10 << 28), 0)),
        rec(93, 16, fl(0.979331)),
        (4, Msg(2, 0x28000000, 5)),
        (5, Msg(1, 0x9251E, fl(0.38))),
    ]
    writes = ku.decode_writes(msgs)
    assert [(w.record, w.field) for w in writes] == [(93, 16), (-1, 0x9251E)]


def test_decay_tau_is_44_ms_for_the_release_word_and_none_for_hold():
    word = 0x3F7FBB6E
    assert abs(ku.decay_tau_ms(ku.as_float(word)) - 44.0) < 0.5
    assert abs(ku.decay_tau_ms(ku.as_float(0x3F7AB576)) - 2.2) < 0.1
    assert ku.decay_tau_ms(1.0) is None
    assert ku.decay_tau_ms(0.0) is None


def test_rate_decay_count_matches_the_documented_note_on_word():
    # D226: the key-up record rate word 0x3F7FF38E is a 0.2426 s time constant (d = 3186).
    d = ku.rate_decay_count(0x3F7FF38E)
    assert d == 3186.0
    tau = 1.0 / (d / (1 << 24) * ku.UPDATE_HZ)
    assert abs(tau - 0.2426) < 0.01
    assert ku.rate_decay_count(fl(1.0)) == 0.0


def test_release_and_hold_events_follow_the_f16_word():
    writes = ku.decode_writes(release(80, 1.0) + release(93, 0.979331, 4))
    evs = ku.events(writes)
    assert [(e.kind, e.record, e.tick) for e in evs] == [("hold", 80, 3), ("release", 93, 4)]
    assert evs[0].tau_ms is None and abs(evs[1].tau_ms - 2.2) < 0.1


def test_start_event_reads_level_rate_and_ramp_from_the_record_block():
    msgs = start(97, 0x92528, 1.0, ku.as_float(0x3F7FF38E), 0.25)
    (ev,) = ku.events(ku.decode_writes(msgs))
    assert (ev.kind, ev.record, ev.tick) == ("start", 97, 40)
    assert ev.level == 1.0 and ev.ramp == 0.25
    assert ev.rate_d == 3186.0


def test_start_picks_the_block_by_its_pointer_word():
    msgs = start(80, 0x9251C, 0.38, 0.99998, 0.0) + start(81, 0x92528, 0.39, 0.99998, 0.25)
    evs = {e.record: e for e in ku.events(ku.decode_writes(msgs))}
    assert abs(evs[80].level - 0.38) < 1e-6 and evs[80].ramp == 0.0
    assert abs(evs[81].level - 0.39) < 1e-6 and evs[81].ramp == 0.25


def test_start_without_a_block_keeps_none_fields():
    (ev,) = ku.events(ku.decode_writes([rec(97, -18, 1, 40)]))
    assert ev.kind == "start" and ev.level is None and ev.rate_d is None


def test_ramp_event_for_a_pedal_crossfade():
    msgs = [rec(81, -1, 0), rec(81, 2, 0), rec(81, 3, fl(0.141686)), rec(81, -1, 0x37560000)]
    (ev,) = ku.events(ku.decode_writes(msgs))
    assert ev.kind == "ramp" and abs(ev.ramp - 0.141686) < 1e-6


def test_midi_bytes_of_each_step_kind():
    assert ku.midi_bytes("cc", 127, 60) == bytes([0xB0, 64, 127])
    assert ku.midi_bytes("on", 90, 60) == bytes([0x90, 60, 90])
    assert ku.midi_bytes("off", 0, 60) == bytes([0x80, 60, 0])
    assert ku.midi_bytes("wait", 0, 60) == b""


def test_scenarios_end_with_a_long_tail_and_use_known_steps():
    kinds = {"cc", "on", "off", "wait"}
    for name, steps in ku.SCENARIOS.items():
        assert steps and all(k in kinds for k, _v, _t in steps), name
        assert steps[-1][2] >= 1000, f"{name}: the delayed key-up stage needs a tail"
    assert [s[0] for s in ku.SCENARIOS["d"]].count("on") == 2


def test_with_tail_replaces_only_the_last_run_length():
    steps = ku.SCENARIOS["a"]
    assert ku.with_tail(steps, None) == list(steps)
    changed = ku.with_tail(steps, 12000)
    assert changed[:-1] == list(steps[:-1]) and changed[-1] == ("cc", 0, 12000)
    assert ku.SCENARIOS["a"][-1][2] == ku.TAIL, "the table itself is not modified"


def test_with_cc_replaces_only_non_zero_pedal_values():
    steps = ku.SCENARIOS["half"]
    swept = ku.with_cc(steps, 100)
    assert swept[0] == ("cc", 100, 12) and swept[-1] == ("cc", 0, ku.TAIL)
    assert [k for k, _v, _t in swept] == [k for k, _v, _t in steps]
    assert ku.with_cc(steps, None) == list(steps)
    on = [s for s in ku.with_cc(ku.SCENARIOS["a"], 64) if s[0] == "on"]
    assert on == [("on", 90, 200)], "notes are not touched"


def test_again_scenarios_send_a_second_note_after_the_key_up():
    for name in ("again", "again_pedal"):
        kinds = [k for k, _v, _t in ku.SCENARIOS[name]]
        assert kinds.index("off") < len(kinds) - 1 and kinds[-1] == "on", name


def test_rows_use_absolute_ticks_and_microseconds():
    ev = ku.Event(40, "release", 80, f16=0.998954, tau_ms=44.0)
    (line,) = ku.rows("plain", 1, "off:0", 240, [ev])
    cols = line.split(",")
    assert cols[:7] == ["plain", "1", "off:0", "280", "11667", "release", "80"]
    assert math.isclose(float(cols[7]), 0.998954, rel_tol=1e-6)
    assert len(cols) == len(ku.CSV_HEADER.split(","))
