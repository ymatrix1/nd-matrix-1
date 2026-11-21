#!/usr/bin/env python3
"""Chorus delay from the AUDIO of a sustained note: comb filter fit of the partial amplitudes.

A chorus adds a delayed copy `y = x + g x(t - d)`. Its magnitude response is
`|1 + g exp(-j 2 pi f d)|`, a comb with period `1 / d` in Hz, so the amplitudes of the partials of
a held note ripple against frequency with that period. This tool reads the delay `d` per analysis
frame without any scope calibration (`docs/research/fx-chorus-hw-clip.md` s8, D173):

1. short-time spectrum (Hann, 0.35 s), partial peaks between 110 Hz and 4.5 kHz;
2. detrend the peak levels (dB) with a running median over 9 partials;
3. correlate the residual with the comb ripple of every candidate `d` (0.2 to 6 ms, `g` given);
4. the best `d` per frame, kept when the correlation is at least `MIN_SCORE`;
5. ``lfo``: fit `d(t) = c + s sin(2 pi f t + phi)` (robust, capped residual) to the kept frames.

    python tools/fx_chorus_comb.py comb L.wav --t0 22.5 --t1 37.5 --out comb.csv
    python tools/fx_chorus_comb.py lfo comb.csv

Only numbers are printed or written; the audio stays in ``.reference/``.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Sequence

import numpy as np

WINDOW_S = 0.35  # EMPIRICAL: resolves partials 87 Hz apart, short against the 3 s LFO period
HOP_S = 0.2
F_LO, F_HI = 110.0, 4500.0  # Hz, partial band
MIN_SCORE = 0.3  # correlation of ripple and comb below which a frame is dropped
DELAYS_MS = np.arange(0.2, 6.0, 0.02)


def peaks(mag: np.ndarray, distance: int, prom: float) -> np.ndarray:
    """Local maxima standing `prom` above the lower of the two minima within `distance` bins on
    each side, strongest first, at least `distance` apart (numpy version of scipy's prominence)."""
    cand = np.flatnonzero((mag[1:-1] > mag[:-2]) & (mag[1:-1] >= mag[2:])) + 1
    keep = [
        i
        for i in cand
        if mag[i] - max(mag[max(i - distance, 0) : i].min(), mag[i + 1 : i + distance + 1].min())
        >= prom
    ]
    kept: list[int] = []
    for i in sorted(keep, key=lambda k: -mag[k]):
        if all(abs(i - j) >= distance for j in kept):
            kept.append(int(i))
    return np.array(sorted(kept), dtype=int)


def running_median(v: np.ndarray, n: int) -> np.ndarray:
    """Running median over `n` samples (odd), edges padded with the edge value."""
    pad = np.pad(v, n // 2, mode="edge")
    return np.median(np.lib.stride_tricks.sliding_window_view(pad, n), axis=1)


def frame_delay(x: np.ndarray, fs: float, gain: float) -> tuple[float, float, int] | None:
    """Best comb delay (ms), its correlation and the partial count of one frame, or None."""
    spec = np.abs(np.fft.rfft(x * np.hanning(len(x)), 8 * len(x)))
    f = np.fft.rfftfreq(8 * len(x), 1.0 / fs)
    lo, hi = np.searchsorted(f, [F_LO, F_HI])
    mag, ff = spec[lo:hi], f[lo:hi]
    pk = peaks(mag, int(40 / f[1]), mag.max() * 0.01)
    if len(pk) < 25:
        return None
    level = 20 * np.log10(mag[pk] + 1e-12)
    resid = level - running_median(level, 9)
    resid -= resid.mean()
    best = (-1.0, 0.0)
    for d in DELAYS_MS:
        ripple = 10 * np.log10(1 + gain**2 + 2 * gain * np.cos(2 * np.pi * ff[pk] * d * 1e-3))
        ripple -= ripple.mean()
        score = float(ripple @ resid / (np.linalg.norm(ripple) * np.linalg.norm(resid) + 1e-9))
        if score > best[0]:
            best = (score, float(d))
    return best[1], best[0], len(pk)


def comb_delays(x: np.ndarray, fs: float, t0: float, t1: float, gain: float = 0.86) -> np.ndarray:
    """Rows `(time s, delay ms, score, partials)` for the frames of `[t0, t1]`."""
    win, hop = int(WINDOW_S * fs), int(HOP_S * fs)
    rows = []
    for s in range(int(t0 * fs), int(t1 * fs) - win, hop):
        r = frame_delay(x[s : s + win], fs, gain)
        if r is not None:
            rows.append((s / fs, r[0], r[1], r[2]))
    return np.array(rows)


def lfo_fit(rows: np.ndarray) -> dict[str, float]:
    """Robust sine fit of the kept frames: rate Hz, centre, swing, range and mean (ms)."""
    rows = rows[rows[:, 2] >= MIN_SCORE]
    t, d = rows[:, 0], rows[:, 1]
    best = None
    for f in np.arange(0.28, 0.40, 0.004):
        for ph in np.linspace(0, 2 * np.pi, 24, endpoint=False):
            for c in np.arange(1.0, 3.0, 0.1):
                for s in np.arange(0.4, 2.6, 0.1):
                    r = np.minimum(np.abs(d - (c + s * np.sin(2 * np.pi * f * t + ph))), 1.0).sum()
                    if best is None or r < best[0]:
                        best = (r, f, c, s)
    _, f, c, s = best
    return {
        "frames": float(len(t)),
        "rate_hz": float(f),
        "centre_ms": float(c),
        "swing_ms": float(s),
        "min_ms": float(c - s),
        "max_ms": float(c + s),
    }


def read_mono(path: Path) -> tuple[np.ndarray, float]:
    import wave

    with wave.open(str(path)) as w:
        if w.getsampwidth() != 2:
            raise SystemExit("16-bit PCM WAV expected")
        x = np.frombuffer(w.readframes(w.getnframes()), "<i2").astype(float)
        x = x.reshape(-1, w.getnchannels())[:, 0]
        return x / np.abs(x).max(), float(w.getframerate())


def main(argv: Sequence[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("comb", help="per-frame delay of a WAV")
    c.add_argument("wav", type=Path)
    c.add_argument("--t0", type=float, default=22.5)
    c.add_argument("--t1", type=float, default=37.5)
    c.add_argument("--gain", type=float, default=0.86, help="wet over dry (clip: 0.86)")
    c.add_argument("--out", type=Path)
    lf = sub.add_parser("lfo", help="LFO fit of a comb CSV")
    lf.add_argument("csv", type=Path)
    args = ap.parse_args(argv)
    if args.cmd == "comb":
        x, fs = read_mono(args.wav)
        rows = comb_delays(x, fs, args.t0, args.t1, args.gain)
        if args.out:
            np.savetxt(args.out, rows, fmt="%.3f", delimiter=",", header="t_s,delay_ms,score,peaks")
        print(lfo_fit(rows))
    else:
        print(lfo_fit(np.loadtxt(args.csv, delimiter=",")))
    return 0


if __name__ == "__main__":
    sys.exit(main())
