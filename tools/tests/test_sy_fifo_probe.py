"""Tests of `sy_fifo_probe`: snapshot and report parsing, summary."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import sy_fifo_probe as sp  # noqa: E402

SNAPS = (
    "S r=0,1,2,80000,4,5,6,7;i=93046,0,0,0,0,0,0,0;l=280,0 "
    "r=0,1,2,80000,4,5,6,7;i=1,2,3,4,5,6,7,8;l=280,0"
)


def test_snapshots_are_split_into_register_lists() -> None:
    snaps = sp.parse_snapshots(SNAPS)
    assert len(snaps) == 2
    assert snaps[0]["r"][3] == 0x80000 and snaps[0]["i"][0] == 0x93046 and snaps[0]["l"][0] == 0x280
    assert sp.parse_snapshots("S") == []


def test_report_numbers_come_from_the_sy_token() -> None:
    line = "P pass=7 sy=isrs:2,resets:12,pairs:30,queued:4,refills:0,staged:1 ihits=1c063f:9"
    assert sp.parse_report(line) == {
        "isrs": 2,
        "resets": 12,
        "pairs": 30,
        "queued": 4,
        "refills": 0,
        "staged": 1,
    }
    assert sp.parse_report("P pass=1") == {}


def test_summary_reports_r3_and_pairs_per_reset() -> None:
    out = sp.summarize(sp.parse_snapshots(SNAPS), {"resets": 10, "pairs": 25, "queued": 3})
    assert out["r3_values"] == ["0x80000"]
    assert out["pairs_per_reset"] == 2.5 and out["queued"] == 3 and out["snapshots"] == 2
    assert sp.summarize([], {})["pairs_per_reset"] == 0.0


def test_script_is_read_per_pass(tmp_path: Path) -> None:
    f = tmp_path / "SY0.script"
    f.write_text("3 0x92526 0x0009395e\n3 0x92527 0x1\n9 0x2400 0xff\n")
    msgs = sp.read_script(f)
    assert msgs[3] == [(0x92526, 0x9395E), (0x92527, 1)] and msgs[9] == [(0x2400, 0xFF)]
