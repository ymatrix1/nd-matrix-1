"""Tests for `tools/synth_env_rates.py` (D212)."""

import csv
from pathlib import Path

import numpy as np
import pytest
import synth_env_rates as R
import synth_env_trace as t

DATA = Path(__file__).resolve().parents[2] / "docs/research/data"


def rows(name: str) -> list[dict]:
    with (DATA / name).open(encoding="utf-8") as f:
        return list(csv.DictReader(f))


def test_index_words_put_the_index_in_the_top_byte_of_all_nine_words() -> None:
    w = R.index_words(-40)
    assert sorted(w) == [t.ADR_BASE + i for i in range(9)]
    assert set(w.values()) == {(-40 << 24) & 0xFFFFFFFF}


def test_settled_row_accepts_a_constant_tail_and_rejects_a_moving_one() -> None:
    tr = np.array([[1, 2]] * 3 + [[5, 6]] * 40, dtype=np.uint32)
    assert list(R.settled_row(tr, 32)) == [5, 6]
    with pytest.raises(ValueError, match="still moving"):
        R.settled_row(tr, 41)


def test_check_chains_requires_equal_coefficients_across_the_three_chains() -> None:
    R.check_chains(np.array([1, 2, 3, 1, 2, 3, 1, 2, 3], dtype=np.uint32))
    with pytest.raises(ValueError, match="disagree"):
        R.check_chains(np.array([1, 2, 3, 9, 2, 3, 1, 2, 3], dtype=np.uint32))


def test_f32_bits_roundtrip() -> None:
    assert R.f32_to_bits(1.0) == 0x3F800000


def test_bucket_is_a_sixteen_wide_range() -> None:
    got = [R.bucket(i) for i in (-64, -49, -48, -1, 0, 15, 16, 63, 64)]
    assert got == [-64, -64, -48, -16, 0, 0, 16, 48, 64]


def test_committed_settled_table_is_complete_and_decay_equals_release() -> None:
    s = rows("synth-env-rates-settled.csv")
    assert [int(r["index"]) for r in s] == list(range(-64, 65))
    assert all(r["decay_coeff"] == r["release_coeff"] for r in s)
    assert all(int(r["attack_bits"], 16) == R.f32_to_bits(float(r["attack_coeff"])) for r in s)


def test_settled_attack_equals_the_d210_attack_table_to_six_digits() -> None:
    new = {int(r["index"]): float(r["attack_coeff"]) for r in rows("synth-env-rates-settled.csv")}
    for r in rows("synth-env-attack-table.csv"):
        assert abs(new[int(r["index"])] / float(r["attack_coeff"]) - 1) < 5e-3, r["index"]


def test_stale_sweep_is_off_at_104_indexes_for_both_tables() -> None:
    new, old = rows("synth-env-rates-settled.csv"), rows("synth-env-rate-table.csv")
    assert len(R.stale_indexes(new, old, False)) == 104
    assert len(R.stale_indexes(new, old, True)) == 104
    assert R.rate_ratio(0.9, 0.8, True) == pytest.approx(0.5)
    assert R.rate_ratio(0.5, 0.25, False) == 2.0


def test_stale_sweep_matches_at_the_plateau_anchors() -> None:
    new, old = rows("synth-env-rates-settled.csv"), rows("synth-env-rate-table.csv")
    off = R.stale_indexes(new, old, False)
    assert {-64, -58, -53} & off == set()


def test_census_counts_layers_ranges_and_stale_entries() -> None:
    def env(a: int, d: int, r: int) -> dict:
        return {"attack": a, "decay": d, "release": r}

    rows_ = [
        {"program": "p", "layer": 0, "amp": env(-64, 63, -11), "filter": env(-1, 52, 22)},
        {"program": "p", "layer": 1, "amp": env(-64, 63, -11), "filter": env(-1, 52, 22)},
        {"summary": {}},
    ]
    res = R.census(rows_, {-1}, {63})
    assert (res["layers"], res["programs"], res["entries"]) == (2, 1, 12)
    assert res["ranges"]["amp.attack"] == {-64: 2}
    assert res["ranges"]["filter.decay"] == {48: 2}
    assert res["stale_entries"] == 4 and res["stale_layers"] == 2 and res["stale_programs"] == 1


def test_plateaus_counts_repeated_values() -> None:
    assert R.plateaus([1.0, 1.0, 2.0, 2.0, 2.0, 3.0]) == 3


def test_rust_words_format_eight_per_line() -> None:
    text = R.rust_words("X", list(range(9)))
    assert text.startswith("pub(super) const X: [u32; 9] = [")
    assert text.count(chr(10)) == 4 and text.endswith("];" + chr(10))


def test_settled_decay_matches_the_release_ratio_the_chain_plays() -> None:
    """Chain tail ratio `y[n+1] / y[n]` of the committed velocity sweep against the table."""
    new = {int(r["index"]): float(r["decay_coeff"]) for r in rows("synth-env-rates-settled.csv")}
    seen: dict[int, float] = {}
    for r in rows("synth-env-velocity-sweep.csv"):
        if r["chain"] == "filter" and r["release_ratio"]:
            seen.setdefault(int(r["word_index"]), float(r["release_ratio"]))
    assert len(seen) == 6
    for w, ratio in seen.items():
        assert abs((1 - ratio) / (1 - new[w]) - 1) < 0.01, w
    stale = {
        int(r["index"]): float(r["decay_release_coeff"]) for r in rows("synth-env-rate-table.csv")
    }
    assert max(abs((1 - ratio) / (1 - stale[w]) - 1) for w, ratio in seen.items()) > 0.3
