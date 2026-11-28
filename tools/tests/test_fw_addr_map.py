import os
import struct
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import fw_addr_map as m  # noqa: E402

BASE = m.LOAD_BASE


def image(ws: list[int]) -> bytes:
    return struct.pack(f"<{len(ws)}I", *ws)


def filler(n: int, seed: int) -> list[int]:
    """Deterministic non-branch words (top nibble 0xE, bits 27-25 never 101)."""
    return [0xE1A0_0000 | ((seed * 2654435761 + i * 40503) & 0xFFFF) for i in range(n)]


def bl(offset: int) -> int:
    return 0xEB00_0000 | (offset & 0x00FF_FFFF)


FUNC = [0xE92D_4010, *filler(10, 7), bl(0x1234), *filler(6, 9), 0xE8BD_8010]


def test_mask_clears_only_the_branch_offset():
    assert m.mask(bl(0x1234)) == 0xEB00_0000
    assert m.mask(0xEA00_0005) == 0xEA00_0000  # B
    assert m.mask(0xE1A0_0001) == 0xE1A0_0001  # mov, untouched
    assert m.mask(0xE59F_0010) == 0xE59F_0010  # ldr, untouched
    assert m.mask(0xC055_B150) == m.ANY_ADDRESS  # literal pool address
    assert m.mask(0x8000_1234) == m.ANY_ADDRESS  # SRAM address
    assert m.mask(0x6000_0015) == 0x6000_0015  # device address, a real constant


def test_a_moved_function_with_a_different_call_offset_is_found():
    old = image(filler(40, 1) + FUNC + filler(40, 2))
    moved = [w if w != bl(0x1234) else bl(0x9999) for w in FUNC]
    new = image(filler(40, 1) + filler(3, 5) + moved + filler(40, 2))
    a = BASE + 4 * 40
    r = m.map_address(m.Image(old), m.Image(new), a)
    assert r == ("code", BASE + 4 * 43, 8)


def test_an_address_with_no_equivalent_is_unmapped():
    old = image(filler(40, 1) + FUNC)
    new = image(filler(40, 3))
    assert m.map_address(m.Image(old), m.Image(new), BASE + 4 * 40) is None


def test_repeated_stubs_are_told_apart_by_where_the_neighbours_moved():
    stub = [0xE3E0_2000, 0xE1A0_1002, 0xE92D_4010, bl(1), *filler(8, 4)] + filler(300, 13)
    anchor = filler(64, 11)
    old = image(anchor + stub + filler(70, 12) + stub)
    new = image(filler(2, 8) + anchor + stub + filler(70, 12) + stub)
    first = BASE + 4 * 64
    got = m.map_code(m.Image(old), m.Image(new), first)
    assert got == (BASE + 4 * 66, 0)


def test_a_data_cell_is_followed_through_the_ldr_that_loads_it():
    cell_old, cell_new = 0xC055_B150, 0xC055_B4F0
    ldr = 0xE59F_0000  # ldr r0, [pc, #0] -> literal two words after the ldr
    code = [*filler(4, 1), 0xE92D_4010, ldr, 0xE1A0_0000, 0, *filler(30, 2)]
    old_ws, new_ws = list(code), list(code)
    old_ws[7] = cell_old
    new_ws[7] = cell_new
    new_ws = filler(3, 6) + new_ws
    r = m.map_address(m.Image(image(old_ws)), m.Image(image(new_ws)), cell_old)
    assert r is not None and r[:2] == ("data", cell_new)


def test_main_prints_old_and_new_addresses(tmp_path, capsys):
    old = tmp_path / "old.bin"
    new = tmp_path / "new.bin"
    old.write_bytes(image(filler(20, 1) + FUNC))
    new.write_bytes(image(filler(2, 5) + filler(20, 1) + FUNC))
    rc = m.main([str(old), str(new), f"{BASE + 80:X}", "C0000FF0"])
    out = capsys.readouterr().out
    assert rc == 0
    assert f"0x{BASE + 80:08X} -> 0x{BASE + 88:08X} (code" in out
    assert "0xC0000FF0 -> unmapped" in out
