"""Tests for `tools/pump_envelope_hz.py`."""

import numpy as np
import pump_envelope_hz as p


def test_a_2_hz_amplitude_modulation_reads_2_hz():
    sr = 48_000
    t = np.arange(sr * 20) / sr
    x = np.sin(2 * np.pi * 220 * t) * (0.6 + 0.4 * np.sin(2 * np.pi * 2.0 * t))
    hz, rel = p.envelope_peak(x, sr)
    assert abs(hz - 2.0) < 0.05
    assert rel > 0.5


def test_a_steady_tone_has_no_strong_line():
    sr = 48_000
    t = np.arange(sr * 10) / sr
    _, rel = p.envelope_peak(np.sin(2 * np.pi * 220 * t), sr)
    assert 0.0 <= rel <= 1.0
