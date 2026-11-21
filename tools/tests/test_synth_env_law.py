"""Tests for `tools/synth_env_law.py` (D205)."""

import csv
from pathlib import Path

import synth_env_law as L
import synth_env_trace as t

DATA = Path(__file__).resolve().parents[2] / "docs/research/data"


def test_parse_cases_reads_triples_split_by_space_or_semicolon() -> None:
    assert L.parse_cases("-64:-20:-20 3:0:5;7:8:9") == [(-64, -20, -20), (3, 0, 5), (7, 8, 9)]


def test_case_words_repeat_the_triple_in_all_three_chains() -> None:
    w = L.case_words(-5, 7, 9)
    for chain in range(3):
        base = t.ADR_BASE + 3 * chain
        assert w[base] == (-5 << 24) & 0xFFFFFFFF
        assert w[base + 1] == 7 << 24 and w[base + 2] == 9 << 24
    assert w[t.COMMON_CUTOFF] == t.cutoff_word(100)


def test_scaled_index_laws() -> None:
    assert L.scaled_index(-64, 10, "g128-floor") == -5
    assert L.scaled_index(-2, 10, "g128-floor") == -1
    assert L.scaled_index(-40, 127, "g128-floor") == -40
    assert L.scaled_index(30, 10, "g128-floor") == 30
    assert L.scaled_index(-60, 64, "g127-round") == -30
    assert L.scaled_index(-2, 10, "g127-trunc") == 0
    assert L.scaled_index(-2, 10, "g127-ceil") == 0
    assert L.scaled_index(-2, 10, "g127-floor") == -1


def test_fit_laws_picks_the_floor_law_on_the_committed_oracle_rows() -> None:
    """Amp rows from `synth-env-law-attack.csv`; each selected index is a filter reference row."""
    rows: list[dict] = []
    seen: set[int] = set()
    with (DATA / "synth-env-law-attack.csv").open(encoding="utf-8") as f:
        for r in csv.DictReader(f):
            g, w, idx = int(r["g"]), int(r["word_index"]), int(r["amp_index"])
            slope = r["amp_attack_slope"]
            rows.append({"chain": "amp", "g": g, "attack": w, "attack_slope": slope})
            if idx not in seen:
                seen.add(idx)
                rows.append({"chain": "filter", "g": 127, "attack": idx, "attack_slope": slope})
    fit = L.fit_laws(rows)
    hits, total = fit["g128-floor"]
    assert hits == total >= 250
    assert fit["g127-round"][0] < hits
    assert fit["g127-trunc"][0] < fit["g127-round"][0]
