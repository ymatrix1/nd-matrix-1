"""Unit tests for tools/synth_e2e_metrics.py (levels, resampling, bands, null depth)."""

from __future__ import annotations

import numpy as np
import pytest
import synth_e2e_metrics as m


def sine(f: float, fs: float, n: int) -> np.ndarray:
    return np.sin(2 * np.pi * f * np.arange(n) / fs)


def test_db_rms_and_windowed_rms() -> None:
    assert m.db(1.0) == 0.0 and m.db(0.1) == pytest.approx(-20.0)
    assert m.db(0.0) < -500
    assert m.rms(np.array([3.0, -3.0])) == 3.0 and m.rms(np.array([])) == 0.0
    w = m.windowed_rms(np.concatenate([np.ones(8), 2 * np.ones(8), np.ones(3)]), 8)
    assert list(w) == [1.0, 2.0]


def test_resample_keeps_a_periodic_tone() -> None:
    x = sine(440.0, 8_000.0, 8_000)
    y = m.resample_fft(x, 8_000.0, 12_000.0)
    assert len(y) == 12_000
    assert np.max(np.abs(y - sine(440.0, 12_000.0, 12_000))) < 1e-9


def test_fundamental_finds_the_tone_and_the_band_levels_rank_it_first() -> None:
    fs = 48_000.0
    x = sine(1_000.0, fs, 48_000)
    assert m.fundamental_hz(x, fs) == pytest.approx(1_000.0, abs=0.5)
    assert m.fundamental_hz(np.zeros(1_000), fs) == 0.0
    edges = m.third_octave_edges()
    lv = m.band_levels_db(x, fs, edges)
    top = int(np.argmax(lv))
    assert edges[top][0] <= 1_000.0 < edges[top][1]


def test_null_depth_is_deep_for_scaled_copies_and_zero_for_unrelated_signals() -> None:
    a = sine(300.0, 48_000.0, 4_800)
    depth, gain = m.null_depth_db(a, 0.25 * a)
    assert depth < -200 and gain == pytest.approx(4.0)
    depth2, _ = m.null_depth_db(a, sine(777.0, 48_000.0, 4_800))
    assert depth2 > -1.0
    with pytest.raises(ValueError):
        m.null_depth_db(a, a[:-1])


def test_time_to_fraction() -> None:
    env = np.array([0.0, 0.2, 0.9, 1.0])
    assert m.time_to_fraction(env, 0.01, 0.9, 1.0) == pytest.approx(0.02)
    assert m.time_to_fraction(env, 0.01, 2.0, 1.0) == -1.0


def test_time_to_below_and_band_delta() -> None:
    env = np.array([1.0, 0.5, 0.05, 0.001])
    assert m.time_to_below(env, 0.01, 0.01, 1.0) == pytest.approx(0.03)
    assert m.time_to_below(env, 0.01, 1e-6, 1.0) == -1.0
    a = np.array([0.0, -10.0, -300.0])
    b = np.array([-2.0, -10.0, -500.0])
    assert m.band_delta_db(a, b) == pytest.approx(1.0)  # the two far-below bands are ignored
    assert m.band_delta_db(a, a - 30.0) == 0.0  # a pure level offset is not a shape difference
    # one band above the floor in one vector only counts as its distance to the floor
    one, other = np.array([0.0, 0.0]), np.array([0.0, -200.0])
    assert m.band_delta_db(one, other, 80.0) == pytest.approx(40.0)
    assert m.band_delta_db(np.array([-300.0]), np.array([-300.0]), -1.0) == 0.0
