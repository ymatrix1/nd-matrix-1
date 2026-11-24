"""Tests for sharc_listing_rust.py, organ_bass_fw_gen.py (mem_runs) and organ_bass_voice.py."""

from __future__ import annotations

from pathlib import Path

import organ_bass_fw_gen as gen
import organ_bass_voice as voice
import sharc_listing_rust as slr
import sharc_listing_vm as vm

LISTING = """\
00000000  0000  i2=dm(0xffffffe0,i1)
00000003  0000  lcntr=0x3, do (pc,0xff800005) until lce
00000006  0000  r4=dm(i3,m2), r0=pm(i12,m10)
00000008  0000  f13=f1*f6, f9=f9+f13, r1=dm(i1,m2)
0000000c  0000  dm(0xb33c0)=i0
0000000e  0000  if lt r5=r5+r6
00000010  0000  r8=fext r14 by 0x19:0x7 (se)
00000012  0000  MRF=r0*r2(ssf)
00000014  0afe  rts (db)
"""


def rows() -> list[list[int]]:
    return slr.decode(vm.parse_listing(LISTING))


def test_loads_use_the_broadcast_flag_and_the_address_kinds() -> None:
    r = rows()
    assert r[0] == [slr.INS, 1, 0, 0, 0, 0, 0, 0]
    # `i2=dm(0xffffffe0,i1)`: pre modify by an immediate through I1 (broadcast), register i2 = 34
    assert r[1] == [slr.LOAD, 34, 4, 1, -32, 2, 0, 0]


def test_loops_carry_their_body_length() -> None:
    r = rows()
    loop = next(i for i, x in enumerate(r) if x[0] == slr.LOOP)
    assert r[loop][1] == 3
    body = r[loop + 1 : loop + 1 + r[loop][2]]
    assert sum(1 for x in body if x[0] == slr.INS) == 2  # the two instructions of the loop


def test_alu_forms_and_conditional_execution_are_decoded() -> None:
    r = rows()
    mul = next(x for x in r if x[0] == slr.ALU and x[2] == slr.FMUL)
    assert mul[3:5] == [1, 6]
    cond = [x for x in r if x[0] == slr.INS and x[2] == 1]
    assert len(cond) == 1
    fext = next(x for x in r if x[0] == slr.ALU and x[2] == slr.FEXT_I)
    assert fext[3:7] == [14, 0x19, 0x7, 1]  # source, pos, length, sign extend
    assert any(x[0] == slr.MRF_SET for x in r)
    assert not any(x[0] == slr.LOAD and x[1] > 96 for x in r)  # nothing after the rts


def test_memory_runs_merge_consecutive_words() -> None:
    lines = gen.mem_runs([(0x10, 1), (0x11, 2), (0x20, 0xFF)])
    assert lines == ["M,10,1 2", "M,20,ff"]


def test_voice_slot_words_take_the_last_write_of_each_address() -> None:
    writes = []
    for base, n in voice.SLOT:
        writes += [(base + n * 2 + k, 0x100 + k) for k in range(n)]
    writes.append((voice.SLOT[0][0] + 40, 0x999))  # rewrite of the first word of slot 2
    words = voice.slot_words(writes, 2)
    assert words is not None and len(words) == 36 and words[0] == 0x999
    assert voice.slot_words(writes, 3) is None


def test_voice_table_relocates_first_five_presses(tmp_path: Path) -> None:
    lines = ["press,key,item,addr,value"]
    for press, key in ((1, 60), (7, 61)):  # press 7 is beyond the slot layout and ignored
        for base, n in voice.SLOT:
            lines += [f"{press},{key},0,{base + n * (press % 5) + k:x},{k + 1:x}" for k in range(n)]
    path = tmp_path / "cap.csv"
    path.write_text("\n".join(lines) + "\n")
    table = voice.build_table([path])
    assert list(table) == [60] and table[60][0] == 1 and len(table[60]) == 36
    assert voice.key_list("3:1") == [3, 2, 1] and voice.key_list("60,64") == [60, 64]
