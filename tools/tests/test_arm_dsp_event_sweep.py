"""Unit tests for tools/arm_dsp_event_sweep.py (pure classifier; the runs need local firmware)."""

from __future__ import annotations

import gzip
from pathlib import Path

import arm_dsp_event_sweep as sw

ON = 0x0B_00_E2_00  # plain key on: lane bits set
OFF = 0x08_00_E2_00
ONE = 0x3F800000  # float 1.0


def test_key_word_layer_follows_three_per_voice():
    assert sw.key_word_layer(0xC1DCC) == "A"
    assert sw.key_word_layer(0xC1DCD) == "B"
    assert sw.key_word_layer(0xC1DCE) == "C"
    assert sw.key_word_layer(0xC1DCF) == "A"  # second voice
    assert sw.key_word_layer(0xC1DCB) is None
    assert sw.key_word_layer(0xC1DCC + sw.VOICE_SPAN) is None


def test_is_on_needs_a_lane_bit():
    assert sw.is_on(ON)
    assert not sw.is_on(OFF)
    assert sw.is_on(0x01_00_00_00)
    assert not sw.is_on(0)


def test_as_float_decodes_ieee_single():
    assert sw.as_float(ONE) == 1.0
    assert sw.as_float(0) == 0.0


def test_classify_splits_voice_on_off_per_layer():
    rows = [(0, 2, 0xC1DCC, ON), (1, 2, 0xC1DCD, ON), (700, 2, 0xC1DCC, OFF)]
    res = sw.classify(rows)["layers"]
    assert res["A"]["voice_on_ms"] == [0]
    assert res["A"]["voice_off_ms"] == [700]
    assert res["B"]["voice_on_ms"] == [1]
    assert res["C"]["voice_on_ms"] == []


def test_classify_gate_keeps_only_level_changes():
    rows = [
        (0, 2, 0xC1396, ONE),
        (5, 2, 0xC1397, ONE),
        (10, 2, 0xC1396, ONE),  # repeat: no edge
        (20, 2, 0xC1396, 0),
        (30, 2, 0xC13E6, ONE),
    ]
    res = sw.classify(rows)["layers"]
    assert [e["t_ms"] for e in res["A"]["gate_edges"]] == [0, 5, 20]
    assert res["A"]["gate_edges"][0] == {"t_ms": 0, "dm": "0xc1396", "value": 1.0}
    assert res["A"]["gate_edges"][2]["value"] == 0.0
    assert [e["dm"] for e in res["B"]["gate_edges"]] == ["0xc13e6"]
    assert res["C"]["gate_edges"] == []


def test_classify_counts_other_by_layer_block_then_unassigned():
    rows = [(0, 1, 0xC1398, 1), (1, 1, 0xC1398, 2), (2, 1, 0xC13EA, 3), (3, 1, 0x93500, 4)]
    res = sw.classify(rows)
    assert res["layers"]["A"]["other"] == {"0xc1398": 2}
    assert res["layers"]["B"]["other"] == {"0xc13ea": 1}
    assert res["unassigned"] == {"0x93500": 1}


def test_entry_has_the_agreed_schema():
    e = sw.entry("Bank A/X", [(0, 2, 0xC1DCC, ON)])
    assert set(e) == {"program", "layers", "unassigned"}
    assert set(e["layers"]) == {"A", "B", "C"}
    assert set(e["layers"]["A"]) == {"voice_on_ms", "voice_off_ms", "gate_edges", "other"}


def test_period_ms_is_the_mean_interval():
    assert sw.period_ms([0, 100, 200]) == 100.0
    assert sw.period_ms([5]) is None


def test_parse_log_reads_hex_words():
    assert sw.parse_log(["12 2 0xc1dcc 0x0b00e200", ""]) == [(12, 2, 0xC1DCC, 0x0B00E200)]


def test_program_names():
    p = Path("Program/Bank A/My Prog.ns4p")
    assert sw.program_id(p) == "Bank A/My Prog"
    assert sw.out_name(p) == "Bank_A__My_Prog"


def test_order_programs_puts_listed_first_and_pending_skips_done(tmp_path):
    a, b, c = (Path(f"Program/Bank A/{n}.ns4p") for n in "abc")
    assert sw.order_programs([a, b, c], ["Bank A/c", "Bank A/b"]) == [c, b, a]
    (tmp_path / f"{sw.out_name(b)}.txt.gz").write_bytes(b"")
    assert sw.pending([a, b, c], tmp_path) == [a, c]


def test_arp_programs_reads_rows_with_arp_layers(tmp_path):
    f = tmp_path / "s.csv"
    f.write_text("program,arp_layers\nBank A/x,\nBank A/y,A\n", encoding="utf-8")
    assert sw.arp_programs(f) == ["Bank A/y"]


def test_merge_builds_sorted_entries(tmp_path):
    for name, row in (("Bank B/z", "0 2 0xc1dcc 0x0b00e200"), ("Bank A/y", "")):
        with gzip.open(tmp_path / f"{name[-1]}.txt.gz", "wt", encoding="utf-8") as fh:
            fh.write(f"# program {name}\n{row}\n")
    out = sw.merge(tmp_path)
    assert [e["program"] for e in out] == ["Bank A/y", "Bank B/z"]
    assert out[1]["layers"]["A"]["voice_on_ms"] == [0]
