"""Tests for `tools/synth_env_limit.py` (D212)."""

import csv
from pathlib import Path

import synth_env_limit as L

DATA = Path(__file__).resolve().parents[2] / "docs/research/data"


def test_limit_is_one_below_g_99_whatever_the_words() -> None:
    for g in (0, 60, 98):
        for a, d in ((-64, -64), (-50, -60), (0, 0)):
            assert L.amp_limit(a, d, g) == 1.0


def test_limit_staircase_of_the_fastest_words() -> None:
    assert L.amp_limit(-64, -64, 99) == 1.2 and L.amp_limit(-64, -64, 100) == 1.2
    assert L.amp_limit(-64, -64, 107) == 2.0 and L.amp_limit(-64, -64, 108) == 2.0
    assert L.amp_limit(-64, -64, 126) == 3.8
    assert L.amp_limit(-64, -64, 127) == 4.0, "g = 127 acts as 128"


def test_limit_falls_with_slower_words_and_vanishes_for_normal_programs() -> None:
    assert L.amp_limit(-64, -60, 127) == 3.2 and L.amp_limit(-60, -64, 127) == 3.2
    assert L.amp_limit(-64, 63, 127) == 1.0, "Hold decay"
    assert L.amp_limit(-20, -20, 127) == 1.0
    assert L.amp_limit(0, -64, 127) == 1.0


def test_limit_uses_the_velocity_scaled_attack_index() -> None:
    # g = 110: attack -63 scales to -55 like -64 (floor(w * 110 / 128)), so the limits agree.
    assert L.amp_limit(-63, -64, 110) == L.amp_limit(-64, -64, 110) == 2.2
    assert L.amp_limit(-62, -64, 110) == 2.0


def test_committed_oracle_grid_matches_the_law_on_every_row() -> None:
    with (DATA / "synth-env-amp-limit.csv").open(encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
    assert len(rows) >= 500
    for r in rows:
        want = L.amp_limit(int(r["attack"]), int(r["decay"]), int(r["g"]))
        assert abs(want - float(r["limit"])) < 1e-4, r
    assert max(float(r["limit"]) for r in rows) == 4.0
    assert {int(r["key"]) for r in rows} == {36, 60, 84}


def test_effective_g_maps_127_to_128() -> None:
    assert (L.effective_g(126), L.effective_g(127), L.effective_g(5)) == (126, 128, 5)
