"""Tests of `program_rank_chain`: command line, result extraction, resume, program listing."""

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import program_rank_chain as pc  # noqa: E402


def ns() -> argparse.Namespace:
    p = Path("x")
    return argparse.Namespace(
        fw_dir=p, eeprom=p / "e", backup=p / "b", pno1=p / "p1", pno2=p / "p2", org2=p / "o2",
        sy=p / "sy", live=p / "live",
    )  # fmt: skip


def test_entry_name_of_a_program_file() -> None:
    assert pc.entry_of(Path("a/Program/Bank C/My Pad.ns4p")) == "Program/Bank C/My Pad.ns4p"


def test_live_command_has_default_chips_and_optional_return_path() -> None:
    cmd = pc.live_command(ns(), Path("Bank A/P.ns4p"), Path("o.json"), audio=False)
    chips = [cmd[i + 1] for i, a in enumerate(cmd) if a == "--chip"]
    assert [c.split("=")[0] for c in chips] == ["PNO:1", "PNO:2", "SY:0"]
    assert "--return-path" not in cmd and "--sy-timeline" in cmd
    assert pc.WINDOW in chips[1]
    assert pc.live_command(ns(), Path("P"), Path("o"), audio=True)[-1] == "--return-path"


def test_chips_follow_the_engines_of_scene_one() -> None:
    assert pc.chips_for(["organ"]) == ["PNO1"]
    assert pc.chips_for(["piano"]) == ["PNO1"]
    assert pc.chips_for(["none"]) == ["PNO1"]
    assert pc.chips_for(["synth"]) == ["PNO2"]
    assert pc.chips_for(["organ", "piano", "sample"]) == ["PNO1", "PNO2", "SY0"]
    cmd = pc.live_command(ns(), Path("P"), Path("o"), audio=False, chips=["PNO2"])
    assert "--sy-timeline" not in cmd and cmd.count("--chip") == 1


def run_result() -> dict:
    chip = {
        "lane_peak": 0.5,
        "lane_rms": 0.1,
        "messages_sent": 7,
        "passes": 3,
        "halted": False,
        "errors": [],
    }
    return {
        "final": {"PNO1": chip, "PNO2": chip},
        "sy_timeline": {"prepare": 2, "go": 1, "peak": 0.04, "audio_ticks": 9, "junk": 1},
        "flagged_layers": [0, 2],
        "arm_messages": {"SY0p1": 290},
        "ticks": 2700,
    }


def test_extract_keeps_only_numbers_of_interest() -> None:
    e = pc.extract(run_result())
    assert set(e["chips"]) == {"PNO1", "PNO2"} and e["chips"]["PNO1"]["peak"] == 0.5
    assert e["sy"]["prepare"] == 2 and "junk" not in e["sy"]
    assert e["sy"]["voice0_playing_tick"] is None
    assert e["flagged_layers"] == 2 and e["ticks"] == 2700


def test_extract_without_sy_timeline() -> None:
    r = run_result()
    del r["sy_timeline"]
    assert pc.extract(r)["sy"]["prepare"] is None


def test_finished_reads_entries_for_resume(tmp_path: Path) -> None:
    f = tmp_path / "chain.jsonl"
    assert pc.finished(f) == set()
    f.write_text(json.dumps({"entry": "a"}) + "\n" + json.dumps({"entry": "b"}) + "\n", "utf-8")
    assert pc.finished(f) == {"a", "b"}


def test_programs_listing_filters_and_orders(tmp_path: Path) -> None:
    for bank, name in [("Bank B", "z"), ("Bank A", "y"), ("Bank A", "x")]:
        d = tmp_path / bank
        d.mkdir(exist_ok=True)
        (d / f"{name}.ns4p").write_text("", "utf-8")
    names = [p.stem for p in pc.programs_of(tmp_path, None, 1, 10)]
    assert names == ["x", "y", "z"]
    assert [p.stem for p in pc.programs_of(tmp_path, "y", 1, 10)] == ["y"]
    assert [p.stem for p in pc.programs_of(tmp_path, None, 2, 10)] == ["x", "z"]
    assert len(pc.programs_of(tmp_path, None, 1, 2)) == 2


def test_arp_programs_are_held_longer_and_the_command_carries_it() -> None:
    assert pc.key_ticks_for({"arp": True}) == pc.ARP_KEY_TICKS == 15_000
    assert pc.key_ticks_for({"arp": False}) is None
    assert pc.key_ticks_for(None) is None
    cmd = pc.live_command(ns(), Path("P.ns4p"), Path("o.json"), False, ["PNO2"], 15_000)
    assert cmd[cmd.index("--key-ticks") + 1] == "15000"
    assert "--key-ticks" not in pc.live_command(ns(), Path("P.ns4p"), Path("o.json"), False)
