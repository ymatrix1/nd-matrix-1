"""Tests of tools/fx_code_presence.py on synthetic byte images (no firmware needed)."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import fx_code_presence as fp  # noqa: E402


def image(size: int, seed: int) -> bytearray:
    """Deterministic pseudo-random bytes (xorshift), so pieces are unique."""
    out = bytearray()
    x = seed
    for _ in range(size):
        x ^= (x << 13) & 0xFFFFFFFF
        x ^= x >> 17
        x ^= (x << 5) & 0xFFFFFFFF
        out.append(x & 0xFF)
    return out


def test_pieces_step_two_parcels():
    ref = bytes(range(32))
    ps = fp.pieces(ref, 4, length=16, width=8)  # pc 4 -> byte 8
    assert ps[0] == bytes(range(8, 16))
    assert ps[1] == bytes(range(10, 18))
    assert len(ps) == (16 - 8) // 2 + 1


def test_copy_at_another_address_is_present():
    ref = bytes(image(4096, 1))
    target = bytes(image(512, 2)) + ref[200 : 200 + 0x300] + bytes(image(512, 3))
    assert fp.presence(ref, 100, target) == 1.0  # pc 100 -> byte 200


def test_absent_routine_scores_zero():
    ref = bytes(image(4096, 1))
    target = bytes(image(4096, 9))
    assert fp.presence(ref, 100, target) == 0.0


def test_patched_operand_only_breaks_its_pieces():
    ref = bytes(image(4096, 1))
    copy = bytearray(ref[200 : 200 + 0x300])
    copy[0x100:0x104] = b"\xff\xff\xff\xff"
    frac = fp.presence(ref, 100, bytes(copy))
    assert 0.9 < frac < 1.0


def test_empty_window_is_zero_and_parse_routine():
    assert fp.presence(b"", 0, b"abc") == 0.0
    assert fp.parse_routine("0x1256AD:leslie") == (0x1256AD, "leslie")
    assert fp.parse_routine("10") == (10, "10")


def test_rows_cover_every_pair():
    ref = bytes(image(4096, 1))
    out = list(fp.rows(ref, [(100, "a"), (200, "b")], {"x": ref, "y": b""}))
    assert [(n, t) for n, t, _ in out] == [("a", "x"), ("a", "y"), ("b", "x"), ("b", "y")]
