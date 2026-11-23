"""Unit tests for tools/synth_env_level_link.py (velocity words, factor law, row reduction)."""

from __future__ import annotations

import csv
from pathlib import Path

import numpy as np
import synth_env_level_link as ll
import synth_env_trace as t

LINK = Path(__file__).resolve().parent.parent.parent / "docs/research/data/synth-env-level-link.csv"


def test_velocity_words_are_the_arm_pair() -> None:
    on = ll.velocity_words(True)
    assert on == {ll.STRIP0_W19: 0, ll.STRIP6_W19: 0x3F800000}
    off = ll.velocity_words(False)
    assert off == {ll.STRIP0_W19: 0x3F800000, ll.STRIP6_W19: 0}
    assert ll.STRIP6_W19 == 0xE0187


def test_effective_key_byte_saturates_at_128() -> None:
    assert [ll.effective_key_byte(g) for g in (0, 10, 126, 127)] == [0, 10, 126, 128]


def test_predicted_factor_is_g_over_128_only_with_the_setting_on() -> None:
    assert ll.predicted_factor(64, True) == 0.5
    assert ll.predicted_factor(127, True) == 1.0
    assert ll.predicted_factor(10, False) == 1.0


def test_row_reduces_a_synthetic_run_to_peak_semitones() -> None:
    on, off = 5, 15
    n = 20
    k = np.full(n, 57.0)
    k[on:off] = 88.0
    r = np.array([_r_of_knob(x) for x in k])
    env = np.zeros((n, 1))
    env[on:off, 0] = 0.5
    extra = np.zeros((n, 2))
    extra[on:off, 0] = 0.24
    extra[off - 1, 1] = 0.75
    got = ll.row(True, 64, env, extra, r, on, off)
    assert got["vel_setting"] == 1 and got["g"] == 64
    assert abs(got["semitones_peak"] - 31.0) < 0.05
    assert got["chain_peak"] == 0.5 and got["f4_peak"] == 0.24 and got["slot_f"] == 0.75


def _r_of_knob(knob: float) -> float:
    hz = t.KNOB_HZ0 * 2.0 ** (knob / 12.0)
    return float(np.tan(np.pi * hz / t.ORACLE_STAGE_RATE_HZ))


def test_committed_oracle_rows_follow_the_engine_peak_factor() -> None:
    """Oracle peak semitones equal `127 amount * G / 128` with the setting on, `127 amount` off."""
    rows = list(csv.DictReader(LINK.read_text(encoding="utf-8").splitlines()))
    assert len(rows) >= 16
    for r in rows:
        full = float(r["amount_knob"])
        want = full * ll.predicted_factor(int(r["g"]), bool(int(r["vel_setting"])))
        assert abs(float(r["semitones_peak"]) - want) < 0.05, r
        assert float(r["chain_peak"]) > 0.999
