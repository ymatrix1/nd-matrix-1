"""Tests for tools/fx_stage_law.py: the dispatcher stage cross-fade law and its oracle table."""

import csv
from pathlib import Path

import fx_stage_law as sl

DATA = Path(__file__).resolve().parents[2] / "docs/research/data/fx-stage-law.csv"


def test_default_record_is_muted_into_a_pure_bus():
    # image words: level -0.5, step -0.01, limit 0.5: c = -0.5, g = 0, the bus passes unchanged
    c, g, k = sl.law(-0.5, -0.01, 0.5)
    assert (c, g, k) == (-0.5, 0.0, 1.0)


def test_unit_record_replaces_the_bus_by_the_stage_output():
    c, g, k = sl.law(0.5, 0.0, 0.5)
    assert (c, g, k) == (0.5, 1.0, 0.0)
    assert sl.bus_gain(0.5, 0.0, 0.5) == 1.0


def test_a_pass_through_stage_sags_in_the_middle_of_the_fade():
    # g^2 + k = 0.75 at g = 0.5: a cross-fade between the bus and its own copy is not constant
    assert sl.bus_gain(0.0, 0.0, 0.5) == 0.75


def test_clip_limits_the_magnitude_symmetrically():
    assert sl.clip(0.7, 0.5) == 0.5
    assert sl.clip(-0.7, 0.5) == -0.5
    assert sl.clip(0.2, 0.5) == 0.2


def test_a_step_ramps_the_level_to_its_limit():
    assert sl.steady_gain(-0.5, 0.25, 0.5) == 1.0
    assert sl.steady_gain(0.5, -0.25, 0.5) == 1.0  # g = 0, k = 1: the bus keeps its level


def test_the_oracle_table_matches_the_law():
    rows = list(csv.DictReader(DATA.open()))
    assert len(rows) == 10
    for r in rows:
        lv, st, lim = float(r["level"]), float(r["step"]), float(r["limit"])
        assert abs(sl.steady_gain(lv, st, lim) - float(r["expected_gain"])) < 1e-6
        # the oracle prints RMS to 6 digits: measured stays within 5e-5 of the law
        assert abs(float(r["measured_gain"]) - float(r["expected_gain"])) < 5e-5, r


def test_stage_gain_never_exceeds_unity_at_the_image_limit():
    for lv in (-0.5, -0.25, 0.0, 0.25, 0.5):
        assert sl.bus_gain(lv, 0.0, 0.5) <= 1.0
