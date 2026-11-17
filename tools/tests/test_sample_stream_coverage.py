"""Unit tests for tools/sample_stream_coverage.py (classification of captures)."""

from __future__ import annotations

import sample_stream_coverage as c


def row(phase: str, dm: int, w2: int) -> dict[str, object]:
    return {"phase": phase, "chip": "SY", "path": 1, "dm": dm, "w2": w2}


def test_layer_count_and_prepared_phases() -> None:
    assert c.layer_count({"synth_a": "x", "synth_b": None, "synth_c": "z"}) == 2
    rows = [row("k84", 0x92414, 0x1C07CD), row("k36", 0x92414, 0x1C0794), row("k60", 0x92416, 1)]
    assert c.prepared_phases(rows) == ["k84"]
    assert c.prepared_phases([{**row("k36", 0x92414, 0x1C07CD), "chip": "SMP"}]) == []


def test_classify_each_blocker() -> None:
    one = {"synth_a": "m"}
    assert c.classify({"synth_a": None}, ["k60"]) == "no sample member resolved"
    assert c.classify(one, None) == "no capture"
    assert c.classify(one, ["k36", "k60"]) == "voices at key 60"
    assert c.classify(one, ["k36"]) == "voices only at other keys"
    assert c.classify(one, []) == "no voice queued"


def test_summarize_counts() -> None:
    rows = [{"class": "a", "layers": 1}, {"class": "a", "layers": 2}, {"class": "b", "layers": 1}]
    s = c.summarize(rows)
    assert s["programs"] == 3 and s["classes"] == {"a": 2, "b": 1} and s["layers"] == {1: 2, 2: 1}
