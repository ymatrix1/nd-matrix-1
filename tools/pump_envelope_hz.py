#!/usr/bin/env python3
"""Strongest amplitude-envelope modulation of a WAV in a frequency band (PUMP rate check).

Method (`docs/research/fx-mod-types.md` s7.2): mono mix, 5 ms moving average of `|x|`, decimated to
200 Hz, log, mean removed, Hann window, zero padded FFT (8x); the peak between `--lo` and `--hi`
Hz. The bin spacing of a 28 s clip is 0.036 Hz before padding.

    python tools/pump_envelope_hz.py hw.wav emu.wav [--lo 1.5] [--hi 8]
"""

from __future__ import annotations

import argparse
import sys

import numpy as np

ENV_RATE_HZ = 200
SMOOTH_S = 0.005


def envelope_peak(x: np.ndarray, sr: int, lo: float = 1.5, hi: float = 8.0) -> tuple[float, float]:
    """`(peak Hz, peak height relative to the strongest 0.1..8 Hz line)` of the log envelope."""
    k = max(1, int(sr * SMOOTH_S))
    env = np.convolve(np.abs(x), np.ones(k) / k, "same")[:: max(1, sr // ENV_RATE_HZ)]
    env = np.log(env + 1e-6)
    env -= env.mean()
    n = len(env)
    spec = np.abs(np.fft.rfft(env * np.hanning(n), n * 8))
    freq = np.fft.rfftfreq(n * 8, 1 / ENV_RATE_HZ)
    band = (freq >= lo) & (freq <= hi)
    i = int(np.argmax(spec[band]))
    ref = spec[(freq >= 0.1) & (freq <= 8)].max()
    return float(freq[band][i]), float(spec[band][i] / ref)


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - thin CLI
    import scipy.io.wavfile as wav

    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("wavs", nargs="+")
    ap.add_argument("--lo", type=float, default=1.5)
    ap.add_argument("--hi", type=float, default=8.0)
    ns = ap.parse_args(argv)
    for path in ns.wavs:
        sr, x = wav.read(path)
        x = x.astype(float)
        x = x.mean(1) if x.ndim > 1 else x
        hz, rel = envelope_peak(x, sr, ns.lo, ns.hi)
        print(f"{path}: {hz:.3f} Hz (relative height {rel:.2f})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
