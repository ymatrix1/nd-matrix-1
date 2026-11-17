"""Unit tests for tools/sample_stream_audio.py (metrics helpers; the run itself needs firmware)."""

from __future__ import annotations

import math
import struct
from pathlib import Path

import numpy as np
import pytest
import sample_stream_audio as a


def tone(f: float, rate: float, secs: float, decay: float = 0.0) -> np.ndarray:
    t = np.arange(int(rate * secs)) / rate
    return np.sin(2 * math.pi * f * t) * np.exp(-decay * t)


def test_entry_and_member_helpers() -> None:
    assert a.entry_of("Bank_A__NS4_Palette") == "Program/Bank A/NS4 Palette.ns4p"
    assert a.first_member({"synth_a": None, "synth_b": "b", "synth_c": "c"}) == "b"
    assert a.first_member({"synth_a": None}) is None
    assert a.expected_hz(69) == pytest.approx(440.0)
    assert a.expected_hz(60) == pytest.approx(261.6256, abs=1e-3)


def test_f0_and_centroid_of_a_sine() -> None:
    x = tone(261.63, a.SY_RATE, 0.5)
    assert a.f0_autocorr(x, a.SY_RATE) == pytest.approx(261.63, rel=0.01)
    assert a.centroid_hz(x, a.SY_RATE) == pytest.approx(261.63, rel=0.05)
    assert a.f0_autocorr(np.zeros(4096), a.SY_RATE) is None
    assert a.centroid_hz(np.zeros(4096), a.SY_RATE) is None
    assert a.centroid_hz(np.ones(10), a.SY_RATE) is None


def test_metrics_of_a_decaying_note() -> None:
    x = 0.25 * tone(261.63, a.SY_RATE, 1.5, decay=3.0)
    m = a.audio_metrics(x, a.SY_RATE)
    assert m["nonsilent"] and m["peak"] == pytest.approx(0.25, rel=0.02)
    assert abs(m["cents"]) < 25  # type: ignore[arg-type]
    assert 1.0 < m["duration_s"] < 1.5  # type: ignore[operator]  # 60 dB at decay 3: 2.3 s, clipped by 1.5 s
    assert m["rms_dbfs"] < m["peak_dbfs"]  # type: ignore[operator]


def test_metrics_of_silence() -> None:
    m = a.audio_metrics(np.zeros(1000), a.SY_RATE)
    assert m["nonsilent"] is False and m["f0_hz"] is None and m["duration_s"] == 0.0


def test_read_wav_f32_mono_mix(tmp_path: Path) -> None:
    pcm = np.array([[1.0, 0.0], [0.5, 0.5]], dtype="<f4").tobytes()
    head = (
        b"RIFF"
        + struct.pack("<I", 36 + len(pcm))
        + b"WAVEfmt "
        + struct.pack("<IHHIIHH", 16, 3, 2, 48000, 384000, 8, 32)
    )
    (tmp_path / "w.wav").write_bytes(head + b"data" + struct.pack("<I", len(pcm)) + pcm)
    x, rate = a.read_wav_f32(tmp_path / "w.wav")
    assert rate == 48000 and list(x) == [0.5, 0.5]


def test_mono_mix_and_arm_order_and_pad_stats() -> None:
    assert list(a.mono_of({"L": [1.0, 0.0], "R": [0.0, 1.0]})) == [0.5, 0.5]
    rows = [
        {"chip": "SY", "path": 1, "dm": 1, "w2": 2},
        {"chip": "SMP", "path": 1, "dm": 3, "w2": 4},
        {"chip": "SY", "path": 2, "dm": 5, "w2": 6},
    ]
    assert a.scn_arm_order(rows) == [(1, 2)]
    assert a.pad_stats([0.0, 1.0, 2.0]) == {"ring_nonzero": 2, "ring_len": 3}


def test_trap_summary() -> None:
    err = "interrupt 17 not finished within 400000 steps (pc 0x1c12dd, last pcs 1 2)"
    assert a.trap_summary(err) == "interrupt 17 trap at pc 0x1c12dd"
    assert a.trap_summary("") == "no output"
    assert a.trap_summary("bad scenario" + chr(10) + "x" * 300) == "x" * 120


def test_step_ratio_and_prepare_sent() -> None:
    word = struct.unpack("<f", struct.pack("<I", 0x0CE70000))[0]
    assert a.step_ratio(word) == pytest.approx(0x0CE70000 / 2.0**24)
    rows = [{"chip": "SY", "path": 1, "dm": 0x92414, "w2": 0x1C0794}]
    assert not a.prepare_sent(rows)
    assert a.prepare_sent([*rows, {"chip": "SY", "path": 1, "dm": 0x92414, "w2": 0x1C07CD}])


def test_pitch_shift_cents_finds_a_known_shift() -> None:
    rate = 48000.0
    t = np.arange(int(rate * 1.5)) / rate
    rng = np.random.default_rng(1)
    amps = rng.uniform(0.2, 1.0, 12)
    sig = lambda f0: sum(a_ * np.sin(2 * math.pi * f0 * (k + 1) * t) for k, a_ in enumerate(amps))  # noqa: E731
    base = sig(261.63)
    up = sig(261.63 * 2 ** (300 / 1200))
    got = a.pitch_shift_cents(up, rate, base, rate)
    assert got == pytest.approx(300, abs=50)
    assert a.pitch_shift_cents(np.zeros(100000), rate, base, rate) is None
