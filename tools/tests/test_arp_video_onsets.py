from __future__ import annotations

import wave

import arp_video_onsets as v
import numpy as np
import pytest

SR = 48_000


def click_train(period_s: float, seconds: float, jitter: float = 0.0) -> np.ndarray:
    """Short decaying noise bursts every `period_s` seconds."""
    rng = np.random.default_rng(1)
    x = rng.normal(0, 0.002, int(seconds * SR))
    t = 0.3
    while t < seconds - 0.1:
        n = int(0.03 * SR)
        burst = rng.normal(0, 1, n) * np.exp(-np.arange(n) / (0.006 * SR))
        i = int(t * SR)
        x[i : i + n] += 0.5 * burst
        t += period_s + rng.normal(0, jitter)
    return x


def write_wav(path, x, channels: int = 1) -> None:
    with wave.open(str(path), "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes((np.clip(x, -1, 1) * 32767).astype("<i2").tobytes())


def test_autocorr_finds_the_click_period_and_its_multiples():
    flux, dt = v.onset_flux(click_train(0.15, 12.0), SR)
    peaks = dict(v.autocorr_peaks(flux, dt))
    assert any(abs(p - 150.0) < 3.0 for p in peaks)
    assert any(abs(p - 300.0) < 3.0 for p in peaks)


def test_comb_period_fits_a_jittered_grid():
    flux, dt = v.onset_flux(click_train(0.1128, 12.0, jitter=0.002), SR)
    period, strength = v.comb_period(v.pick_onsets(flux, dt))
    assert abs(period - 112.8) < 2.0, (period, strength)
    assert strength > 0.8


def test_analyze_reads_a_wav_and_reports_windows(tmp_path):
    path = tmp_path / "clip.wav"
    write_wav(path, click_train(0.15, 10.0))
    out = v.analyze(path, t0=100.0, windows=[(101.0, 105.0), (105.0, 109.0)])
    assert len(out["windows"]) == 2
    assert abs(out["comb_period_ms"] - 150.0) < 2.0
    assert out["onsets"] > 20


def test_parse_window_and_wav_validation(tmp_path):
    assert v.parse_window("287:300.5") == (287.0, 300.5)
    stereo = tmp_path / "s.wav"
    write_wav(stereo, np.zeros(200), channels=2)
    with pytest.raises(ValueError):
        v.read_wav_mono(stereo)
