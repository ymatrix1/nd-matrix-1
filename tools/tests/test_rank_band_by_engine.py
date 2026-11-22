"""Unit tests for ``tools/rank_band_by_engine.py``."""

from __future__ import annotations

import math

import rank_band_by_engine as m


def row(on: dict[str, bool], d: float, f: float) -> dict:
    return {
        "scene_1": on,
        "scene_2": {},
        "default": {"bands_db": [d] * 8},
        "firmware": {"bands_db": [f] * 8},
    }


def test_engine_class_joins_enabled_sections() -> None:
    assert (
        m.engine_class(row({"piano": True, "synth": True, "organ": False}, 0, 0)) == "piano+synth"
    )
    assert m.engine_class(row({"piano": False}, 0, 0)) == "none"


def test_share_db_adds_power() -> None:
    assert math.isclose(m.share_db([-10.0, -10.0]), -10.0 + 10 * math.log10(2), rel_tol=1e-9)


def test_summary_reads_the_treble_difference() -> None:
    rows = [row({"piano": True}, -10.0, -10.0), row({"piano": True}, -7.0, -10.0)]
    groups = m.group_rows(rows)
    s = m.summarize(groups["piano"])
    assert s["n"] == 2 and s["over_1db"] == 1
    assert math.isclose(s["treble_worst_db"], 3.0, abs_tol=1e-9)
    assert math.isclose(s["treble_mean_db"], 1.5, abs_tol=1e-9)


def test_rows_without_both_renders_are_skipped() -> None:
    bad = {
        "scene_1": {"piano": True},
        "default": {"panic": "x"},
        "firmware": {"bands_db": [0.0] * 8},
    }
    assert m.group_rows([bad]) == {}
