"""Unit tests for `tools/synth_osc_env_pitch.py` (pure helpers; the oracle runs need firmware)."""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import synth_osc_env_pitch as m  # noqa: E402

FS = 48_000.0


def test_peak_pitch_of_a_sine_is_exact_to_a_few_cents() -> None:
    n = np.arange(4096)
    for f in (65.4, 262.0, 1046.5):
        x = np.sin(2 * np.pi * f * n / FS)
        assert abs(m.semitones(m.pitch_peak_hz(x, FS), f)) < 0.05


def test_peak_pitch_of_silence_is_zero() -> None:
    assert m.pitch_peak_hz(np.zeros(1024), FS) == 0.0


def test_trace_follows_a_pitch_step() -> None:
    n = np.arange(24_000)
    f = np.where(n < 12_000, 440.0, 880.0)
    x = np.sin(2 * np.pi * np.cumsum(f) / FS)
    tr = m.pitch_trace(x, FS, 2048, 1024, 20.0, 4000.0)
    assert abs(m.semitones(tr[0, 1], 440.0)) < 0.2
    assert abs(m.semitones(tr[-1, 1], 880.0)) < 0.2
    assert tr[0, 0] == 1024.0


def test_model_is_48_semitones_per_word_and_level() -> None:
    assert m.model_semitones(1.0, 1.0) == 48.0
    assert m.model_semitones(-0.5, 0.5) == -12.0
    assert m.model_semitones(0.3, 0.0) == 0.0


def test_semitones_guards_non_positive_input() -> None:
    assert m.semitones(0.0, 440.0) == 0.0
    assert abs(m.semitones(880.0, 440.0) - 12.0) < 1e-9


def test_dsp_word_port_hits_ends_and_is_odd() -> None:
    table = [48.0 * i / 127.0 for i in range(128)]
    assert m.dsp_word(0.0, table) == 0.0
    assert abs(m.dsp_word(60.0, table) - 1.0) < 1e-6
    assert abs(m.dsp_word(-30.0, table) + m.dsp_word(30.0, table)) < 1e-9
    assert abs(m.dsp_word(90.0, table) - 1.0) < 1e-6


def test_at_times_interpolates_a_trace() -> None:
    trace = np.array([[0.0, 0.0], [48_000.0, 10.0]])
    assert abs(m.at_times(trace, FS, 0.0, [0.5])[0] - 5.0) < 1e-9


def test_f2w_is_the_float32_bit_pattern() -> None:
    assert m.f2w(1.0) == 0x3F800000
    assert m.f2w(-1.0) == 0xBF800000


def test_layer_addresses_follow_the_slot_layout() -> None:
    assert [m.word12_addr(k) for k in range(3)] == [0xE02F4, 0xE0314, 0xE0334]
    assert [m.osc_adr_addr(k) for k in range(3)] == [0xE0020, 0xE009C, 0xE0118]
    assert [m.level_addr(k) for k in range(4)] == [0xC118E, 0xC118F, 0xC11CC, 0xC11CD]
    assert [m.samples_addr(k) for k in range(3)] == [0xC1F89, 0xC1FBF, 0xC1FF5]


def test_velocity_factor_is_g_over_128_with_a_full_top() -> None:
    assert m.vel_factor(0) == 0.0
    assert m.vel_factor(64) == 0.5
    assert m.vel_factor(126) == 126 / 128
    assert m.vel_factor(127) == 1.0


def test_key_word_carries_the_velocity_byte() -> None:
    assert m.key_word(98) == 0x0B00E200  # the factory key 60, velocity 100
    assert m.key_word(0) == 0x0B008000
