"""Unit tests for tools/arm_sender_sites.py (synthetic ARM words only)."""

from __future__ import annotations

import struct
from pathlib import Path

import arm_sender_sites as s

BASE = 0xC000_0000
W10, W78 = s.PATH1_WRITERS


def _bl(pc: int, target: int) -> int:
    return 0xEB00_0000 | (((target - (pc + 8)) >> 2) & 0x00FF_FFFF)


def _code(*words: int) -> bytes:
    return struct.pack(f"<{len(words)}I", *words)


def _const_site() -> bytes:
    return _code(
        0xE59F_1004,  # ldr r1, [pc, #4]  (literal at +12)
        0xE3A0_0004,  # mov r0, #4
        _bl(BASE + 8, W10),
        0x0009_4213,  # literal
    )


def test_find_sites_resolves_constant_comm_and_word1() -> None:
    sites = s.find_sites(_const_site())
    assert sites == [s.Site(BASE + 8, W10, 4, 0x9_4213, None)]
    assert sites[0].chip == "SMP"


def test_unknown_comm_and_other_callees() -> None:
    code = _code(0xE595_0000, _bl(BASE + 4, W78), _bl(BASE + 8, 0xC000_1000))
    sites = s.find_sites(code)
    assert len(sites) == 1 and sites[0].comm is None and sites[0].chip == "unknown"
    assert s.Site(0, W10, 7, None, None).chip == "comm7"


def test_window_detection_ignores_opcode_bits() -> None:
    assert s.in_window(0x9354A) and s.in_window(0x1009_3500)
    assert not s.in_window(0x935A0) and not s.in_window(None) and not s.in_window(0x92533)


def test_summarise_counts() -> None:
    sites = [s.Site(1, W10, 3, 0x9354A, None), s.Site(2, W10, None, None, None)]
    info = s.summarise(sites)
    assert info["sites"] == 2 and info["by_chip"] == {"SY": 1, "unknown": 1}
    assert info["window_hits"] == [1] and info["unknown_comm"] == [2]
    assert info["const_word1"] == 1


def test_main_prints_summary(tmp_path: Path, capsys) -> None:  # noqa: ANN001
    path = tmp_path / "code.bin"
    path.write_bytes(_const_site())
    assert s.main([str(path), "--list"]) == 0
    out = capsys.readouterr().out
    assert "call sites: 1" in out and "word1=0x94213" in out and "SMP" in out
