"""Tests of `program_rank`: distance score, engine tags, chain flags, CSV determinism."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import program_rank as pr  # noqa: E402


def render(rms=-24.0, centroid=1000.0, bands=None) -> dict:
    return {
        "rms_db": rms,
        "peak_db": rms + 12.0,
        "centroid_hz": centroid,
        "bands_db": bands or [-90.0, -40.0, -10.0, -8.0, -12.0, -20.0, -40.0, -90.0],
    }


def native(entry="Program/Bank A/X.ns4p", organ=False, piano=False, synth=True, sample=0, **kw):
    return {
        "entry": entry,
        "scene_1": {"organ": organ, "piano": piano, "synth": synth},
        "scene_2": {"organ": False, "piano": False, "synth": False},
        "layers": {"organ_model": "B3", "piano": int(piano), "synth": 1, "synth_sample": sample},
        "default": kw.get("default", render()),
        "firmware": kw.get("firmware", render()),
    }


def chain(pno1=0.0, pno2=0.0, org2=0.0, prepare=0, audio=None):
    chips = {
        "PNO1": {"peak": pno1},
        "PNO2": {"peak": pno2},
        "ORG2": {"peak": org2},
    }
    return {
        "entry": "Program/Bank A/X.ns4p",
        "census": {"chips": chips, "sy": {"prepare": prepare}},
        "audio": {"sy": {"peak": audio}} if audio is not None else None,
    }


def test_identical_renders_score_zero() -> None:
    d = pr.distance(render(), render())
    assert d["score"] == 0.0 and d["d_level_db"] == 0.0


def test_score_adds_level_bands_and_weighted_centroid() -> None:
    a = render()
    b = render(rms=-21.0, centroid=2000.0, bands=[x + 2.0 for x in a["bands_db"]])
    d = pr.distance(a, b)
    assert abs(d["d_level_db"] - 3.0) < 1e-9
    assert abs(d["d_centroid_oct"] - 1.0) < 1e-9
    # bands -90 dB are not relevant: only 6 bands count, each +2 dB
    assert abs(d["d_band_db"] - 2.0) < 1e-9
    assert abs(d["score"] - (3.0 + 2.0 + 6.0)) < 1e-9
    assert pr.dominant(d) == "centroid"


def test_silent_mismatch_and_both_silent_and_panic() -> None:
    quiet = render(rms=-200.0)
    assert pr.distance(quiet, render())["score"] == pr.MISMATCH_SCORE
    assert pr.distance(quiet, quiet)["score"] == 0.0
    d = pr.distance({"panic": "x"}, render())
    assert d["score"] is None and pr.dominant(d) == "panic"
    assert pr.dominant(pr.distance(quiet, render())) == "silent_mismatch"


def test_engines_split_sample_from_synth() -> None:
    assert pr.engines(native(organ=True, piano=True, synth=True)) == ["organ", "piano", "synth"]
    assert pr.engines(native(synth=True, sample=1)) == ["sample"]
    assert pr.engines(native(synth=False)) == ["none"]


def test_chain_flags_per_engine() -> None:
    f = pr.chain_flags(native(synth=True), chain())
    assert f["flags"] == ["synth_chain_silent"]
    assert pr.chain_flags(native(synth=True), chain(pno2=0.4))["flags"] == []
    assert pr.chain_flags(native(synth=True), chain(org2=0.1))["flags"] == []
    o = pr.chain_flags(native(organ=True, synth=False), chain())
    assert o["flags"] == ["organ_chain_silent"]
    p = pr.chain_flags(native(piano=True, synth=False), chain())
    assert p["flags"] == ["piano_no_chain_tap"]


def test_piano_chain_tap_column_replaces_the_no_tap_flag_when_the_smp_tap_has_audio() -> None:
    n = native(piano=True, synth=False)
    c = chain()
    c["census"]["smp"] = {"kind": "audio", "peak_dbfs": -15.9, "fundamental_cycles": 0.00601}
    f = pr.chain_flags(n, c)
    assert f["flags"] == [] and (f["smp_peak_dbfs"], f["smp_cycles"]) == (-15.9, 0.00601)
    c["census"]["smp"] = {"kind": "silent", "peak_dbfs": None, "fundamental_cycles": None}
    assert pr.chain_flags(n, c)["flags"] == ["piano_chain_no_audio"]
    assert "smp_peak_dbfs" in pr.csv_header() and "smp_cycles" in pr.csv_header()


def test_sample_flags_cover_no_voice_lost_layer_and_silent_audio() -> None:
    n = native(sample=3)
    assert pr.chain_flags(n, chain(prepare=0))["flags"] == ["sample_no_voice"]
    assert pr.chain_flags(n, chain(prepare=2))["flags"] == ["sample_layer_lost"]
    ok = pr.chain_flags(n, chain(prepare=3, audio=0.05))
    assert ok["flags"] == [] and ok["sy_audio_peak"] == 0.05
    assert pr.chain_flags(n, chain(prepare=3, audio=0.0))["flags"] == ["sample_voiced_no_audio"]


def test_failed_chain_run_is_flagged() -> None:
    assert pr.chain_flags(native(), None)["flags"] == ["chain_run_failed"]
    assert (
        pr.chain_flags(native(), {"entry": "x", "census": None, "audio": None})["chain_run"]
        is False
    )


def test_rows_csv_and_tables_are_deterministic(tmp_path: Path) -> None:
    ns = [
        native("Program/Bank B/Z.ns4p", firmware=render(rms=-18.0)),
        native("Program/Bank A/X.ns4p"),
    ]
    rows = pr.build_rows(ns, [chain(pno2=0.3)])
    assert [r["entry"] for r in rows] == ["Program/Bank A/X.ns4p", "Program/Bank B/Z.ns4p"]
    a, b = tmp_path / "a.csv", tmp_path / "b.csv"
    pr.write_csv(rows, a)
    pr.write_csv(pr.build_rows(list(reversed(ns)), [chain(pno2=0.3)]), b)
    assert a.read_bytes() == b.read_bytes()
    lines = a.read_text("utf-8").splitlines()
    assert lines[0].split(",") == pr.csv_header() and len(lines) == 3
    assert pr.worst(rows)[0]["entry"].endswith("Z.ns4p")
    t = pr.tables(rows)
    assert "Median score" in t and "chain_run_failed" in t


def test_main_writes_csv_from_jsonl(tmp_path: Path, capsys) -> None:
    import json

    nat, ch = tmp_path / "n.jsonl", tmp_path / "c.jsonl"
    nat.write_text(json.dumps(native()) + "\n", "utf-8")
    ch.write_text(json.dumps(chain(pno2=0.2)) + "\n", "utf-8")
    out = tmp_path / "o.csv"
    assert (
        pr.main(["--native", str(nat), "--chain", str(ch), "--csv", str(out), "--md-tables"]) == 0
    )
    assert out.exists() and "1 programs" in capsys.readouterr().out
    assert pr.bank_of("Program/Bank C/Q.ns4p") == "C"


def test_cause_row_names_the_group_whose_revert_removes_the_distance(tmp_path: Path) -> None:
    base = render()
    row = {
        "entry": "Program/Bank A/X.ns4p",
        "default": base,
        "firmware": render(rms=-18.0),
        "ablate": {"fx": base, "organ_tone": render(rms=-18.0), "synth_filter": render(rms=-20.0)},
    }
    c = pr.cause_row(row)
    assert c["carrier"] == "fx" and c["left"]["fx"] == 0.0 and c["score"] == 6.0
    out = tmp_path / "c.csv"
    pr.write_cause_csv([c], out)
    head = out.read_text("utf-8").splitlines()[0]
    assert (
        head
        == "entry,score,left_without_fx,left_without_organ_tone,left_without_synth_filter,carrier"
    )
    same = {**row, "firmware": base}
    assert pr.cause_row(same)["carrier"] == "none"


def test_expected_silence_is_not_a_chain_defect() -> None:
    silent = {"audible": {"organ": False, "piano": False, "synth": False}}
    sounds = {"audible": {"organ": True, "piano": False, "synth": True}}
    n = native(organ=True, synth=True)
    assert pr.chain_flags(n, chain(), silent)["flags"] == [
        "organ_expected_silent",
        "synth_expected_silent",
    ]
    assert pr.chain_flags(n, chain(), sounds)["flags"] == [
        "organ_chain_silent",
        "synth_chain_silent",
    ]
    assert pr.chain_flags(n, chain())["flags"] == ["organ_chain_silent", "synth_chain_silent"]
    rows = pr.build_rows([n], [chain()], [{"entry": n["entry"], **silent}])
    assert rows[0]["chain"]["flags"] == ["organ_expected_silent", "synth_expected_silent"]


def test_by_engine_median_of_an_even_count_is_the_mean_of_the_two_middle_scores() -> None:
    """Regression: `sc[len(sc) // 2]` is the upper median (scores 1, 2, 3, 10 gave 3, not 2.5)."""
    rows = [
        {"engines": "synth", "dist": {"score": s}, "chain": {"flags": []}}
        for s in (10.0, 1.0, 3.0, 2.0)
    ]
    (row,) = pr.by_engine(rows)
    assert row[1] == 4 and row[2] == 2.5 and row[3] == 10.0
