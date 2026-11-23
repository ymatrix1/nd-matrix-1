"""Tests for jumps / flags (sharc_listing_vm, sharc_listing_ext, sharc_listing_rust) and
organ_farfisa_fw.py."""

from __future__ import annotations

import numpy as np
import organ_farfisa_fw as ff
import sharc_listing_ext as ext
import sharc_listing_rust as slr
import sharc_listing_vm as vm

BASE = 0x100000
LISTING = """\
00000000  0000  f1=pass f0
00000003  0000  if ne jump 0x100008
00000006  0000  r2=1
00000008  0000  r3=2
0000000a  0000  jump 0x10000c
0000000c  0000  rts
"""


def run(r0: int) -> ext.GenVm:
    machine = ext.GenVm({})
    machine.r[0] = machine.s[0] = r0 << 8
    machine.run(vm.parse_listing(LISTING), base=BASE)
    return machine


def test_if_ne_jump_follows_the_az_flag_of_pass() -> None:
    zero = run(0)  # pass 0 sets AZ: `ne` is false, the instruction after the jump runs
    assert zero.az[0] and zero.r[2] >> 8 == 1 and zero.r[3] >> 8 == 2
    one = run(0x3F800000)  # pass 1.0 clears AZ: the jump skips `r2 = 1`
    assert not one.az[0] and one.r[2] >> 8 == 0 and one.r[3] >> 8 == 2


def test_alu_flags_come_from_the_result_and_not_from_the_multiplier() -> None:
    m = ext.GenVm({})
    m.r[1] = m.r[2] = 0x3F800000 << 8  # 1.0
    m.step(vm.Insn(0, "f3=f1-f2", ["f3=f1-f2"]))
    assert m.az[0] and not m.an[0]
    m.r[2] = 0x40000000 << 8  # 2.0: 1 - 2 is negative
    m.step(vm.Insn(0, "f3=f1-f2", ["f3=f1-f2"]))
    assert m.an[0] and not m.az[0]
    m.step(vm.Insn(0, "f3=f1*f2", ["f3=f1*f2"]))  # the multiplier leaves AZ / AN alone
    assert m.an[0] and not m.az[0]


def test_conditions_other_than_ne_eq_are_rejected() -> None:
    try:
        ext.GenVm({}).cond_ok("lt")
    except ValueError:
        return
    raise AssertionError("lt is not a jump condition of the model")


def test_decode_resolves_forward_jumps_to_row_indices() -> None:
    rows = slr.decode(vm.parse_listing(LISTING), BASE)
    jumps = [(k, r) for k, r in enumerate(rows) if r[0] == slr.INS and r[3]]
    assert [r[3] for _k, r in jumps] == [2, 1]  # `if ne jump`, `jump`
    ne_row = jumps[0][1]
    assert rows[ne_row[4]][0] == slr.INS and ne_row[4] > jumps[0][0]
    # the target is the first row of `r3 = 2`: an INS row followed by a `MOVE_DI` item
    assert rows[ne_row[4] + 1][0] == slr.MOVE_DI and rows[ne_row[4] + 1][2] == 2


def test_hann_lines_find_the_strongest_peaks_in_hz() -> None:
    n = 8192 + ff.SKIP
    k = 400  # an exact bin of the 8192 point DFT
    hz = k * ff.RATE / 8192
    x = 0.25 * np.sin(2 * np.pi * k * np.arange(n) / 8192)
    lines = ff.hann_lines(x, 3, ff.SKIP)
    top = max(lines, key=lambda t: t[1])
    assert abs(top[0] - hz) < 1e-6 and abs(top[1] - 0.25) < 1e-3


def test_kept_words_are_the_non_zero_words_of_the_machine_ranges() -> None:
    mem = {0x95FFC: 5, 0x95FFD: 0, 0x1234: 7, 0xC4001: 9}
    assert ff.kept(mem) == [(0x95FFC, 5), (0xC4001, 9)]


def test_static_words_read_the_kept_ranges_of_a_flat_sram_image() -> None:
    sram = bytearray(4 * 0xC4010)
    sram[4 * 0x95FFC : 4 * 0x95FFC + 4] = (5).to_bytes(4, "little")
    sram[4 * 0x1234 : 4 * 0x1234 + 4] = (7).to_bytes(4, "little")  # outside the kept ranges
    sram[4 * 0xC4001 : 4 * 0xC4001 + 4] = (9).to_bytes(4, "little")
    assert ff.static_words(bytes(sram)) == {0x95FFC: 5, 0xC4001: 9}


def test_install_delta_lists_changed_new_and_cleared_words() -> None:
    static = {0x95FFC: 5, 0xC27E0: 0x3F800000, 0xC27E1: 1}
    oracle = {0x95FFC: 5, 0xC27E0: 0, 0xC27E1: 1, 0xC27E2: 8, 0xC4001: 0, 0x1234: 7}
    # unchanged words and words outside the kept ranges are not in the delta; a clear is a 0
    assert ff.install_delta(oracle, static) == [(0xC27E0, 0), (0xC27E2, 8)]
