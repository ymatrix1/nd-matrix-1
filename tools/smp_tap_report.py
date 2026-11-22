#!/usr/bin/env python3
"""Pitch, interrupt rate and level of the SMP piano tap (OQ41, D206).

`arm_dsp_live --piano-bind --chip SMP:0=smp=IMAGE` captures the post stage ring of the SMP chip,
one stereo frame per audio interrupt (`run/smp_tap.f32`, f32 little endian `L R`). This tool turns
that tap into two numbers the harness could not give before:

* `rate`: the interrupt rate the tap implies. The tap pitch in cycles per frame is the pitch of the
  reference decode of the stroke (`smp_stroke_dump`, cycles per decoded sample) times the step word
  the ARM wrote (`0x9251D`, fraction of a sample per interrupt); Hz needs the interrupt rate. The
  report lists the candidate rates (DSP frame rate 43,402.34 Hz, 44.1 kHz, 48 kHz) as cents against
  the note and the rate that gives the note exactly.
* `level`: RMS of the tap against a native render (`ns4 render-patch --wav`, f32 stereo, 48 kHz)
  over the same time window from the note onset, with the receiver scale of the FX path
  (`0.3732` per ring unit, `docs/specs/piano-engine.md`) applied optionally.

    python tools/smp_tap_report.py rate run/smp_tap.f32 stroke513.f32 --step 0.7982983589
    python tools/smp_tap_report.py level run/smp_tap.f32 native.wav --window-ms 69
"""

from __future__ import annotations

import argparse
import json
import math
import statistics
import struct
import sys
from collections.abc import Sequence
from pathlib import Path

import numpy as np

# VERIFIED (docs/research/dsp-frame-rate.md s3.1): the ARM literal `0x47298A58` = 44,444,000 / 1024.
FRAME_RATE_HZ = 44_444_000 / 1024
# VERIFIED (`piano-engine.md`, D39 receiver path): ring value to DAC fraction, `2 * 0.1866`.
RING_TO_DAC = 0.3732
C4_HZ = 440.0 * 2 ** (-9 / 12)
CANDIDATE_RATES = {"dsp_frame_rate": FRAME_RATE_HZ, "44.1k": 44_100.0, "48k": 48_000.0}


def read_frames(data: bytes) -> np.ndarray:
    """`(n, 2)` floats of an f32 little endian `L R` capture (a partial last frame is dropped)."""
    n = len(data) // 8
    return np.frombuffer(data[: 8 * n], dtype="<f4").reshape(n, 2).astype(float)


