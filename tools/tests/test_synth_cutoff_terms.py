"""Unit tests for tools/synth_cutoff_terms.py (term models against the oracle CSV)."""

from __future__ import annotations

import csv
from pathlib import Path

import pytest
import synth_cutoff_terms as c
import synth_env_trace as t

CSV_PATH = Path(__file__).resolve().parents[2] / "docs/research/data/synth-cutoff-terms-oracle.csv"


def rows(kind: str) -> list[dict]:
    with CSV_PATH.open(encoding="utf-8") as f:
        return [r for r in csv.DictReader(f) if r["kind"] == kind]


def field(inputs: str, key: str) -> str:
    return dict(p.split("=") for p in inputs.split(" "))[key]


def test_lfo_depth_word_is_the_arm_law() -> None:
    assert c.lfo_depth_word(0) == 0
    assert c.lfo_depth_word(127) == 0x40000000
    assert abs(c.lfo_depth_word(32) - 0x10204080) <= 4  # ARM float path: 24 bit mantissa
    assert abs(c.lfo_depth_word(64) - 0x20408100) <= 4
    assert c.lfo_depth_word(500) == 0x40000000


def test_q31_is_signed() -> None:
    assert c.q31(0x40000000) == 0.5
    assert c.q31(0xC0000000) == -0.5
    assert c.q31(0) == 0.0


def test_lfo_rows_match_512_depth_lfo() -> None:
    data = rows("lfo")
    assert len(data) == 12
    for r in data:
        amt, lfo = int(field(r["inputs"], "amt")), int(field(r["inputs"], "lfo"), 0)
        model = c.lfo_semitones(c.lfo_depth_word(amt), lfo)
        assert model == pytest.approx(float(r["oracle"]), abs=0.02), r["inputs"]
        assert float(r["error"]) == pytest.approx(float(r["oracle"]) - float(r["model"]), abs=1e-3)


def test_env_rows_match_128_w7_f4() -> None:
    data = rows("env")
    assert len(data) == 6
    for r in data:
        w7_part, f4_part = r["inputs"].split(" F-4=")
        w7 = int(w7_part.split("=")[1], 0)
        assert c.env_semitones(float(f4_part), w7) == pytest.approx(float(r["oracle"]), abs=0.02)
    assert c.env_semitones(1.0) == pytest.approx(127.0)


def test_key_offset_and_drive_rows() -> None:
    for r in rows("m0"):
        assert float(r["oracle"]) == pytest.approx(int(field(r["inputs"], "m0")), abs=0.01)
    assert all(abs(float(r["oracle"])) < 1e-3 for r in rows("drive"))


def test_r_law_rows_split_by_stage() -> None:
    n = 0
    for r in rows("r_law"):
        knob = float(field(r["inputs"], "knob"))
        model = c.ramp_r_of_knob(knob) if r["stage"] == "ladder" else c.exp_r_of_knob(knob)
        assert model == pytest.approx(float(r["oracle"]), rel=0.006), r["inputs"]
        n += 1
    assert n == 8
    for r in rows("r_law_key"):
        knob = float(r["inputs"].split(" ")[0].split("=")[1])
        assert c.ramp_r_of_knob(knob - 12) == pytest.approx(float(r["oracle"]), rel=0.006)


def test_knob_inverses_round_trip() -> None:
    for knob in (0.0, 30.0, 81.0, 110.0):
        assert c.knob_of_exp_r(c.exp_r_of_knob(knob)) == pytest.approx(knob, abs=1e-3)
    for knob in (30.0, 52.0, 81.0, 100.0):
        assert c.knob_of_ramp_r(c.ramp_r_of_knob(knob)) == pytest.approx(knob, abs=1e-3)
    # the ladder tap of knob 81 is the ramp 0.5908, the exponential law gives 0.0536
    assert c.ramp_r_of_knob(81) == pytest.approx(0.5908, abs=1e-3)
    assert c.exp_r_of_knob(81) == pytest.approx(0.0536, abs=1e-3)


def test_every_pass_writes_before_each_pass() -> None:
    s = c.every_pass([(0xC2717, 0x0C000000), (0xC1E37, 5)], 2)
    assert s.splitlines() == [
        "0 0xc2717 0x0c000000",
        "0 0xc1e37 0x00000005",
        "1 0xc2717 0x0c000000",
        "1 0xc1e37 0x00000005",
    ]


def test_row_helper_rounds_and_reports_the_error() -> None:
    r = c.row("lfo", "p", "s", "i", 1.23456, 1.2)
    assert (r["oracle"], r["model"], r["error"]) == (1.2346, 1.2, 0.0346)
    assert set(r) == set(c.CSV_FIELDS)


def test_cutoff_word_matches_the_trace_tool() -> None:
    assert t.cutoff_word(81) == 29 << 24
