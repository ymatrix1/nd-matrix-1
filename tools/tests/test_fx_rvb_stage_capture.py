"""Tests for tools/fx_rvb_stage_capture.py and its committed capture (kRvbAmt / kRvbType words)."""

import csv
from collections import defaultdict
from pathlib import Path
from types import SimpleNamespace

import fx_rvb_stage_capture as sc

DATA = Path(__file__).resolve().parents[2] / "docs/research/data/fx-rvb-stage-capture.csv"


def test_parse_ints_takes_decimal_and_hex():
    assert sc.parse_ints("8,10,0x26") == [8, 10, 38]
    assert sc.parse_ints("") == []


def test_script_is_enable_type_then_amounts():
    steps = sc.script(8, [0, 127])
    assert [s[0] for s in steps] == ["enable", "type8", "amt0", "amt127"]
    assert steps[1][1] == "Edt_Effects_kRvbType"


def test_row_carries_the_owning_state_row():
    m = SimpleNamespace(core=1, dm_word=0xC1C0E, word2=0x3F800000, pc=0xC0215CC4)
    r = sc.row(4, "amt0", 8, 0, m, (0xC1C0C, -30))
    assert (r["row"], r["ctx"], r["dm_word"], r["word"]) == (
        "0xc1c0c",
        -30,
        "0xc1c0e",
        "0x3f800000",
    )
    assert sc.row(4, "amt0", 8, 0, m)["row"] == ""


def _amount_words() -> dict[tuple[str, str, str], set[tuple[str, str]]]:
    out: dict[tuple[str, str, str], set[tuple[str, str]]] = defaultdict(set)
    for r in csv.DictReader(DATA.open()):
        if r["step"].startswith("amt"):
            out[(r["unit"], r["type"], r["amt"])].add((r["row"], r["ctx"]))
    return out


def test_every_amount_edit_writes_exactly_the_send_and_dry_pair_of_two_rows():
    words = _amount_words()
    assert len(words) == 3 * 10 * 3  # units 4, 10, 6; types 2..11; amounts 0, 38, 127
    for key, got in words.items():
        assert len(got) == 4, key
        assert {c for _r, c in got} == {"-31", "-30", "18", "19"}, key


def test_no_reverb_edit_writes_a_stage_record_or_the_wrapper_words():
    for r in csv.DictReader(DATA.open()):
        assert not 0xB6000 <= int(r["dm_word"], 16) < 0xB7000, r  # dispatcher records
        if r["row"] in ("0xc1c0c", "0xc1b6c", "0xc1a28"):  # multi-tap rows of units 4, 10, 6
            assert r["ctx"] not in ("-28", "42", "43", "-25"), r


def test_amount_values_follow_the_send_law():
    seen = {}
    for r in csv.DictReader(DATA.open()):
        if r["step"].startswith("amt") and r["ctx"] in ("-31", "-30"):
            seen[(r["amt"], r["ctx"])] = r["word"]
    assert seen[("127", "-31")] == "0x3f800000" and seen[("127", "-30")] == "0x00000000"
    assert seen[("0", "-31")] == "0x00000000" and seen[("0", "-30")] == "0x3f800000"
    assert seen[("38", "-31")] == "0x3e19b500"  # x^2 (2 - x), x = 38 / 128 = 0.1531
