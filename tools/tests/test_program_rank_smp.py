"""Tests of `program_rank_smp`: merging the SMP census into the ranking CSV rows."""

import csv
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import program_rank_smp as ps  # noqa: E402


def row(entry: str, engines: str = "piano", flags: str = "piano_no_chain_tap") -> dict:
    return {
        "entry": entry,
        "engines": engines,
        "smp_peak_dbfs": "",
        "smp_cycles": "",
        "flags": flags,
    }


def rec(entry: str, smp: dict | None, census: bool = True) -> dict:
    return {"entry": entry, "census": {"smp": smp} if census else None}


def test_audio_tap_drops_the_no_tap_flag_and_fills_columns() -> None:
    smp = {"kind": "audio", "peak_dbfs": -23.456, "fundamental_cycles": 4.123456}
    out = ps.merge([row("a")], [rec("a", smp)])[0]
    assert (
        out["flags"] == "" and out["smp_peak_dbfs"] == "-23.46" and out["smp_cycles"] == "4.12346"
    )


def test_silent_tap_flags_no_audio_and_keeps_foreign_flags() -> None:
    smp = {"kind": "silent", "peak_dbfs": None, "fundamental_cycles": None}
    out = ps.merge(
        [row("a", "organ+piano", "organ_chain_silent;piano_no_chain_tap")], [rec("a", smp)]
    )[0]
    assert out["flags"] == "organ_chain_silent;piano_chain_no_audio" and out["smp_peak_dbfs"] == ""


def test_failed_run_and_missing_record_keep_the_row_flagged() -> None:
    out = ps.merge([row("a"), row("b"), row("c", "synth", "")], [rec("a", None, census=False)])
    assert out[0]["flags"] == "piano_no_chain_tap;smp_run_failed"
    assert out[1] == row("b") and out[2] == row("c", "synth", "")


def test_non_piano_row_is_untouched_even_with_a_record() -> None:
    assert ps.merge([row("c", "synth", "")], [rec("c", None)])[0]["flags"] == ""


def test_summary_counts_outcomes() -> None:
    rows = [
        row("a", flags=""),
        row("b"),
        row("c", flags="piano_chain_no_audio"),
        row("d", "synth", ""),
    ]
    rows[0]["smp_peak_dbfs"] = "-20"
    assert ps.summary(rows) == {
        "piano_programs": 3,
        "tap_audio": 1,
        "no_audio": 1,
        "no_tap": 1,
        "run_failed": 0,
    }


def test_main_rewrites_only_the_csv_columns(tmp_path: Path) -> None:
    csv_path, chain = tmp_path / "r.csv", tmp_path / "c.jsonl"
    with csv_path.open("w", newline="", encoding="utf-8") as fh:
        w = csv.DictWriter(fh, fieldnames=list(row("a")), lineterminator="\n")
        w.writeheader()
        w.writerow(row("a"))
    chain.write_text(
        '{"entry": "a", "census": {"smp": {"kind": "audio", '
        '"peak_dbfs": -9, "fundamental_cycles": 2}}}\n'
    )
    assert ps.main(["--chain", str(chain), "--csv", str(csv_path)]) == 0
    assert csv_path.read_text("utf-8").splitlines()[1] == "a,piano,-9,2,"


def test_main_adds_the_columns_to_an_old_csv_before_flags(tmp_path: Path) -> None:
    csv_path, chain = tmp_path / "r.csv", tmp_path / "c.jsonl"
    csv_path.write_text(
        "entry,engines,sy_audio_peak,flags\na,piano,,piano_no_chain_tap\nb,synth,,\n"
    )
    chain.write_text(
        '{"entry": "a", "census": {"smp": {"kind": "audio", '
        '"peak_dbfs": -9, "fundamental_cycles": 2}}}\n'
    )
    assert ps.main(["--chain", str(chain), "--csv", str(csv_path)]) == 0
    lines = csv_path.read_text("utf-8").splitlines()
    assert lines == [
        "entry,engines,sy_audio_peak,smp_peak_dbfs,smp_cycles,flags",
        "a,piano,,-9,2,",
        "b,synth,,,,",
    ]
    assert not csv_path.with_suffix(".tmp").exists()
