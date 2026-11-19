"""Unit tests for tools/sample_gaps_cap.py (launcher helpers; the run needs firmware)."""

from __future__ import annotations

from argparse import Namespace
from pathlib import Path

import sample_gaps_cap as cap


def test_stem_and_first_member() -> None:
    assert cap.stem_of(Path("x/Bank A/Hybrid Str Whl.ns4p")) == "Bank_A__Hybrid_Str_Whl"
    assert cap.first_member({"synth_a": None, "synth_b": "b", "synth_c": "c"}) == "b"
    assert cap.first_member({"synth_a": None}) is None


def test_done_checks_the_capture_file(tmp_path: Path) -> None:
    prog = Path("Bank B/Marimba.ns4p")
    assert not cap.done(tmp_path, prog)
    d = tmp_path / "Bank_B__Marimba"
    d.mkdir()
    (d / "Bank_B__Marimba.txt.gz").write_bytes(b"x")
    assert cap.done(tmp_path, prog)


def test_command_serves_the_whole_member() -> None:
    ns = Namespace(fw_dir=Path("fw"), eeprom=Path("e.bin"), out=Path("o"), backup=Path("b.ns4b"))
    cmd = cap.command(ns, Path("Bank B/Marimba.ns4p"), "Samp Lib/m.nsmp4")
    assert cmd[cmd.index("--member-bytes") + 1] == cap.FULL == "0x1000000"
    assert cmd[cmd.index("--member") + 1] == "Samp Lib/m.nsmp4"
    assert cmd[-1] == str(Path("Bank B/Marimba.ns4p"))
