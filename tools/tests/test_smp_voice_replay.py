"""Tests of `smp_voice_replay`: page decode, history ring reading, ARM script parsing."""

import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import smp_voice_replay as sv  # noqa: E402


def pack_page(res_left: list[int], res_right: list[int], width: int, order: int) -> bytes:
    """Stereo page (interleaved L R residuals, LSB first 16 bit big endian words)."""
    bits = 0
    for n, (a, b) in enumerate(zip(res_left, res_right, strict=True)):
        bits |= (a & ((1 << width) - 1)) << (2 * width * n)
        bits |= (b & ((1 << width) - 1)) << (2 * width * n + width)
    words = [(bits >> (16 * k)) & 0xFFFF for k in range(1021)]
    return bytes([1, order << 5 | width]) + b"".join(struct.pack(">H", w) for w in words)


def test_residuals_are_signed_fields_of_the_word_stream() -> None:
    words = [0xCB82, 0x6864, 0x0341]
    assert sv.page_residual(words, 0, 9, 18) == -126  # the Astoria Grand stroke 513 start
    assert sv.page_residual(words, 1, 9, 18) == 25  # bits 18..26: 0x341 << 14 | 0x6864 >> 2


def test_order_three_decode_integrates_the_residuals_and_right_lane_uses_base() -> None:
    left, right = [-126, 25, 52, -43, -1, 90], [10, -3, 4, 0, 1, -2]
    page = pack_page(left, right, 9, 3)
    got = sv.decode_page(page, [0, -41, -57, -9], 6)
    hist, want = [0, -41, -57, -9], []
    for r in left:
        y = hist[1] - 3 * hist[2] + 3 * hist[3] + r
        want.append(y)
        hist = [*hist[1:], y]
    assert got == want and got[0] == -23  # matches the reference decode of the stroke start
    assert sv.decode_page(page, [0, 0, 0, 0], 2, channel=1)[0] == 10


def test_history_ring_holds_y_n_at_slot_two_times_n_plus_four_mod_eight() -> None:
    ring = [0] * 16
    for n in range(8, 16):
        ring[2 * ((n + 4) & 7)] = n * 100 if n != 9 else (1 << 32) - 5
    assert sv.history_samples(ring, 16) == [800, -5, 1000, 1100, 1200, 1300, 1400, 1500]


def test_script_and_start_history_follow_the_arm_words(tmp_path: Path) -> None:
    f = tmp_path / "SMP0.script"
    lines = ["814 0xc0502 0xffffffd7", "814 0xc0500 0x00000000", "814 0xc0504 0xffffffc7"]
    lines += ["815 0xc0506 0xfffffff7", "816 0xc0500 0x5"]
    f.write_text("\n".join(lines) + "\n")
    msgs = sv.read_script(f)
    assert msgs[815] == [(0xC0506, 0xFFFFFFF7)] and len(msgs) == 3
    assert sv.init_history(msgs) == [0, -41, -57, -9]


def test_voice_addresses_step_by_the_record_and_history_strides() -> None:
    assert sv.voice_addresses(0) == (0x93928, 0xC0500)
    assert sv.voice_addresses(1) == (0x9395E, 0xC0510)  # second voice of the Astoria Grand note-on
    assert sv.voice_addresses(17)[0] == 0x93CBE  # the release voice start of the same program


def test_init_history_reads_the_base_of_the_voice() -> None:
    msgs = {7: [(0xC0510 + k, (k + 1) * 10) for k in range(8)]}  # even slots left, odd right
    assert sv.init_history(msgs, base=0xC0510) == [10, 30, 50, 70]
    assert sv.init_history(msgs, lane=1, base=0xC0510) == [20, 40, 60, 80]


def test_match_counter_accepts_a_ring_one_sample_ahead_of_the_counter() -> None:
    decoded = list(range(400))  # y(n) = n
    ahead = [0] * 16
    for n in range(380, 388):  # y(n) sits at slot 2 * ((n + 4) & 7): samples 380..387 stored
        ahead[2 * ((n + 4) & 7)] = n
    assert sv.match_counter(ahead, 387, decoded) == 388  # the counter word lags by one
    assert sv.match_counter(ahead, 388, decoded) == 388
    assert sv.match_counter(ahead, 300, decoded) is None


def test_order_four_row_is_the_flac_fixed_polynomial_of_the_release_strokes() -> None:
    left, right = [3, -2, 5, 1, -4, 2], [0] * 6
    page = pack_page(left, right, 4, 4)
    got = sv.decode_page(page, [0, 2, 4, 4], 6)
    hist, want = [0, 2, 4, 4], []
    for r in left:
        y = -hist[0] + 4 * hist[1] - 6 * hist[2] + 4 * hist[3] + r
        want.append(y)
        hist = [*hist[1:], y]
    assert got == want and sv.LPC[4] == (-1, 4, -6, 4)
