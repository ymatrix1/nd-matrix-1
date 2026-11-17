"""Unit tests for tools/arm_program_synth_velocity.py (pure helpers, no firmware)."""

from __future__ import annotations

import io
import zipfile
from pathlib import Path

import arm_program_synth_velocity as sv
import pytest


def test_edit_ids_are_the_registry_ids() -> None:
    assert sv.EDIT_IDS == {"amp": 0x131, "flt": 0x13C, "osc": 0x124}


def test_parse_expect_reads_layers() -> None:
    assert sv.parse_expect(["A=3,1,0", "C=0,0,1"]) == {"A": (3, 1, 0), "C": (0, 0, 1)}


@pytest.mark.parametrize("bad", ["D=1,1,1", "A=1,1", "A1,1,1"])
def test_parse_expect_rejects_bad_items(bad: str) -> None:
    with pytest.raises(ValueError):
        sv.parse_expect([bad])


def test_compare_lists_only_the_mismatching_layers() -> None:
    recalled = {"A": (3, 1, 0), "B": (0, 1, 1), "C": (2, 0, 1)}
    assert sv.compare(recalled, {"A": (3, 1, 0), "C": (2, 0, 1)}) == []
    out = sv.compare(recalled, {"A": (3, 1, 0), "B": (0, 1, 0)})
    assert len(out) == 1 and out[0].startswith("layer B")
    assert sv.compare({}, {"A": (0, 0, 0)}) == ["layer A: recalled None != file (0, 0, 0)"]


def test_read_member_returns_the_archive_bytes(tmp_path: Path) -> None:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        z.writestr("Program/Bank A/X.ns4p", b"abc")
    path = tmp_path / "x.ns4b"
    path.write_bytes(buf.getvalue())
    assert sv.read_member(path, "Program/Bank A/X.ns4p") == b"abc"
