"""Unit tests for tools/arm_program_morph.py (pure helpers, no firmware)."""

from __future__ import annotations

import io
import zipfile
from pathlib import Path

import arm_program_morph as pm
import pytest

PARAMS = Path(__file__).resolve().parents[2] / "docs/research/arm-edit-params.csv"


def test_event_bytes_per_source() -> None:
    assert pm.event_bytes("wheel", 127) == bytes([0xB0, 1, 127])
    assert pm.event_bytes("pedal", 64) == bytes([0xB0, 11, 64])
    assert pm.event_bytes("at", 5) == bytes([0xD0, 5])
    assert pm.event_bytes("note", 60) == bytes([0x90, 60, pm.NOTE_VELOCITY])


def test_event_bytes_rejects_out_of_range() -> None:
    with pytest.raises(ValueError):
        pm.event_bytes("wheel", 128)


def test_parse_scenario_orders_events() -> None:
    assert pm.parse_scenario("wheel:127,at:64") == [("wheel", 127), ("at", 64)]
    assert pm.parse_scenario("note:60,wheel:1") == [("note", 60), ("wheel", 1)]


@pytest.mark.parametrize("bad", ["knob:1", "wheel", "wheel:x"])
def test_parse_scenario_rejects_bad_events(bad: str) -> None:
    with pytest.raises(ValueError):
        pm.parse_scenario(bad)


def test_fw_weight_is_v_over_128_with_full_scale_at_127() -> None:
    # VERIFIED run values (morph-fw-law.csv): a +127 delta reads 127 * w.
    assert pm.fw_weight(0) == 0.0
    assert pm.fw_weight(64) == 0.5
    assert pm.fw_weight(126) == pytest.approx(126 / 128)
    assert pm.fw_weight(127) == 1.0


def test_native_weight_is_v_over_127() -> None:
    assert pm.native_weight(127) == 1.0
    assert pm.native_weight(64) == pytest.approx(64 / 127)


def test_morph_sum_clamps_the_sum_not_each_source() -> None:
    # synth B cutoff of the run: base 64, +50 wheel, +100 aftertouch, -100 pedal, all at 127.
    got = pm.morph_sum(64, (50, 100, -100), (1.0, 1.0, 1.0), 0, 127)
    assert got == 114
    assert pm.morph_sum(64, (50, 100, 0), (1.0, 1.0, 0.0), 0, 127) == 127
    assert pm.morph_sum(10, (-30, 0, 0), (1.0, 0.0, 0.0), 0, 127) == 0


def test_signed32_and_cell_rows() -> None:
    assert pm.signed32(0xFFFFFFE2) == -30
    base, deltas = pm.cell_rows([52, 10, 20, 0xFFFFFFE2])
    assert (base, deltas) == (52, (10, 20, -30))


def test_last_values_keeps_the_final_dispatch_per_id() -> None:
    log = [(1, 0x134, 52 << 16), (1, 0x134, 62 << 16), (2, 0xB7, None), (2, 0x138, 0xFFFF0000)]
    assert pm.last_values(log) == {(1, 0x134): 62.0, (2, 0x138): -1.0}


def test_rows_csv_has_header_and_rows() -> None:
    text = pm.rows_csv([("wheel:1", "0x10", 4, "synth", "0x134", "Edt_Syn_kFltFreq", "52.0781")])
    lines = text.splitlines()
    assert lines[0] == ",".join(pm.TICK_FIELDS)
    assert lines[1].startswith("wheel:1,0x10,4,synth,0x134")


def test_read_member_reads_an_archive_member(tmp_path: Path) -> None:
    path = tmp_path / "a.zip"
    with zipfile.ZipFile(path, "w") as z:
        z.writestr("Program/x.ns4p", b"abc")
    assert pm.read_member(path, "Program/x.ns4p") == b"abc"
    assert isinstance(io.StringIO(), io.StringIO)


def test_registry_helpers_find_the_morph_ids() -> None:
    names = pm.id_names(PARAMS)
    ids = pm.morph_ids(PARAMS)
    assert names[0x134] == "Edt_Syn_kFltFreq"
    assert len(ids) == 39 and 0x134 in ids and 0xB7 in ids


def test_parse_scenario_none_is_the_empty_baseline() -> None:
    assert pm.parse_scenario("none") == []


def test_changed_rows_drops_baseline_and_unchanged_values() -> None:
    rows = [
        ("none", "0x1", 4, "synth", "0x134", "n", "52.0000"),
        ("none", "0x1", 4, "synth", "0x135", "n", "0.0000"),
        ("wheel:127", "0x1", 4, "synth", "0x134", "n", "62.0000"),
        ("wheel:127", "0x1", 4, "synth", "0x135", "n", "0.0000"),
    ]
    assert pm.changed_rows(rows) == [rows[2]]
