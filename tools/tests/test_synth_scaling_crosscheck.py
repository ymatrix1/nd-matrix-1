"""Tests for `tools/synth_scaling_crosscheck.py` (D212)."""

import synth_scaling_crosscheck as X


def test_published_forms_hit_their_documented_ends() -> None:
    assert abs(X.attack_seconds(0) - 0.0005) < 1e-12
    assert 44 < X.attack_seconds(127) < 46
    assert abs(X.decay_seconds(0) - 0.003) < 1e-12
    assert (X.cutoff_index(52), X.cutoff_index(117), X.cutoff_index(127)) == (0, 65, 106)
    assert X.resonance_k(0) == 2.0 and X.resonance_k(128) == 0.0


def test_settled_tables_agree_with_the_published_times_within_one_percent() -> None:
    s, o = X.load("synth-env-rates-settled.csv"), X.load("synth-env-rate-table.csv")
    for stats in (X.attack_check(s), X.decay_check(s, "decay_coeff")):
        assert stats[0] < 0.01
    assert X.attack_check(o)[0] > 1.0, "the stale sweep is off by more than 100 %"
    assert X.decay_check(o, "decay_release_coeff")[0] > 0.5


def test_cutoff_index_law_is_exact() -> None:
    assert X.cutoff_max_error() < 1e-5


def test_resonance_table_is_the_quadratic_only_below_index_38() -> None:
    b = [float(r["B_0xB1015"]) for r in X.load("synth-filter-tables.csv")]
    n, err = X.resonance_agreement(b)
    assert n == 38 and err < 1e-6
    assert b[64] > 1.4 * X.resonance_k(64), "0.7071 against 0.5 at index 64"


def test_log_ratio_stats() -> None:
    m, k, med = X.log_ratio_stats([1.0, 2.0, 3.3], [1.0, 2.0, 3.0])
    assert k == 2 and abs(m - 0.1) < 1e-9 and med == 0.0
