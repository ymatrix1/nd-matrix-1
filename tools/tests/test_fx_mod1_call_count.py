"""Tests of tools/fx_mod1_call_count.py: script parsing, block-count lookup, frame deviations."""

import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import fx_mod1_call_count as cc  # noqa: E402


def test_read_script_keeps_the_last_value_per_address(tmp_path):
    p = tmp_path / "s.script"
    p.write_text("0 0x10 0x1\n0 0x11 0x2\n3 0x10 0x5\n")
    assert cc.read_script(p) == {0x10: 5, 0x11: 2}


def test_block_counts_follow_the_row_and_the_kind_script(tmp_path):
    csv = tmp_path / "words.csv"
    csv.write_text(
        "slot,type,list,entry,ctx,dm_word,word,kinds\n"
        "Fx1,4,0x0,0,-32,0x1000,0x0,synth\n"
        "Fx1,4,0x0,0,-32,0x2000,0x0,piano\n"
        "Fx1,4,0x0,0,-32,0x3000,0x0,organ\n"
        "Rvb,4,0x0,0,-32,0x4000,0x0,piano\n"
    )
    (tmp_path / "fx_core1.script").write_text(f"0 {0x1020 - 16:#x} 0x2\n0 {0x2020 - 16:#x} 0x1\n")
    (tmp_path / "pno_core1.script").write_text(f"0 {0x3020 - 16:#x} 0x1\n")
    got = cc.block_counts(csv, tmp_path)
    assert got[("RM", "synth")] == [(0x1020, 2)]
    assert got[("RM", "piano")] == [(0x2020, 1)]
    assert got[("RM", "organ")] == [(0x3020, 1)]


def test_deviation_by_position_is_zero_where_the_pass_through_holds():
    x = np.arange(1, 41, dtype=np.float64)
    y = cc.CHAIN_GAIN * x
    y[0::4] *= 2.0  # only frame position 0 is processed
    dev = cc.deviation_by_position(y, x)
    assert dev[0] > 0.9 and dev[1] == dev[2] == dev[3] == 0.0
