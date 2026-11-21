"""Tests for the piano timbre gain model and the ARM capture helpers (no firmware needed)."""

from __future__ import annotations

import json
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import arm_piano_timbre as cap  # noqa: E402
import piano_timbre_gain as pt  # noqa: E402

G = [round(2**23 * 2 ** (-i / 60)) for i in range(61)]
G[0] = 0x7FFFFF


def fake_code(
    s: dict[int, int] | None = None,
    t1: dict[int, int] | None = None,
    t2: dict[int, int] | None = None,
) -> bytes:
    """A zeroed ROM image with the level table and a few table words set."""
    code = bytearray(0x2B0000)
    for i, v in enumerate(G):
        struct.pack_into("<i", code, pt.G_TABLE - pt.LOAD_BASE + 4 * i, v)
    for base, words in ((pt.S_TABLE, s), (pt.T1_TABLE, t1), (pt.T2_TABLE, t2)):
        for i, v in (words or {}).items():
            struct.pack_into("<i", code, base - pt.LOAD_BASE + 4 * i, v)
    return bytes(code)


def test_committed_cases_are_well_formed() -> None:
    cases = json.loads(pt.CASES.read_text(encoding="utf-8"))["cases"]
    assert len(cases) == 108
    assert {c["mode"] for c in cases} == {0, *range(0x10, 0x1B)}
    assert all(set(c) == {"mode", "key", "velocity", "r8", "word"} for c in cases)
    assert all(c["word"] == 0x3E800000 for c in cases if c["mode"] == 0)


def test_float_word_of_unity_is_a_quarter() -> None:
    assert pt.float_word(pt.UNITY_Q23) == 0x3E800000


def test_sl_is_r8_shifted_by_three() -> None:
    assert [pt.sl_of_r8(v) for v in (17, 65, 115, 203, 284, 416, 497, 656)] == [
        2,
        8,
        14,
        25,
        35,
        52,
        62,
        82,
    ]


def test_modes_without_tables_are_unity() -> None:
    tab = pt.load_tables(fake_code())
    assert pt.amp_q23(tab, 0, 60, 52) == pt.UNITY_Q23
    assert pt.amp_q23(tab, 0x0F, 60, 52) == pt.UNITY_Q23
    assert pt.amp_q23(tab, 0x1B, 60, 52) == pt.UNITY_Q23


def test_attenuation_index_follows_the_formula_and_clamps_at_zero() -> None:
    # mode 0x11 (r2 = 1): S[sl + 128], T1[key + 128], T2[key + 128]
    code = fake_code(s={50 + 128: -1000}, t1={30 + 128: 200}, t2={30 + 128: 393216})
    tab = pt.load_tables(code)
    assert pt.attenuation_index(tab, 0x11, 30, 50) == ((-1000 * 200 + 393216) * 10) >> 16
    negative = pt.load_tables(fake_code(s={128: -1000}, t1={128: 200}))
    assert pt.attenuation_index(negative, 0x11, 0, 0) == 0
    try:
        pt.attenuation_index(tab, 0x20, 0, 0)
    except ValueError:
        pass
    else:
        raise AssertionError("mode 0x20 has no tables")


def test_amp_octave_reduction_matches_the_level_table() -> None:
    tab = pt.load_tables(fake_code(t2={0: 60 * 65536 // 10}))  # n = 60: G[60] >> 2
    assert pt.amp_q23(tab, 0x10, 0, 0) == G[60] >> 2 == 0x100000
    tab = pt.load_tables(fake_code(t2={0: 70 * 65536 // 10}))  # n = 70: G[10] >> 3
    assert pt.amp_q23(tab, 0x10, 0, 0) == G[10] >> 3
    tab = pt.load_tables(fake_code())  # n = 0: G[0] >> 2 = 0x1FFFFF
    assert pt.amp_q23(tab, 0x10, 0, 0) == 0x1FFFFF


def test_check_reports_a_difference() -> None:
    code = fake_code()
    ok = {"mode": 0x10, "key": 0, "velocity": 1, "r8": 17, "word": pt.float_word(0x1FFFFF)}
    assert pt.check(code, [ok]) == []
    bad = dict(ok, word=0)
    assert len(pt.check(code, [bad])) == 1


def test_r8_table_reads_curve_six() -> None:
    code = bytearray(0x2B4000)
    base = pt.R8_TABLE + pt.R8_OFFSET + 2 * 128 * pt.R8_CURVE - pt.LOAD_BASE
    for v in range(1, 128):
        struct.pack_into("<h", code, base + 2 * v, 10 * v)
    table = pt.r8_table(bytes(code))
    assert len(table) == 127 and table[0] == 10 and table[63] == 640


def test_rust_tables_have_every_array() -> None:
    code = bytearray(0x2B4000)
    code[pt.G_TABLE - pt.LOAD_BASE : pt.G_TABLE - pt.LOAD_BASE + 4] = struct.pack("<i", 1)
    text = pt.rust_tables(bytes(code))
    for name, size in (("S", 1408), ("T1", 1408), ("T2", 1408)):
        assert f"pub const {name}: [i32; {size}]" in text
    assert "pub const R8: [u16; 127]" in text


def test_capture_helpers() -> None:
    assert cap.gain_word([(1, 2), (cap.GAIN_WORD1, 7), (cap.GAIN_WORD1, 9)]) == 7
    assert cap.gain_word([(1, 2)]) is None
    assert cap.case_rows(0x10, 60, 100, 416, 5) == {
        "mode": 16,
        "key": 60,
        "velocity": 100,
        "r8": 416,
        "word": 5,
    }
    assert cap.MODES == tuple(range(0x10, 0x1B))
