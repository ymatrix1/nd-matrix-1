"""Unit tests for tools/ns4_arm_init_script.py (synthetic words, no firmware needed)."""

from __future__ import annotations

import ns4_arm_init_script as s


def test_parse_records_reads_type_4_blocks_and_skips_strays() -> None:
    words = [0x93DCF, 4, 2, 0x93E0C, 0xFFFFFFFF, 7, 1, 1, 0x3D, 4, 1, 0x95C69, 0x92426]
    assert s.parse_records(words) == [(0x93E0C, [0xFFFFFFFF, 7]), (0x95C69, [0x92426])]


def test_parse_records_rejects_addresses_outside_dm() -> None:
    assert s.parse_records([4, 1, 0x100, 5, 0, 0]) == []


def test_poke_lines_one_per_word() -> None:
    assert s.poke_lines([(0x95C7B, [0x95C6D, 0x1C12DD])]) == [
        "poke 0x95C7B 0x95C6D",
        "poke 0x95C7C 0x1C12DD",
    ]


def test_sy_records_from_a_synthetic_image() -> None:
    import struct

    size = s.SY_SPAN[1] - s.LOAD_BASE
    code = bytearray(size)
    struct.pack_into("<4I", code, s.SY_SPAN[0] - s.LOAD_BASE, 4, 1, 0x95C7B, 0x95C6D)
    assert s.sy_records(bytes(code)) == [(0x95C7B, [0x95C6D])]


def _synthetic_tcb_code() -> bytes:
    """A code image with a TCB record: slot k has A -> 0xB64CB + k (1 word), X -> 0x935D3 (1 word),
    B -> 0x93FF7 + 16 k (count 8 from the ISR, 0 here)."""
    import struct

    vals = [0] * 1224
    for k in range(s.SLOTS):
        for off, ii, c in ((0, 0xB64CB + k, 1), (6, 0x935D3, 1), (12, 0x93FF7 + 16 * k, 8)):
            p = s.TCB_BASE + 5 + s.SLOT_WORDS * k + off - s.TCB_BASE
            vals[p], vals[p - 1], vals[p - 2] = ii, 1, c
    size = s.TCB_SPAN[1] - s.LOAD_BASE
    code = bytearray(size)
    struct.pack_into("<3I", code, s.TCB_SPAN[0] - s.LOAD_BASE, 4, 1224, s.TCB_BASE)
    struct.pack_into("<1224I", code, s.TCB_SPAN[0] - s.LOAD_BASE + 12, *vals)
    return bytes(code)


def test_tcb_table_decodes_three_tcbs_per_slot_and_none_hits_the_pitch_window() -> None:
    code = _synthetic_tcb_code()
    ((addr, vals),) = s.tcb_records(code)
    assert (addr, len(vals)) == (s.TCB_BASE, 1224)
    nodes = s.tcb_nodes(vals)
    assert len(nodes) == 3 * s.SLOTS
    assert nodes[0]["p"] == 0xB6008 and nodes[1]["p"] == 0xB600E and nodes[2]["p"] == 0xB6014
    assert nodes[2]["ii"] == 0x93FF7 and nodes[-1]["ii"] == 0x93FF7 + 16 * 67
    lines = s.tcb_summary(nodes)
    assert all(x.endswith("pitch window hits 0") for x in lines)
    assert lines[0].startswith("A: 68 TCBs, internal 0xB64CB..0xB650E")


def test_tcb_summary_counts_a_window_hit() -> None:
    nodes = [
        {
            "slot": 0,
            "kind": ord(k),
            "p": 0,
            "cp": 0,
            "em": 1,
            "ei": 0,
            "c": 2,
            "im": 1,
            "ii": 0x93549,
        }
        for k in "AXB"
    ]
    assert all(x.endswith("pitch window hits 2") for x in s.tcb_summary(nodes))


def test_tcb_records_rejects_a_missing_record() -> None:
    import pytest

    code = bytes(s.TCB_SPAN[1] - s.LOAD_BASE)
    with pytest.raises(ValueError):
        s.tcb_records(code)


def test_tcb_b_count_zero_is_checked_with_the_isr_count() -> None:
    nodes = [
        {
            "slot": 0,
            "kind": ord("B"),
            "p": 0,
            "cp": 0,
            "em": 1,
            "ei": 0,
            "c": 0,
            "im": 1,
            "ii": 0x93549,
        }
    ]
    nodes += [{**nodes[0], "kind": ord(k), "c": 1, "ii": 0x93FF7} for k in "AX"]
    lines = s.tcb_summary(nodes)
    assert lines[2].startswith("B: 1 TCBs, internal 0x93549..0x93550")
    assert lines[2].endswith("pitch window hits 8")


def test_tcb_records_rejects_a_short_image() -> None:
    import pytest

    with pytest.raises(ValueError):
        s.tcb_records(bytes(16))


def test_main_emits_tcb_pokes_and_the_summary(tmp_path, capsys) -> None:
    code = tmp_path / "code.bin"
    code.write_bytes(_synthetic_tcb_code())
    assert s.main([str(code), "--tcb-summary"]) == 0
    assert "A: 68 TCBs" in capsys.readouterr().out
    assert s.main([str(code), "--tcb"]) == 0
    assert capsys.readouterr().out.startswith("poke 0xB6003 ")
