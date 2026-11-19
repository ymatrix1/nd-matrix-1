"""Unit tests for tools/arm_program_touch_bits.py (pure helpers and the committed recall data)."""

from __future__ import annotations

import io
import json
import zipfile
from pathlib import Path

import arm_program_touch_bits as tb
import pytest

DATA = Path(__file__).resolve().parents[2] / "docs" / "research" / "data"
CENSUS_TEXT = (
    "entry,slot,name,touch_a,touch_b,layer_a_on,layer_b_on,section_on\n"
    "Program/Bank A/Pearl Upright.ns4p,A:11,Pearl Upright,0,1,1,0,1\n"
    "Program/Bank A/Shimmer Upright.ns4p,A:43,Shimmer Upright,2,1,1,0,1\n"
)


def test_constants_are_the_registry_and_handler_values() -> None:
    assert tb.TOUCH_EDIT_ID == 0x159
    assert tb.TOUCH_STORE_PC == 0xC011AA38
    assert tb.CURVE_IDS == (10, 11, 13)


@pytest.mark.parametrize(("value", "curve"), [(0, 10), (1, 11), (2, 13)])
def test_expected_curve_id_follows_the_handler_table(value: int, curve: int) -> None:
    assert tb.expected_curve_id(value) == curve


def test_parse_census_reads_both_layers_and_use() -> None:
    census = tb.parse_census(CENSUS_TEXT)
    assert census["Program/Bank A/Pearl Upright.ns4p"] == {"A": 0, "B": 1, "in_use": True}
    assert census["Program/Bank A/Shimmer Upright.ns4p"]["A"] == 2


def test_parse_result_line_ignores_other_output() -> None:
    assert tb.parse_result_line("boot ok") is None
    assert tb.parse_result_line('RESULT {"A": 1}') == {"A": 1}


def test_compare_program_lists_layer_and_curve_mismatches() -> None:
    file_touch = {"A": 0, "B": 1}
    assert tb.compare_program(file_touch, {"A": 0, "B": 1, "curve_ids": [10, 11]}) == []
    bad = tb.compare_program(file_touch, {"A": 2, "B": 1, "curve_ids": [13, 11]})
    assert len(bad) == 2 and bad[0].startswith("layer A") and bad[1].startswith("curveId")
    assert tb.compare_program(file_touch, {"curve_ids": []})  # nothing recalled is a mismatch


def test_check_counts_matches_and_reports_missing_and_extra() -> None:
    census = tb.parse_census(CENSUS_TEXT)
    good = {"A": 0, "B": 1, "curve_ids": [10, 11]}
    results = {"Program/Bank A/Pearl Upright.ns4p": good, "Program/Bank A/Extra.ns4p": good}
    matching, problems = tb.check(census, results)
    assert matching == 1
    assert any("Shimmer Upright" in p and "no recall result" in p for p in problems)
    assert any("Extra" in p and "not in the census" in p for p in problems)


def test_summary_reports_n_of_n_and_the_value_mix() -> None:
    census = tb.parse_census(CENSUS_TEXT)
    text = tb.summary(census, 2)
    assert text.startswith("2 of 2 programs")
    assert "values heavy/medium/light over 4 layers: 1/2/1" in text


def test_read_member_returns_the_archive_bytes(tmp_path: Path) -> None:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        z.writestr("Program/Bank A/X.ns4p", b"abc")
    path = tmp_path / "x.ns4b"
    path.write_bytes(buf.getvalue())
    assert tb.read_member(path, "Program/Bank A/X.ns4p") == b"abc"


def test_check_command_prints_n_of_n(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    census = tmp_path / "c.csv"
    census.write_text(CENSUS_TEXT, encoding="utf-8")
    good = {"A": 0, "B": 1, "curve_ids": [10, 11]}
    light = {"A": 2, "B": 1, "curve_ids": [13, 11]}
    results = tmp_path / "r.json"
    results.write_text(
        json.dumps(
            {
                "programs": {
                    "Program/Bank A/Pearl Upright.ns4p": good,
                    "Program/Bank A/Shimmer Upright.ns4p": light,
                }
            }
        ),
        encoding="utf-8",
    )
    assert tb.main(["check", str(census), str(results)]) == 0
    assert capsys.readouterr().out.startswith("2 of 2 programs")


def test_committed_recall_equals_the_decoder_census_for_every_factory_program() -> None:
    """The firmware recall of every factory program equals the decoded file bits (D99 follow-up).

    Both files are committed derived data: the census is `piano_touch_census --csv` (decoder), the
    recall is `arm_program_touch_bits.py batch` (firmware).
    """
    results_path = DATA / "piano-touch-recall.json"
    census = tb.parse_census((DATA / "piano-touch-program.csv").read_text(encoding="utf-8"))
    results = json.loads(results_path.read_text(encoding="utf-8"))["programs"]
    matching, problems = tb.check(census, results)
    assert problems == []
    assert matching == len(census) == 384
