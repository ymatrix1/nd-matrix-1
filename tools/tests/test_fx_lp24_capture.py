"""Tests for the stateful LP24 / HP24 model, the template and the word tables (no firmware)."""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import fx_lp24_capture as cap  # noqa: E402
import fx_model_lp24 as lp  # noqa: E402


def test_golden_oracle_words_are_reproduced_over_24_calls() -> None:
    golden = json.loads(cap.GOLDEN.read_text(encoding="utf-8"))
    assert len(golden["cases"]) == len(cap.SCENARIOS)
    for case in golden["cases"]:
        scenario = tuple(case["scenario"])
        frames = cap.lcg_frames(case["seed"], cap.CALLS * cap.FRAMES, 0.4)
        got = cap.model_run(scenario, frames)
        peak = max(abs(lp.f32(w)) for w in case["oracle"])
        assert peak > 0.2, "scenario is not trivially quiet"
        for want, have in zip(case["oracle"], got, strict=True):
            assert abs(lp.f32(want) - have) < 1e-5


def test_tables_cover_the_ranges_and_start_at_the_known_words() -> None:
    t = cap.load_tables()
    assert len(t["mid_freq"]) == 128 and len(t["mid"]) == 121
    assert t["mid_freq"][0] == 0x38824000 and lp.f32(t["mid"][60][1]) == lp.f32(0x3EFD70A4)
    assert t["mid"][0] == [lp.bits(0.5), lp.bits(0.5)]


def test_type_9_words_equal_type_8_at_all_249_indices() -> None:
    t8 = cap.load_tables()
    t9 = cap.load_tables(cap.DATA / "fx-lp24-words-type9.csv")
    assert t8 == t9


def test_template_words_are_the_assumed_values_confirmed_by_the_image() -> None:
    w = cap.template_words()
    assert lp.f32(w[-25]) == 1.0 and lp.f32(w[-24]) == 0.5 and lp.f32(w[-23]) == 0.25
    assert w[-3] == 2 and lp.f32(w[-26]) == 1.0


def test_states_are_carried_and_the_pregain_settles_to_the_mid_word() -> None:
    m = cap.Lp24(0, 64, 0)
    first = m.c[-26]
    for _ in range(1500):
        m.call([(0.0, 0.0)] * cap.FRAMES)
    assert first == 1.0 and abs(m.c[-26] - m.c[-28]) < 1e-3
