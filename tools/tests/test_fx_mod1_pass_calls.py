"""Tests of tools/fx_mod1_pass_calls.py: output parsing, stage window, merged scripts."""

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import fx_mod1_pass_calls as pc  # noqa: E402


def test_parse_calls_reads_the_calls_at_line():
    text = "output L peak 0.0\ncalls of 0x125346 in one pass: 35\n"
    assert pc.parse_calls(text) == 35


def test_parse_calls_rejects_text_without_the_line():
    with pytest.raises(ValueError):
        pc.parse_calls("no count here")


def test_stage_window_covers_head_record_group_headers_and_rows_only():
    assert pc.in_stage_window(0xB62CE) and pc.in_stage_window(0xB63EE)
    assert pc.in_stage_window(0xB6506)
    assert not pc.in_stage_window(0xB62BF) and not pc.in_stage_window(0xB6520)
    assert not pc.in_stage_window(0xC1AB9)  # the Amp unit slots are outside


def test_script_hits_counts_only_window_addresses():
    lines = ["0 0xb63ee 0x1", "0 0xc1ab9 0x2", "3 0xb62ce 0x3", "bad"]
    assert pc.script_hits(lines) == 2


def _row(core, dm, w2, chip="FX"):
    return {"chip": chip, "core": core, "dm": dm, "w2": w2}


def test_capture_hits_filters_chip_and_core():
    rows = [
        _row(1, 0xB63EE, 1),
        _row(2, 0xB63EE, 1),
        _row(1, 0xC1AB9, 1),
        _row(1, 0xB63EE, 1, "PNO"),
    ]
    assert pc.capture_hits(rows) == 1
    assert pc.capture_hits(rows, core=2) == 1


def test_merged_script_appends_core_messages_at_pass_zero():
    rows = [_row(1, 0xC1AB9, 0x1243F4), _row(2, 0xC1A19, 0x7)]
    text = pc.merged_script(["0 0x10 0x1"], rows)
    assert text == "0 0x10 0x1\n0 0xc1ab9 0x001243f4\n"


def test_frames_processed_counts_deviating_positions():
    assert pc.frames_processed([0.3, 0.3, 0.0, 0.0]) == 2
    assert pc.frames_processed([0.3, 0.3, 0.3, 0.3]) == 4


def test_mod1_pcs_are_core2_pcs_shifted_by_the_core_offset():
    core2 = {"A-PAN": 0x122D93, "TREM": 0x122C39, "RM": 0x1239D0, "A-WAH": 0x123E9B,
             "WAH": 0x123DD2, "PUMP": 0x122D2C}  # fmt: skip
    assert {k: v + 0x974 for k, v in core2.items()} == pc.MOD1_PCS
