"""Tests of `smp_tap_report`: pitch, interrupt rate candidates, level windows."""

import math
import struct
import sys
from pathlib import Path

import numpy as np
import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import smp_tap_report as tr  # noqa: E402


def tone(cycles: float, n: int, harmonics: int = 3) -> np.ndarray:
    t = np.arange(n)
    return sum(np.sin(2 * np.pi * cycles * k * t) / k for k in range(1, harmonics + 1))


def test_frame_rate_is_the_arm_literal() -> None:
    assert tr.FRAME_RATE_HZ == pytest.approx(43402.34375)
    assert tr.C4_HZ == pytest.approx(261.6256, abs=1e-3)


def test_fundamental_finds_a_harmonic_series_within_a_tenth_of_a_cent() -> None:
    x = tone(0.006030, 8192)
    assert tr.fundamental(x, 0.004, 0.009) == pytest.approx(0.006030, abs=5e-7)
    assert tr.fundamental(tone(0.00755, 20000), 0.005, 0.011) == pytest.approx(0.00755, abs=5e-7)


def test_rate_report_candidates_and_the_rate_that_gives_the_note() -> None:
    # Astoria Grand (D206): tap 0.0060301 cycles/frame, reference 0.0075510, step 0.79830
    rep = tr.rate_report(0.0060301, 0.0075510, 0.7982984)
    assert rep["consistency"] == pytest.approx(1.0, abs=5e-4)
    cand = rep["candidates"]
    assert abs(cand["dsp_frame_rate"]["cents_vs_note"]) < 1.0
    assert cand["44.1k"]["cents_vs_note"] == pytest.approx(28.2, abs=0.5)
    assert cand["48k"]["cents_vs_note"] == pytest.approx(175.0, abs=1.0)
    assert rep["rate_for_note"] == pytest.approx(43387, abs=3)


def test_read_frames_drops_a_partial_frame() -> None:
    data = struct.pack("<5f", 1, 2, 3, 4, 5)
    assert tr.read_frames(data).tolist() == [[1.0, 2.0], [3.0, 4.0]]
    assert tr.read_frames(b"").shape == (0, 2)


def wav_f32(frames: np.ndarray, rate: int) -> bytes:
    pcm = frames.astype("<f4").tobytes()
    fmt = struct.pack("<HHIIHH", 3, 2, rate, rate * 8, 8, 32)
    body = (
        b"WAVE"
        + b"fmt "
        + struct.pack("<I", 16)
        + fmt
        + b"data"
        + struct.pack("<I", len(pcm))
        + pcm
    )
    return b"RIFF" + struct.pack("<I", len(body)) + body


def test_read_wav_f32_round_trip_and_rejections() -> None:
    frames = np.array([[0.5, -0.5], [0.25, 0.0]])
    rate, got = tr.read_wav_f32(wav_f32(frames, 48000))
    assert rate == 48000 and got.tolist() == frames.tolist()
    with pytest.raises(ValueError):
        tr.read_wav_f32(b"nope" * 8)
    with pytest.raises(ValueError):
        tr.read_wav_f32(
            b"RIFF\0\0\0\0WAVE" + b"fmt " + struct.pack("<IHHIIHH", 16, 1, 2, 48000, 0, 4, 16)
        )


def test_rms_db_and_onset() -> None:
    assert tr.rms_db(np.ones(10)) == pytest.approx(0.0)
    assert tr.rms_db(np.full(10, 0.1)) == pytest.approx(-20.0)
    assert tr.rms_db(np.zeros(4)) == -math.inf
    x = np.zeros((100, 2))
    x[40:, 0] = 1.0
    assert tr.onset(x) == 40
    assert tr.onset(np.zeros((5, 2))) == 0


def test_level_report_aligns_onsets_and_windows_in_milliseconds() -> None:
    tap = np.zeros((10000, 2))
    tap[500:5000] = 0.1  # onset after a silent lead
    native = np.zeros((20000, 2))
    native[1200:9000] = 1.0
    rep = tr.level_report(tap, 43402.34375, native, 48000.0, 50.0)
    assert rep["tap_minus_native_db"] == pytest.approx(-20.0, abs=1e-9)
    assert rep["tap_peak"] == pytest.approx(0.1) and rep["native_peak"] == 1.0
    scaled = tr.level_report(tap, 43402.34375, native, 48000.0, 50.0, scale=tr.RING_TO_DAC)
    assert scaled["tap_minus_native_db"] == pytest.approx(
        -20.0 + 20 * math.log10(tr.RING_TO_DAC), abs=1e-9
    )


def test_class_table_orders_classes_and_takes_the_median() -> None:
    table = tr.class_table([("grand", -10.0), ("clav", -6.9), ("grand", -13.3), ("grand", -9.1)])
    assert list(table) == ["clav", "grand"]
    assert table["grand"] == {"n": 3, "min": -13.3, "median": -10.0, "max": -9.1}
