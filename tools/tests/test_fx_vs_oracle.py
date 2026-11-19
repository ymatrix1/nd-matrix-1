"""Tests of tools/fx_vs_oracle.py: case table, stimuli and comparison metrics (pure functions).

No firmware image or built binary is needed.
"""

import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import fx_vs_oracle as fv  # noqa: E402


def test_case_table_covers_every_family_and_step():
    cs = fv.cases(None)
    fams = {c["family"] for c in cs}
    assert {"identity", "M2-CHOR0", "M2-SPIN1", "M1-APAN0", "M1-PUMP1", "COMP", "REVERB0"} <= fams
    assert {"RVBM-2", "RVBM-11"} <= fams and "RVBM-6" not in fams  # type 6 has no words
    chor = [c for c in cs if c["family"] == "M2-CHOR0"]
    assert {c["stim"] for c in chor} == set(fv.STIMULI)
    assert {(c["rate"], c["amount"]) for c in chor} == {(64, 64), *fv.STEPS}
    assert [c["stim"] for c in cs if c["family"] == "REVERB0"] == list(fv.STIMULI)
    assert fv.cases(["COMP"]) and {c["family"] for c in fv.cases(["COMP"])} == {"COMP"}


def test_stimuli_are_deterministic_and_have_the_stated_energy_end():
    for kind in fv.STIMULI:
        a, end = fv.stimulus(kind, 60_000)
        b, _ = fv.stimulus(kind, 60_000)
        assert a.shape == (60_000, 2) and np.array_equal(a, b)
        assert np.isfinite(a).all() and np.abs(a).max() > 0.05, kind
        assert 1 <= end < 60_000
    sweep, end = fv.stimulus("sweep", 60_000)
    assert np.abs(sweep[end + 100 :]).max() == 0.0  # silent after the sweep


def test_metrics_read_a_scaled_copy_as_a_pure_gain():
    rng = np.random.default_rng(3)
    x = rng.uniform(-0.2, 0.2, (4000, 2)).astype(np.float32)
    got = fv.metrics(0.5 * x, x, 100, 0.5)
    assert got["lag"] == 0 and abs(got["level_delta_db"]) < 0.01
    assert got["null_fit_db"] < -100
    off = fv.metrics(x, x, 100, 0.5)  # +6 dB against the chain gain
    assert abs(off["level_delta_db"] - 6.02) < 0.05
    shifted = fv.metrics(np.roll(x, 3, axis=0), x, 100, 1.0)
    assert shifted["lag"] == 3  # oracle delayed by 3: a[n + 3] = b[n]


def test_band_gains_report_a_single_band_boost():
    n = 1 << 14
    t = np.arange(n) / fv.FRAME_RATE
    x = np.sin(2 * np.pi * 1500 * t)
    g = fv.band_gains(2.0 * x, x, 1.0)
    assert len(g) == len(fv.BANDS)
    assert abs(g[5] - 6.02) < 0.1  # 1-2 kHz band holds the tone
