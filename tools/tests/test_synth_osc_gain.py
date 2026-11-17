"""Unit tests for tools/synth_osc_gain.py (variant arguments, needed row gain, CSV rows)."""

from __future__ import annotations

import math

import pytest
import synth_osc_gain as g


def test_row_arg_sets_gain_one_and_the_shape_scale_by_family() -> None:
    assert g.row_arg("saw") == "table:saw=1/0/1/0.5/1"
    assert g.row_arg("shape") == "table:shape=1/0/1/1.0/1"
    assert g.row_arg("shapesq").endswith("/1.0/1")
    assert g.row_arg("noise").endswith("/0.5/1")


def test_variants_are_filter_off_and_differ_in_amp_table_and_unison() -> None:
    d = g.variant_args("default", "saw")
    assert d == ["--no-filter"]
    u = g.variant_args("unity", "saw")
    assert u == ["--no-filter", "--unity"]
    v = g.variant_args("verified", "saw")
    assert v[-3:] == ["--synth-stages", "--synth-p", "verified"]
    one = g.variant_args("g1", "pulse")
    assert "--unity" in one and one[-1] == g.row_arg("pulse") and "--unison" not in one
    u0 = g.variant_args("g1u0", "pulse")
    assert u0[-2:] == ["--unison", "0"]
    with pytest.raises(ValueError):
        g.variant_args("nope", "saw")


def test_needed_row_gain_is_the_linear_level_ratio() -> None:
    assert g.needed_row_gain(-10.0, -10.0) == pytest.approx(1.0)
    assert g.needed_row_gain(-10.0, -16.0206) == pytest.approx(2.0, rel=1e-4)
    assert g.needed_row_gain(-20.0, -10.0) == pytest.approx(10**-0.5)


def _report() -> dict:
    d = {"default": 8.0, "unity": 8.0, "verified": 8.0, "g1": 0.05, "g1u0": 0.05}
    return {
        "program": "Lush 12db Pad",
        "p_wave": "saw",
        "table_gain": 0.4,
        "oracle_rms_db": -16.91,
        "delta_db": d,
        "band_delta_db": {k: 3.1 for k in d},
        "f0_oracle_hz": 261.6,
        "f0_native_hz": 260.2,
        "needed_row_gain": 1.006,
        "needed_row_gain_u0": 1.006,
        "unison": 0,
    }


def test_csv_row_flattens_a_report() -> None:
    r = g.csv_row(_report())
    assert set(r) == set(g.CSV_FIELDS)
    assert r["default_delta_db"] == 8.0 and r["g1_delta_db"] == 0.05
    assert r["needed_row_gain"] == 1.006 and r["table_gain"] == 0.4
    assert math.isclose(r["oracle_rms_db"], -16.91)


def test_csv_row_accepts_a_missing_table_gain_and_old_reports() -> None:
    rep = _report()
    rep["table_gain"] = None
    del rep["needed_row_gain_u0"], rep["unison"]
    r = g.csv_row(rep)
    assert r["table_gain"] == "" and r["needed_row_gain_u0"] == 1.006


def test_voice_constant_is_the_p16_gain_of_a_single_oscillator_stage() -> None:
    assert g.VOICE_CONST == 0.25
