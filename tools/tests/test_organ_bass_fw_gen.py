"""Tests for the install delta builder of organ_bass_fw_gen.py (D128)."""

from __future__ import annotations

import organ_bass_fw_gen as gen


def words(*pairs: tuple[int, int]) -> dict[int, int]:
    return dict(pairs)


def test_ranges_merge_addresses_within_the_gap_and_split_beyond_it() -> None:
    w = words((0x100, 1), (0x103, 2), (0x140, 3), (0x181, 4), (0x2000, 5))
    # 0x140 - 0x103 = 0x3D <= 0x40 merges; 0x181 - 0x140 = 0x41 splits
    assert gen.ranges(w) == [(0x100, 0x41), (0x181, 1), (0x2000, 1)]
    assert gen.ranges(w, gap=0x4000) == [(0x100, 0x1F01)]
    assert gen.ranges(words((7, 1))) == [(7, 1)]


def test_static_words_read_the_kept_ranges_of_a_flat_sram_image() -> None:
    sram = bytearray(4 * 0x300)
    sram[4 * 0x10 : 4 * 0x10 + 4] = (5).to_bytes(4, "little")
    sram[4 * 0x11 : 4 * 0x11 + 4] = (0).to_bytes(4, "little")
    sram[4 * 0x200 : 4 * 0x200 + 4] = (9).to_bytes(4, "little")  # outside the kept ranges
    assert gen.static_words(bytes(sram), [(0x10, 4)]) == {0x10: 5}


def test_install_delta_lists_changed_new_and_cleared_words() -> None:
    static = {0x10: 5, 0x11: 1, 0x12: 7}
    now = {0x10: 5, 0x11: 0, 0x13: 8}
    # 0x10 is unchanged, 0x11 is cleared by the install (0), 0x12 vanished (0), 0x13 is new
    assert gen.install_delta(now, static) == [(0x11, 0), (0x12, 0), (0x13, 8)]


def code_with(*vals: int, pad: int = 8) -> bytes:
    return b"\x00" * pad + b"".join(v.to_bytes(4, "little") for v in vals)


def test_delta_lines_turn_runs_found_in_the_code_into_a_lines_and_keep_the_rest_inline() -> None:
    run = list(range(100, 100 + gen.MIN_A))
    code = code_with(*run)
    delta = [(0x20 + k, v) for k, v in enumerate(run)] + [(0x40, 3), (0x41, 4)]
    assert gen.delta_lines(delta, code) == [f"A,20,{gen.MIN_A:x},8", "M,40,3 4"]


def test_delta_lines_keep_short_runs_zero_words_and_unknown_runs_inline() -> None:
    run = list(range(100, 100 + gen.MIN_A))
    code = code_with(*run)
    short = [(0x20 + k, v) for k, v in enumerate(run[:3])]
    assert gen.delta_lines(short, code) == ["M,20,64 65 66"]
    cleared = [(0x20 + k, 0 if k == 2 else v) for k, v in enumerate(run)]
    lines = gen.delta_lines(cleared, code)
    assert all(line.startswith("M") for line in lines), lines
    unknown = [(0x20 + k, 0x1000 + k) for k in range(gen.MIN_A)]
    assert gen.delta_lines(unknown, code) == [f"M,20,{' '.join(f'{v:x}' for _a, v in unknown)}"]


def test_delta_lines_split_a_run_at_the_end_of_a_code_match() -> None:
    run = list(range(100, 100 + gen.MIN_A))
    code = code_with(*run)
    delta = [(0x20 + k, v) for k, v in enumerate(run + [0x555, 0x556])]
    assert gen.delta_lines(delta, code) == [f"A,20,{gen.MIN_A:x},8", "M,28,555 556"]


def test_image_text_parses_registers_and_word_runs() -> None:
    regs, w = gen.parse_image_csv("# c\nR,r1,b8000\nR,i0,c00f2\nM,95000,1 2 3\nM,95009,a\n")
    assert regs == ["R,r1,b8000", "R,i0,c00f2"]
    assert w == {0x95000: 1, 0x95001: 2, 0x95002: 3, 0x95009: 10}


def test_build_delta_keeps_registers_and_writes_only_what_the_loader_lacks() -> None:
    sram = bytearray(4 * 0x100)
    sram[4 * 0x10 : 4 * 0x10 + 4] = (5).to_bytes(4, "little")
    now = {0x10: 5, 0x11: 7, 0x12: 0}
    lines, keep, n = gen.build_delta(["R,i0,c00f2"], now, bytes(sram), b"")
    assert lines[1:] == ["R,i0,c00f2", "M,11,7"]
    assert keep == [(0x10, 2)]
    assert n == 1
