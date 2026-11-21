"""Tests for the LP24 / HP24 before / after scoring (no renders)."""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import program_rank_amp as pa  # noqa: E402


def metrics(rms: float, centroid: float = 1000.0) -> dict:
    return {"rms_db": rms, "peak_db": rms + 6, "centroid_hz": centroid, "bands_db": [-30.0] * 8}


def line(default: float, flip: float, fw: float) -> str:
    return json.dumps(
        {
            "entry": "x.ns4p",
            "name": "X",
            "amp_filters": {"lp24": 1, "hp24": 0},
            "default": metrics(default),
            "firmware": metrics(fw),
            "flip": {pa.KEY: metrics(flip)},
        }
    )


def test_rows_score_before_and_after_against_the_firmware_render(tmp_path: Path) -> None:
    p = tmp_path / "r.jsonl"
    p.write_text(line(-20.0, -26.0, -26.0) + "\n" + line(-20.0, -20.0, -26.0) + "\n")
    rs = pa.rows(str(p))
    assert rs[0]["before"] > rs[0]["after"] and rs[0]["after"] == 0.0
    assert rs[1]["before"] == rs[1]["after"]
    s = pa.summary(rs)
    assert (s["programs"], s["improved"], s["unchanged"], s["worse"]) == (2, 1, 1, 0)


def test_summary_of_nothing_is_zero() -> None:
    assert pa.summary([])["programs"] == 0
