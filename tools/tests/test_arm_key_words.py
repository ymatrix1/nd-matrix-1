"""Tests of `arm_key_words`: key word fields against the committed key-down captures."""

import csv
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import arm_key_words as kw  # noqa: E402

DATA = Path(__file__).resolve().parents[2] / "docs" / "research" / "data"


def test_plain_key_on_and_off_words() -> None:
    on, off = kw.decode(0x0B00E200), kw.decode(0x0800E200)
    assert on["keyboard"] and on["banks"] == 3 and on["note"] == 60 and on["velocity_byte"] == 0xE2
    assert off["banks"] == 0 and kw.is_on(0x0B00E200) and not kw.is_on(0x0800E200)
    assert kw.decode(0x4B04E200)["group_voice"] == 2 and kw.decode(0x4B04E200)["note"] == 64


def test_arp_step_words_one_bank_and_both_banks() -> None:
    synth_engine, gravity = kw.decode(0x0100E200), kw.decode(0x03F4E200)
    assert (
        not synth_engine["keyboard"] and synth_engine["banks"] == 1 and synth_engine["note"] == 60
    )
    assert gravity["banks"] == 3 and gravity["note"] == 48
    assert kw.decode(0x02F4E200)["banks"] == 2 and not kw.is_on(0x00F4E200)


def test_both_banks_poke_only_changes_on_words() -> None:
    assert kw.with_both_banks(0x0100E200) == 0x0300E200
    assert kw.with_both_banks(0x0000E200) == 0x0000E200


def test_note_offsets_match_the_committed_key_down_capture() -> None:
    rows = list(csv.DictReader((DATA / "keydown-synth-keys.csv").open(encoding="utf-8")))
    rows = [r for r in rows if r["velocity"] == "100" and r["kind"] == "synthA"]
    assert len(rows) > 20
    for r in rows:
        d = kw.decode(int(r["value"], 0))
        assert d["note"] == int(r["note"]) and d["velocity_byte"] == 0xE2 and d["keyboard"]
        assert kw.is_on(int(r["value"], 0)) == (r["phase"] == "on")
