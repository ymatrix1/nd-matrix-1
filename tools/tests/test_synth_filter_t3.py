"""Tests for the stage `0x1257B2` model (no firmware needed)."""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import synth_filter_t3 as t3  # noqa: E402
import synth_filter_t3_golden as gold  # noqa: E402

CASES = Path(__file__).resolve().parents[2] / "docs/research/data/synth-filter-t3-oracle-cases.json"


def test_recorded_oracle_cases_are_reproduced_word_for_word() -> None:
    cases = json.loads(CASES.read_text(encoding="utf-8"))["cases"]
    assert len(cases) == 6
    for case in cases:
        assert t3.model_run(case) == case["oracle"]


def test_random_case_is_deterministic_per_seed() -> None:
    assert t3.random_case(3) == t3.random_case(3)
    assert t3.random_case(3) != t3.random_case(4)


def test_tables_have_the_documented_size_and_ends() -> None:
    assert [len(c) for c in t3.TABLES] == [129] * 3
    assert t3.s.ld(t3.TABLES[1][0]) == 0
    assert t3.s.ld(t3.TABLES[2][0]) == 2


def test_split_takes_the_signed_top_byte_and_the_fraction() -> None:
    idx, fr = t3.split(t3.s.ld(0x3E800000))  # 0.25 -> 0x2000_0000
    assert (idx, fr) == (32, 0)
    idx, fr = t3.split(t3.s.ld(0x3E000000))  # 0.125 -> index 16
    assert idx == 16


def test_cutoff_zero_prologue_gives_the_start_of_the_tables() -> None:
    words, _ = t3.call_coefficients(0, 0)
    assert t3.s.ld(words["g"]) == 0
    assert t3.s.ld(words["v"]) == 2
    assert t3.s.ld(words["kc"]) == t3.QUARTER  # 4.5 * 0 + 0.25


def test_silence_stays_silent() -> None:
    ln = t3.Lane()
    out, _ = t3.process(ln, 0x3E99999A, 0x40000000, [0, 0, 0, 0])
    assert out == [0, 0, 0, 0]


def test_generators_print_the_rust_files() -> None:
    assert "pub(super) const T0: [u32; 129]" in gold.tables()
    golden = gold.golden()
    assert f"pub(super) const COEFS: [u32; {6 * len(gold.ROWS)}]" in golden
    assert f"pub(super) const OUTPUT: [u32; {gold.SAMPLES * len(gold.ROWS)}]" in golden