def read_wav_f32(data: bytes) -> tuple[int, np.ndarray]:
    """`(sample rate, (n, 2) floats)` of a 32 bit float stereo WAV (`ns4 render-patch --wav`)."""
    if data[:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise ValueError("not a RIFF/WAVE file")
    pos, rate, fmt = 12, 0, None
    while pos + 8 <= len(data):
        tag, size = data[pos : pos + 4], struct.unpack_from("<I", data, pos + 4)[0]
        body = data[pos + 8 : pos + 8 + size]
        if tag == b"fmt ":
            fmt, channels, rate = struct.unpack_from("<HHI", body)
            if fmt not in (3, 0xFFFE) or channels != 2:
                raise ValueError(f"need f32 stereo, got format {fmt} channels {channels}")
        elif tag == b"data":
            return rate, read_frames(body)
        pos += 8 + size + (size & 1)
    raise ValueError("no data chunk")


def fundamental(
    x: np.ndarray, lo: float, hi: float, harmonics: int = 6, size: int = 1 << 20
) -> float:
    """Fundamental in cycles per sample between `lo` and `hi` (harmonic-sum of a Hann spectrum).

    A coarse grid of 4,000 points over `[lo, hi]` picks the cell, a second grid of 2,001 points
    inside it refines (resolution about `(hi - lo) / 8e6`).
    """
    x = np.asarray(x, dtype=float)
    x = x - x.mean()
    mag = np.abs(np.fft.rfft(x * np.hanning(len(x)), n=size))
    bins = np.arange(len(mag))

    def score(freqs: np.ndarray) -> np.ndarray:
        return sum(np.interp(k * freqs * size, bins, mag) for k in range(1, harmonics + 1))

    grid = np.linspace(lo, hi, 4000)
    i = int(np.argmax(score(grid)))
    fine = np.linspace(grid[max(i - 1, 0)], grid[min(i + 1, len(grid) - 1)], 2001)
    return float(fine[int(np.argmax(score(fine)))])


def cents(ratio: float) -> float:
    return 1200.0 * math.log2(ratio)


def rate_report(
    tap_cpf: float,
    ref_cps: float,
    step: float,
    note_hz: float = C4_HZ,
    rates: dict[str, float] | None = None,
) -> dict[str, object]:
    """Interrupt rate implied by a tap.

    `tap_cpf`: tap cycles per interrupt frame, `ref_cps`: cycles per decoded sample of the stroke,
    `step`: samples per interrupt the ARM wrote. `consistency` is the tap pitch over
    `ref_cps * step` (1.0 when the voice resamples the decode at exactly the step); `rate_for_note`
    is the interrupt rate at which the tap sounds `note_hz`.
    """
    rates = rates or CANDIDATE_RATES
    return {
        "tap_cycles_per_frame": tap_cpf,
        "reference_cycles_per_sample": ref_cps,
        "step": step,
        "consistency": tap_cpf / (ref_cps * step),
        "rate_for_note": note_hz / tap_cpf,
        "candidates": {
            name: {"hz": tap_cpf * r, "cents_vs_note": cents(tap_cpf * r / note_hz)}
            for name, r in rates.items()
        },
    }


def rms_db(x: np.ndarray) -> float:
    """RMS of a mono or stereo array in dB (mean of the squares over all samples)."""
    p = float(np.mean(np.square(x))) if x.size else 0.0
    return 10 * math.log10(p) if p > 0 else float("-inf")


def onset(x: np.ndarray, fraction: float = 0.01) -> int:
    """First frame whose `max(|L|, |R|)` reaches `fraction` of the capture peak (0 for silence)."""
    m = np.max(np.abs(x), axis=1) if x.ndim == 2 else np.abs(x)
    peak = float(m.max()) if m.size else 0.0
    return int(np.argmax(m >= fraction * peak)) if peak > 0 else 0


def level_report(
    tap: np.ndarray,
    tap_rate: float,
    native: np.ndarray,
    native_rate: float,
    window_ms: float,
    scale: float = 1.0,
) -> dict[str, float]:
    """RMS of `tap * scale` and `native` over `window_ms` from each onset, and the difference in dB.

    The onset of each signal is its first frame at 1 % of its own peak, so the silent lead of the
    capture (the ARM start-up) does not count; both windows cover the same milliseconds.
    """
    t0, n0 = onset(tap), onset(native)
    t_win = tap[t0 : t0 + int(window_ms * tap_rate / 1000)] * scale
    n_win = native[n0 : n0 + int(window_ms * native_rate / 1000)]
    t_db, n_db = rms_db(t_win), rms_db(n_win)
    return {
        "tap_rms_db": t_db,
        "native_rms_db": n_db,
        "tap_minus_native_db": t_db - n_db,
        "tap_peak": float(np.abs(t_win).max()) if t_win.size else 0.0,
        "native_peak": float(np.abs(n_win).max()) if n_win.size else 0.0,
    }


def class_table(rows: Sequence[tuple[str, float]]) -> dict[str, dict[str, float]]:
    """`{class: {n, min, median, max}}` of `(class, tap minus native dB)` rows, classes sorted."""
    by: dict[str, list[float]] = {}
    for name, delta in rows:
        by.setdefault(name, []).append(delta)
    return {
        k: {"n": len(v), "min": min(v), "median": statistics.median(v), "max": max(v)}
        for k, v in sorted(by.items())
    }


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - thin CLI over tested functions
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("rate", help="interrupt rate implied by a tap")
    r.add_argument("tap", type=Path)
    r.add_argument("reference", type=Path, help="smp_stroke_dump output (f32 L R)")
    r.add_argument("--step", type=float, required=True, help="step word / 2^32 (message 0x9251D)")
    r.add_argument("--note-hz", type=float, default=C4_HZ)
    lv = sub.add_parser("level", help="tap RMS against a native render")
    lv.add_argument("tap", type=Path)
    lv.add_argument("native", type=Path, help="ns4 render-patch --wav output")
    lv.add_argument("--window-ms", type=float, default=69.0)
    lv.add_argument("--receiver-scale", action="store_true", help=f"apply {RING_TO_DAC} to the tap")
    ns = ap.parse_args(argv)
    tap = read_frames(ns.tap.read_bytes())
    if ns.cmd == "rate":
        ref = read_frames(ns.reference.read_bytes())
        start = int(np.argmax(np.abs(tap[:, 0])))
        tap_cpf = fundamental(tap[start : start + 8192, 0], 0.004, 0.009)
        ref_cps = fundamental(ref[:40000, 0], 0.005, 0.011)
        out = rate_report(tap_cpf, ref_cps, ns.step, ns.note_hz)
    else:
        rate, native = read_wav_f32(ns.native.read_bytes())
        scale = RING_TO_DAC if ns.receiver_scale else 1.0
        out = level_report(tap, FRAME_RATE_HZ, native, float(rate), ns.window_ms, scale)
    print(json.dumps(out, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
