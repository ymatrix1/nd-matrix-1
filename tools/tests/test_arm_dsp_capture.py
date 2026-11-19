"""Unit tests for tools/arm_dsp_capture.py (row format, capture files, resume, worker command)."""

from __future__ import annotations

from argparse import Namespace
from pathlib import Path

import arm_dsp_capture as cap
import arm_emu as ae
import pytest


def msg(comm: int = 3, dm: int = 0x92414, value: int = 0x1C0794, path: int = 2, core: int = 1):
    base = 0x28800000 if core == 2 else 0x28000000
    word1 = base + 4 * dm if path == 2 else dm
    return ae.Message(path=path, comm=comm, word1=word1, word2=value, pc=0xC01E2CC8)


def test_row_round_trip() -> None:
    row = cap.format_row("k60", 7, msg())
    assert row == "k60 7 SY 2 1 0x92414 0x001c0794 0xc01e2cc8"
    assert cap.parse_row(row) == {
        "phase": "k60", "tick": 7, "chip": "SY", "path": 2, "core": 1,
        "dm": 0x92414, "w2": 0x1C0794, "pc": 0xC01E2CC8,
    }  # fmt: skip


def test_row_round_trip_core2_and_path1() -> None:
    assert cap.parse_row(cap.format_row("load", 0, msg(2, 0xC1DCC, 5, core=2)))["core"] == 2
    p1 = cap.parse_row(cap.format_row("load", 3, msg(4, 0x8000, 9, path=1)))
    assert (p1["chip"], p1["path"], p1["core"], p1["dm"]) == ("SMP", 1, 0, 0x8000)


def test_parse_row_rejects_short_lines() -> None:
    with pytest.raises(ValueError):
        cap.parse_row("load 0 SY")


def test_write_and_read_rows_atomic(tmp_path: Path) -> None:
    target = tmp_path / "sub" / "a.txt.gz"
    lines = [cap.format_row("load", n, msg(dm=0x100 + n)) for n in range(5)]
    cap.write_rows(target, lines)
    assert not target.with_suffix(".tmp").exists()
    rows = list(cap.read_rows(target))
    assert [r["dm"] for r in rows] == [0x100 + n for n in range(5)]


def test_out_name_uses_bank_and_program() -> None:
    assert cap.out_name(Path("x/Bank A/B3 Chords n Solo.ns4p")) == "Bank_A__B3_Chords_n_Solo"


def test_program_files_and_pending(tmp_path: Path) -> None:
    for bank, name in (("Bank A", "One"), ("Bank A", "Two"), ("Bank B", "Three")):
        (tmp_path / bank).mkdir(exist_ok=True)
        (tmp_path / bank / f"{name}.ns4p").write_bytes(b"x")
    (tmp_path / "Bank A" / "note.txt").write_text("no")
    files = cap.program_files(tmp_path)
    assert [p.stem for p in files] == ["One", "Two", "Three"]
    out = tmp_path / "out"
    cap.write_rows(out / "Bank_A__Two.txt.gz", [])
    assert [p.stem for p in cap.pending(files, out)] == ["One", "Three"]


def test_program_files_single_file(tmp_path: Path) -> None:
    one = tmp_path / "A.ns4p"
    one.write_bytes(b"x")
    assert cap.program_files(one) == [one]


def test_worker_command_passes_sample_arguments() -> None:
    ns = Namespace(
        image=Path("c.bin"), sram=Path("s.bin"), eeprom=Path("e.bin"), out=Path("o"),
        ns4b=Path("b.ns4b"), member="Samp Lib/x.nsmp4", spi_neutral=True,
        member_bytes=0x1000000,
    )  # fmt: skip
    cmd = cap.worker_command(ns, Path("p.ns4p"))
    assert cmd[3] == "one" and cmd[-1] == "p.ns4p"
    assert cmd[cmd.index("--member-bytes") + 1] == "0x1000000"
    assert cap.MEMBER_BYTES == 0x400000
    assert cmd[cmd.index("--member") + 1] == "Samp Lib/x.nsmp4"
    ns.ns4b = None
    assert "--ns4b" not in cap.worker_command(ns, Path("p.ns4p"))
    assert "--no-spi-neutral" not in cmd
    ns.spi_neutral = False
    assert "--no-spi-neutral" in cap.worker_command(ns, Path("p.ns4p"))


def test_phase_constants() -> None:
    assert cap.KEYS == (36, 60, 84)
    assert cap.FIELDS[:3] == ("phase", "tick", "chip")
