"""Tests for `tools/synth_env_programs.py` (D212)."""

import csv
from pathlib import Path

import numpy as np
import synth_env_programs as P

DATA = Path(__file__).resolve().parents[2] / "docs/research/data"


def test_mean_max_error() -> None:
    m, x = P.mean_max_error(np.array([0.0, 1.0]), np.array([0.5, 1.0]))
    assert (m, x) == (0.25, 0.5)


def test_pass_to_reach_and_fall() -> None:
    y = np.array([0, 0, 0.2, 0.6, 1.0, 1.0, 0.8, 0.4, 0.1])
    assert P.pass_to_reach(y, 0, 0.5) == 3
    assert P.pass_to_reach(y, 0, 2.0) is None
    assert P.pass_to_reach(y, 0, 0.5, stop=3) is None
    assert P.pass_to_fall(y, 6) == 1  # y[5] = 1.0, half reached at the second pass (0.4)
    assert P.pass_to_fall(np.zeros(5), 3) is None


def test_sample_rows_spreads_evenly_and_skips_the_summary() -> None:
    rows = [{"program": str(i)} for i in range(100)] + [{"summary": {}}]
    s = P.sample_rows(rows, 10)
    assert len(s) == 10 and s[0]["program"] == "0" and s[1]["program"] == "10"


def test_chain_row_formats_the_error_columns() -> None:
    ora = np.concatenate([np.zeros(300), np.linspace(0, 1, 50), np.ones(50)])
    row = P.chain_row("p", 0, "amp", "legacy", (1, 2, 3), ora, ora * 0.9, 400)
    assert row["mean_abs_err"] != "0.00000" and row["variant"] == "legacy"
    assert set(row) == set(P.FIELDS)


def test_committed_programs_csv_settled_beats_legacy_on_every_layer() -> None:
    with (DATA / "synth-env-programs.csv").open(encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
    assert len(rows) == 160
    by: dict[tuple, dict] = {}
    for r in rows:
        key = (r["program"], r["layer"], r["chain"])
        by.setdefault(key, {})[r["variant"]] = float(r["mean_abs_err"])
    assert len(by) == 80
    assert all(v["settled"] <= v["legacy"] for v in by.values())
    s = P.summarize(rows)
    assert s["amp.settled"]["layers_err_above_0.01"] == 0
    assert s["amp.legacy"]["layers_err_above_0.01"] >= 20
    assert s["filter.settled"]["mean_err_max"] < 0.002
