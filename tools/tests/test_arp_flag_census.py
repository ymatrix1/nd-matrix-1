"""Unit tests for tools/arp_flag_census.py (pure folding of run files)."""

from __future__ import annotations

import json

import arp_flag_census as fc


def layer(gate=0.667, notes=6):
    return {"notes_on": notes, "step_ms": 150.0, "gate_ms": 100.0, "gate_fraction": gate,
            "first_on_ms": 149.9, "step_spread_ms": 0.0}  # fmt: skip


def run(name="P", flags=1, writes=None, force=None, hits=0, on_obj=0, gate=0.667):
    return {
        "program": name, "patches": [], "force_flag": force,
        "flags": {"boot": {"1": 1}, "end": {"1": flags}},
        "flag_writes": writes or [],
        "constructor_calls": {"boot": 1, "after_boot": 0},
        "store_sites": {"0xc0000000": {"hits": hits, "on_pattern_object": on_obj}},
        "layers": {"A": layer(gate), "B": layer(None, 0)},
    }  # fmt: skip


def test_rows_have_one_row_per_layer_and_drop_the_spread():
    rows = fc.run_rows(run())
    assert [r["layer"] for r in rows] == ["A", "B"]
    assert "step_spread_ms" not in rows[0] and rows[0]["gate_fraction"] == 0.667


def test_summary_counts_writes_flags_sites_and_gates():
    s = fc.summarize([run("a", hits=3), run("b", gate=0.5), run("c", writes=[{"pc": "0x1"}])])
    assert s["runs"] == 3 and s["flag_writes"] == 1 and s["flags_not_one"] == 0
    assert s["site_hits"] == 3 and s["site_hits_on_pattern_object"] == 0
    assert s["sites_reached"] == ["0xc0000000"]
    assert s["gate_fractions"] == {"0.5": 1, "0.667": 2}


def test_a_flag_that_is_not_one_is_counted_and_force_runs_are_excluded():
    s = fc.summarize([run("a", flags=0), run("f", flags=0, force=0, gate=0.5)])
    assert s["flags_not_one"] == 1 and s["natural_runs"] == 1
    assert s["gate_fractions"] == {"0.667": 1}


def test_census_reads_files_and_main_writes_json(tmp_path):
    (tmp_path / "a.json").write_text(json.dumps(run("a")), encoding="utf-8")
    out = tmp_path / "out" / "c.json"
    out.parent.mkdir()
    assert fc.main([str(tmp_path), "--out", str(out)]) == 0
    data = json.loads(out.read_text(encoding="utf-8"))
    assert data["summary"]["runs"] == 1 and len(data["rows"]) == 2


def test_main_fails_without_run_files(tmp_path):
    assert fc.main([str(tmp_path), "--out", str(tmp_path / "x.json")]) == 1


def native_doc(on_ms, off_ms):
    ev = []
    for t0 in (150.0, 300.0, 450.0):
        ev += [{"on": True, "ms": t0}, {"on": False, "ms": t0 + on_ms}]
    return {"layers": [{"layer": "A", "events": ev}, {"layer": "B", "events": []}]}


def test_native_stats_reads_gate_and_step_of_a_layer():
    s = fc.native_stats(native_doc(100.0, 0), "A")
    assert s["step_ms"] == 150.0 and s["gate_ms"] == 100.0 and s["gate_fraction"] == 0.667


def test_census_joins_native_folders_by_file_name(tmp_path):
    (tmp_path / "runs").mkdir()
    (tmp_path / "off").mkdir()
    (tmp_path / "runs" / "a.json").write_text(json.dumps(run("a")), encoding="utf-8")
    (tmp_path / "off" / "a.json").write_text(json.dumps(native_doc(150.0, 0)), encoding="utf-8")
    data = fc.census([tmp_path / "runs" / "a.json"], {"off": tmp_path / "off", "on": tmp_path})
    row = data["rows"][0]
    assert row["native_off"]["gate_fraction"] == 1.0 and "native_on" not in row
