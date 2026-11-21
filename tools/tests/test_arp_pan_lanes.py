"""Tests of `arp_pan_lanes` (D175): pan to bank bits, key word, lane peaks of a tap file."""

import array
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import arm_key_words as kw  # noqa: E402
import arp_pan_lanes as pl  # noqa: E402


def test_bank_bits_are_three_minus_the_pan_code() -> None:
    assert [pl.banks_of_pan(c) for c in "CLR"] == [3, 2, 1]
    assert [pl.pan_of_banks(b) for b in (3, 2, 1, 0)] == ["C", "L", "R", None]


def test_synth_engine_step_words_decode_to_pan() -> None:
    # the first steps of Synth Engine (pattern pan `CRLCRC..`, ARM steps 1.. = R, L, C, R)
    words = [0x0100E200, 0x0200E200, 0x0300E200]
    assert [pl.pan_of_banks(int(kw.decode(w)["banks"])) for w in words] == ["R", "L", "C"]
    assert [pl.key_word(pl.banks_of_pan(c)) for c in "RLC"] == words


def test_lane_peaks_split_even_and_odd_words() -> None:
    words = array.array("f", [0.5, 0.0, -0.25, 0.1, 0.0, 0.75, float("nan"), 2e9])
    assert pl.lane_peaks(words.tobytes()) == (0.5, 0.75)
    assert pl.lane_peaks(b"") == (0.0, 0.0)
