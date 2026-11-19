#!/usr/bin/env python3
"""Step period of an arpeggiator in a hardware recording, measured from the onsets only.

Used for `docs/research/arp-fw-vs-native.md`: factory A:16 `Horizon Whl` in the Part A video
(`.reference/NEW NORD STAGE 4 - PROGRAMAS PRESETS DEMO (PART A) [_zksT69H30c].webm`, program on
screen from 287 s). The audio (never committed) is cut with ffmpeg to a mono WAV; this tool turns
it into an onset-strength curve (log-magnitude spectral flux, 200 Hz to 6 kHz, 2 ms hop) and

* `autocorr`   the periods at which the curve repeats (50 ms to 800 ms), with parabolic peak
               refinement, over one window or a list of windows;
* `comb`       the grid period that best fits the picked onsets (circular resultant length of the
               onset times modulo the period), 0.1 ms steps.

    ffmpeg -ss 285 -t 60 -i VIDEO.webm -vn -ac 1 -ar 48000 clip.wav
    python tools/arp_video_onsets.py clip.wav --t0 285 --window 287:300 --window 300:315 \
        --out docs/research/data/arp-video-onsets.json

Only numbers are written. EMPIRICAL: the flux curve and the peak picking are heuristics checked on
synthetic click trains in `tools/tests/test_arp_video_onsets.py`; real playing adds jitter, so a
period is reported with the autocorrelation height as its strength.
"""

from __future__ import annotations

import argparse
import json
import sys
import wave
from pathlib import Path

import numpy as np

NFFT = 1024
HOP = 96  # 2 ms at 48 kHz
BAND_HZ = (200.0, 6000.0)
SMOOTH_FRAMES = 50


def read_wav_mono(path: Path) -> tuple[int, np.ndarray]:
    """`(sample rate, float samples)` of a 16-bit mono WAV."""
    with wave.open(str(path), "rb") as w:
        if w.getsampwidth() != 2 or w.getnchannels() != 1:
            raise ValueError("expected a 16-bit mono WAV")
        sr = w.getframerate()
        data = np.frombuffer(w.readframes(w.getnframes()), dtype="<i2")
    return sr, data.astype(np.float64) / 32768.0


def onset_flux(x: np.ndarray, sr: int, hop: int = HOP) -> tuple[np.ndarray, float]:
    """Onset strength per hop (positive spectral flux, local mean removed) and the hop (s)."""
    window = np.hanning(NFFT)
    n_frames = 1 + (len(x) - NFFT) // hop
    freqs = np.fft.rfftfreq(NFFT, 1.0 / sr)
    band = (freqs > BAND_HZ[0]) & (freqs < BAND_HZ[1])
    mags = np.empty((n_frames, int(band.sum())))
    for i in range(n_frames):
        spec = np.abs(np.fft.rfft(x[i * hop : i * hop + NFFT] * window))
        mags[i] = np.log1p(spec[band] * 50.0)
    flux = np.maximum(np.diff(mags, axis=0), 0.0).sum(axis=1)
    flux -= np.convolve(flux, np.ones(SMOOTH_FRAMES) / SMOOTH_FRAMES, mode="same")
    return np.maximum(flux, 0.0), hop / sr


def autocorr_peaks(
    flux: np.ndarray, dt: float, lo_s: float = 0.05, hi_s: float = 0.8, height: float = 0.05
) -> list[tuple[float, float]]:
    """`(period ms, normalized autocorrelation)` of every local maximum in `lo_s..hi_s`."""
    seg = flux - flux.mean()
    ac = np.correlate(seg, seg, mode="full")[len(seg) - 1 :]
    if ac[0] <= 0:
        return []
    ac = ac / ac[0]
    lo, hi = max(int(lo_s / dt), 1), min(int(hi_s / dt), len(ac) - 2)
    out = []
    for k in range(lo, hi):
        if ac[k] >= height and ac[k] > ac[k - 1] and ac[k] >= ac[k + 1]:
            y0, y1, y2 = ac[k - 1], ac[k], ac[k + 1]
            denom = y0 - 2.0 * y1 + y2
            shift = 0.5 * (y0 - y2) / denom if denom else 0.0
            out.append(((k + shift) * dt * 1000.0, float(y1)))
    return out


def pick_onsets(
    flux: np.ndarray, dt: float, rel_height: float = 0.25, min_gap_s: float = 0.06
) -> list[float]:
    """Onset times in seconds: local maxima above `rel_height` of the 99th percentile."""
    thr = rel_height * float(np.percentile(flux, 99))
    out: list[float] = []
    for k in range(1, len(flux) - 1):
        if flux[k] >= thr and flux[k] > flux[k - 1] and flux[k] >= flux[k + 1]:
            if not out or k * dt - out[-1] >= min_gap_s:
                out.append(k * dt)
    return out


def comb_period(
    onsets: list[float], lo_ms: float = 100.0, hi_ms: float = 700.0, step_ms: float = 0.1
) -> tuple[float, float]:
    """Grid period (ms) with the highest phase concentration of `onsets`, and that concentration.

    The search starts at 100 ms: a grid of period `p` also fits every `p / n`, so the shortest
    candidate would always win.
    """
    t = np.asarray(onsets)
    best = (0.0, 0.0)
    for period in np.arange(lo_ms, hi_ms, step_ms):
        phase = 2.0 * np.pi * (t * 1000.0 % period) / period
        r = float(np.hypot(np.cos(phase).mean(), np.sin(phase).mean()))
        if r > best[1]:
            best = (float(period), r)
    return best


def analyze(path: Path, t0: float, windows: list[tuple[float, float]]) -> dict:
    """Autocorrelation peaks per window plus the comb fit over all windows."""
    sr, x = read_wav_mono(path)
    flux, dt = onset_flux(x, sr)
    rows = []
    for a, b in windows:
        seg = flux[int((a - t0) / dt) : int((b - t0) / dt)]
        peaks = autocorr_peaks(seg, dt)
        rows.append(
            {"from_s": a, "to_s": b, "peaks": [(round(p, 1), round(h, 3)) for p, h in peaks[:8]]}
        )
    span = flux[int((windows[0][0] - t0) / dt) : int((windows[-1][1] - t0) / dt)]
    onsets = pick_onsets(span, dt)
    period, strength = comb_period(onsets)
    return {
        "wav": path.name,
        "hop_ms": dt * 1000.0,
        "windows": rows,
        "onsets": len(onsets),
        "comb_period_ms": round(period, 1),
        "comb_strength": round(strength, 3),
    }


def parse_window(text: str) -> tuple[float, float]:
    """`"287:300"` to `(287.0, 300.0)`."""
    a, _, b = text.partition(":")
    return float(a), float(b)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("wav", type=Path)
    ap.add_argument("--t0", type=float, default=0.0, help="video time of the first WAV sample (s)")
    ap.add_argument("--window", action="append", type=parse_window, required=True)
    ap.add_argument("--out", type=Path)
    ns = ap.parse_args(argv)
    result = analyze(ns.wav, ns.t0, ns.window)
    text = json.dumps(result, indent=1)
    if ns.out:
        ns.out.write_text(text + "\n", encoding="utf-8")
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
