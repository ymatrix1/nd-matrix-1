import fx_chorus_oracle_delay as od
import numpy as np
import pytest


def train(delay: int, copy_gain: float, n: int = 4) -> np.ndarray:
    y = np.zeros(od.SPACING * n)
    for k in range(n):
        y[k * od.SPACING + 2] = 0.5
        y[k * od.SPACING + 2 + delay] = 0.5 * copy_gain
    return y


def test_copy_delays_reads_the_offset_and_the_gain_ratio():
    rows = od.copy_delays(train(120, 0.6))
    assert len(rows) == 3
    assert set(rows[:, 1]) == {120.0}
    assert rows[:, 2] == pytest.approx(0.6)


def test_impulse_train_spacing_and_stereo():
    x = od.impulse_train("train", od.SPACING * 3)
    assert x.shape == (od.SPACING * 3, 2)
    assert np.flatnonzero(x[:, 0]).tolist() == [0, od.SPACING, 2 * od.SPACING]


def test_summary_converts_frames_to_ms():
    rows = np.array([[0.0, 43.0, 0.5], [1.0, 86.0, 0.5]])
    s = od.summary(rows)
    assert s["min_ms"] == pytest.approx(43 / 43402.34375 * 1000, rel=1e-6)
    assert s["max_ms"] == pytest.approx(2 * s["min_ms"])
