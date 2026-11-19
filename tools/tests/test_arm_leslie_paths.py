"""Unit tests for tools/arm_leslie_paths.py (literal scan, edit plan, census)."""

from __future__ import annotations

import arm_emu as ae
import arm_leslie_paths as lp
from arm_edit_capture import Param


def msg(word1: int, word2: int, path: int = 2, comm: int = 1) -> ae.Message:
    return ae.Message(path, comm, word1, word2, 0xC0000000)


def test_literal_forms_cover_address_and_code_record() -> None:
    forms = lp.literal_forms(0x1256AD)
    assert forms[0x1256AD] == "address"
    assert forms[0x28000000 + 4 * 0x1256AD] == "code_record"


def test_scan_message_hits_value_and_record_word() -> None:
    assert lp.scan_message(msg(0x28000000 + 4 * 0xB0471, 0x1256AD)) == [
        ("word2", "leslie", "address")
    ]
    record = lp.scan_message(msg(0x28000100, 0x28000000 + 4 * 0x1257B2))
    assert record == [("word2", "stage_1257b2", "code_record")]
    assert lp.scan_message(msg(0x28000100, 0x1234)) == []


def test_scan_words_ignores_neighbours() -> None:
    assert lp.scan_words([("a", 0x1256AC), ("b", 0x1256AE)]) == []


def test_l2_code_store_only_for_path2_window() -> None:
    assert lp.is_l2_code_store(msg(0x20000010, 1))
    assert not lp.is_l2_code_store(msg(0x28000010, 1))
    assert not lp.is_l2_code_store(msg(0x20000010, 1, path=1))


def test_census_names_chip_core_path() -> None:
    counts = lp.census([msg(0x28000000, 1), msg(0x28800000, 1), msg(0x28800004, 2)])
    assert counts == {"PNO/c1/p2": 1, "PNO/c2/p2": 2}


def test_value_set_small_and_large_ranges() -> None:
    assert lp.value_set(0, 3) == (0, 1, 2, 3)
    assert lp.value_set(0, 127) == (0, 31, 63, 96, 127)


def test_edit_plan_groups() -> None:
    params = [
        Param(0x189, "Edt_OrgCom_kEffectsLeslieEnable", 0, 1),
        Param(0x4E, "Edt_Sys_kAudioRoutingDstLeslie", 0, 8),
        Param(0x1A7, "Edt_Effects_kAmpTypeNew", 0, 9),
        Param(0x198, "Edt_Effects_kFx1Type", 0, 11),
        Param(0x99, "Edt_Org_kDrawbar", 0, 8),
        Param(0x6, "Fnc_kRotorSpeed", 0, 5),
    ]
    plan = {e.name: e.group for e in lp.edit_plan(params)}
    assert plan == {
        "Edt_OrgCom_kEffectsLeslieEnable": "leslie",
        "Edt_Sys_kAudioRoutingDstLeslie": "routing",
        "Edt_Effects_kAmpTypeNew": "type",
        "Edt_Effects_kFx1Type": "type",
        "Fnc_kRotorSpeed": "leslie",
    }


def test_summary_counts_hits() -> None:
    from collections import Counter

    out = lp.summary(
        [{"a": 1}], {"load": Counter({"PNO/c1/p2": 3})}, Counter({"load": 1}), Counter()
    )
    assert out["hit_count"] == 1
    assert out["messages"]["load"] == {"PNO/c1/p2": 3}


def test_scan_rows_counts_literals_and_controls_by_phase_group() -> None:
    rows = [
        {"phase": "load", "w2": 0x1256AD},
        {"phase": "edit:Edt_X=1@fx3", "w2": 0x121E2B},
        {"phase": "edit:Edt_X=2@fx3", "w2": 0x121E2B},
        {"phase": "idle", "w2": 0x28000000 + 4 * 0x1257B2},
        {"phase": "idle", "w2": 0x42},
    ]
    hits = lp.scan_rows(rows)
    assert hits[("leslie", "address", "load")] == 1
    assert hits[("b3_processor", "address", "edit")] == 2
    assert hits[("stage_1257b2", "code_record", "idle")] == 1
    assert sum(hits.values()) == 4
