"""Tests of `program_rank_combo`: combination keys, best sets, greedy chain, flip tables."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import program_rank_combo as pc  # noqa: E402


def render(rms=-24.0, centroid=1000.0, bands=None) -> dict:
    return {
        "rms_db": rms,
        "peak_db": rms + 12.0,
        "centroid_hz": centroid,
        "bands_db": bands or [-90.0, -40.0, -10.0, -8.0, -12.0, -20.0, -40.0, -90.0],
    }


def shifted(db: float) -> dict:
    """A render `db` louder than the default one (score `db`: no band or centroid change)."""
    return render(rms=-24.0 + db)


def row(full: float, ablate: dict[str, float], name: str = "Program/Bank A/X.ns4p") -> dict:
    return {
        "entry": name,
        "default": render(),
        "firmware": shifted(full),
        "ablate_fx": {k: shifted(v) for k, v in ablate.items()},
    }


def test_key_of_orders_groups_like_program_rank():
    assert pc.key_of(["fx_mod2", "fx_delay"]) == "fx_delay+fx_mod2"
    assert pc.key_of(["fx_eq"]) == "fx_eq"
    assert pc.key_of([]) == ""


def test_scores_and_best_pick_the_smallest_remaining_score():
    r = row(10.0, {"fx_mod2": 6.0, "fx_eq": 8.0, "fx_eq+fx_mod2": 1.0})
    sc = pc.scores(r)
    assert sc[""] == 10.0
    assert pc.best(sc, 1) == ("fx_mod2", 6.0)
    assert pc.best(sc, 2) == ("fx_eq+fx_mod2", 1.0)
    assert pc.best(sc) == ("fx_eq+fx_mod2", 1.0)
    assert pc.best({"": 3.0}) == ("", None)


def test_greedy_adds_the_group_that_lowers_the_score_most_and_stops_without_data():
    r = row(10.0, {"fx_mod2": 6.0, "fx_eq": 8.0, "fx_eq+fx_mod2": 1.0})
    steps = pc.greedy(pc.scores(r))
    assert steps[0] == ("fx_mod2", 6.0)
    assert steps[1] == ("fx_eq+fx_mod2", 1.0)
    assert len(steps) == 2  # no data for a third group


def test_smallest_explaining_prefers_fewer_groups_then_lower_score():
    sc = {"": 10.0, "fx_eq": 1.9, "fx_mod2": 1.5, "fx_eq+fx_mod2": 0.0}
    assert pc.smallest_explaining(sc) == ("fx_mod2", 1.5)
    assert pc.smallest_explaining({"": 10.0, "fx_eq": 5.0}) == ("none", None)


def test_summarize_reports_floor_pairs_and_explaining_set():
    all_fx = pc.key_of(pc.FX_GROUPS)
    r = row(12.0, {"fx_mod1": 7.0, "fx_mod1+fx_mod2": 1.0, all_fx: 0.0})
    s = pc.summarize(r)
    assert s["score"] == 12.0 and s["floor"] == 0.0
    assert s["single"] == ("fx_mod1", 7.0)
    assert s["pair"] == ("fx_mod1+fx_mod2", 1.0)
    assert s["explain"] == ("fx_mod1+fx_mod2", 1.0)


def flip_row(full: float, flips: dict[str, float], name: str) -> dict:
    return {
        "entry": name,
        "default": render(),
        "firmware": shifted(full),
        "flip": {k: shifted(full - v) for k, v in flips.items()},  # distance to firmware = v
    }


def test_flip_scores_are_distances_to_the_firmware_render():
    r = flip_row(10.0, {"fx_mod2": 4.0}, "Program/Bank A/X.ns4p")
    sc = pc.flip_scores(r)
    assert sc[""] == 10.0
    assert abs(sc["fx_mod2"] - 4.0) < 1e-9


def test_flip_stats_count_improved_and_worse_programs():
    rows = [
        {"entry": "Program/Bank A/A.ns4p", "sc": {"": 12.0, "s": 2.0}},
        {"entry": "Program/Bank A/B.ns4p", "sc": {"": 6.0, "s": 7.0}},
        {"entry": "Program/Bank A/C.ns4p", "sc": {"": 1.0, "s": 1.2}},
    ]
    st = pc.flip_stats(rows, "s")
    assert st["n"] == 3 and st["over5_before"] == 2 and st["over5_after"] == 1
    assert st["over10_before"] == 1 and st["over10_after"] == 0
    assert st["worse"] == 1  # B rose by 1.0; C rose by 0.2, inside the margin
    assert st["max_before"] == 12.0 and st["max_after"] == 7.0


def test_quantile_is_nearest_rank():
    assert pc.quantile([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 0.9) == 9
    assert pc.quantile([5.0], 0.9) == 5.0


def test_flip_md_lists_distribution_and_worst_programs(tmp_path):
    rows = [
        {"entry": "Program/Bank A/A.ns4p", "sc": {"": 12.0, "s": 2.0}},
        {"entry": "Program/Bank A/B.ns4p", "sc": {"": 6.0, "s": 1.0}},
    ]
    md = pc.flip_md(rows, ["s"])
    assert "| none (default) | 2 |" in md and "| s | 2 |" in md
    assert "| 1 | Bank A/A | 12.0 | 2.0 |" in md


def test_main_writes_the_csv_and_table(tmp_path, capsys):
    import json

    fx = tmp_path / "fx.jsonl"
    fx.write_text(json.dumps(row(10.0, {"fx_mod2": 6.0})) + chr(10), encoding="utf-8")
    out = tmp_path / "combos.csv"
    assert pc.main(["--ablate-fx", str(fx), "--csv", str(out), "--md"]) == 0
    assert "fx_mod2 (6.00)" in capsys.readouterr().out
    text = out.read_text("utf-8")
    assert text.splitlines()[0].startswith("entry,score,left_without_fx_mod2")
    flips = tmp_path / "flip.jsonl"
    flips.write_text(
        json.dumps(flip_row(10.0, {"fx_mod2": 4.0}, "Program/Bank A/X.ns4p")) + chr(10)
    )
    fcsv = tmp_path / "flips.csv"
    assert pc.main(["--flip", str(flips), "--csv", str(fcsv), "--md"]) == 0
    assert "after_flip_fx_mod2" in fcsv.read_text("utf-8")
    assert "| fx_mod2 | 1 |" in capsys.readouterr().out
