import fx_chorus_comb as cc
import numpy as np
import pytest

FS = 44_100.0


def chorus_note(seconds: float, lo_ms: float, hi_ms: float, lfo_hz: float, gain: float):
    """A decaying 87.31 Hz harmonic note through `y = x + g x(t - d(t))`, d a sine in ms."""
    t = np.arange(int(seconds * FS)) / FS
    d = 1e-3 * (0.5 * (lo_ms + hi_ms) + 0.5 * (hi_ms - lo_ms) * np.sin(2 * np.pi * lfo_hz * t))

    def note(tt):
        k = np.arange(1, 50)[:, None]
        amp = 1.0 / k**0.7
        return (amp * np.sin(2 * np.pi * 87.31 * k * tt[None, :])).sum(0)

    return note(t) + gain * note(t - d), t


def test_comb_fit_recovers_the_delay_range_of_a_synthetic_chorus():
    x, _ = chorus_note(8.0, 0.9, 3.3, 0.34, 0.86)
    rows = cc.comb_delays(x / np.abs(x).max(), FS, 0.5, 7.5)
    assert len(rows) > 20
    fit = cc.lfo_fit(rows)
    assert fit["rate_hz"] == pytest.approx(0.34, abs=0.05)
    assert fit["min_ms"] == pytest.approx(0.9, abs=0.5)
    assert fit["max_ms"] == pytest.approx(3.3, abs=0.5)


def test_frame_delay_rejects_silence_and_a_pure_tone():
    t = np.arange(int(cc.WINDOW_S * FS)) / FS
    assert cc.frame_delay(np.zeros_like(t), FS, 0.86) is None
    assert cc.frame_delay(np.sin(2 * np.pi * 440 * t), FS, 0.86) is None


def test_lfo_fit_on_exact_rows():
    t = np.arange(0, 15, 0.2)
    d = 2.1 + 1.2 * np.sin(2 * np.pi * 0.36 * t)
    fit = cc.lfo_fit(np.stack([t, d, np.full_like(t, 0.5), np.full_like(t, 30.0)], 1))
    assert fit["centre_ms"] == pytest.approx(2.1, abs=0.1)
    assert fit["swing_ms"] == pytest.approx(1.2, abs=0.1)
