"""Unit tests for tools/gate_edge_probe.py (scripts, one-pole fit, edge numbers)."""

from __future__ import annotations

import math

import gate_edge_probe as g
import numpy as np
import pytest


def one_pole(c: float, start: float, target: float, n: int) -> np.ndarray:
    x = [start]
    for _ in range(n):
        x.append(x[-1] + c * (target - x[-1]))
    return np.array(x, dtype=np.float32)


def row(phase: str, chip: str, core: int, dm: int, w2: int) -> dict:
    return {"phase": phase, "chip": chip, "core": core, "dm": dm, "w2": w2}


def test_load_script_keeps_org_core2_load_and_idle_rows() -> None:
    rows = [
        row("load", "ORG", 2, 0xE0000, 0x1),
        row("idle", "ORG", 2, 0xE0001, 0x2),
        row("load", "ORG", 1, 0xE0002, 0x3),
        row("hold", "ORG", 2, 0xE0003, 0x4),
        row("load", "PNO", 2, 0xE0004, 0x5),
    ]
    assert g.load_script(rows) == "0 0xe0000 0x00000001\n0 0xe0001 0x00000002\n"


def test_gate_script_writes_all_six_words_as_float_bits() -> None:
    lines = g.gate_script([(10, 1.0), (20, 0.0)]).strip().splitlines()
    assert len(lines) == 12
    assert lines[0] == "10 0xc1396 0x3f800000"
    assert lines[-1] == "20 0xc1437 0x00000000"


def test_fit_recovers_the_coefficient_of_a_falling_and_rising_edge() -> None:
    for series, target in (
        (one_pole(0.005, 1.0, 0.0, 600), 0.0),
        (one_pole(0.005, 0.0, 1.0, 600), 1.0),
    ):
        c, dev = g.fit_one_pole(series, 0, target)
        assert c == pytest.approx(0.005, rel=1e-3)
        assert dev < 1e-6


def test_fit_rejects_a_settled_series() -> None:
    with pytest.raises(ValueError):
        g.fit_one_pole(np.ones(10, dtype=np.float32), 0, 1.0)


def test_time_constant_of_c_005_is_about_200_passes() -> None:
    assert g.time_constant_passes(0.005) == pytest.approx(199.5, abs=0.1)
    assert g.time_constant_passes(1 - math.exp(-1 / 50)) == pytest.approx(50.0)


def test_rise_10_90_of_a_one_pole_is_2_2_tau() -> None:
    rise = one_pole(0.005, 0.0, 1.0, 1500)
    assert g.rise_10_90(rise, 0, 0.0, 1.0) == pytest.approx(2.197 * 199.5, abs=2)
    fall = one_pole(0.005, 1.0, 0.0, 1500)
    assert g.rise_10_90(fall, 0, 1.0, 0.0) == g.rise_10_90(rise, 0, 0.0, 1.0)


def test_rise_10_90_rejects_unfinished_step() -> None:
    with pytest.raises(ValueError):
        g.rise_10_90(one_pole(0.005, 0.0, 1.0, 20), 0, 0.0, 1.0)


def test_max_step_fraction_separates_hard_step_from_smoothed() -> None:
    hard = np.array([1, 1, 0, 0], dtype=np.float32)
    assert g.max_step_fraction(hard, 0, 1.0, 0.0) == 1.0
    assert g.max_step_fraction(one_pole(0.005, 1.0, 0.0, 50), 0, 1.0, 0.0) == pytest.approx(0.005)


def test_measure_reports_both_edges() -> None:
    series = np.concatenate([one_pole(0.005, 1.0, 0.0, 1200), one_pole(0.005, 0.0, 1.0, 1200)[1:]])
    m = g.measure(series, 0, 1200)
    assert m["fall"]["c_per_pass"] == pytest.approx(0.005, rel=1e-3)
    assert m["rise"]["tau_ms"] == pytest.approx(9.2, abs=0.05)
    assert m["rise"]["max_step_fraction"] == pytest.approx(0.005, rel=1e-2)
