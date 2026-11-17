"""Signal metrics for the synth end-to-end comparison (oracle against native, D132).

Pure numpy helpers: levels, windowed RMS envelope, FFT resampling, third-octave band levels,
fundamental estimate and the null depth of one signal against another after a best-fit gain.
Used by `tools/synth_e2e_oracle.py`; no audio is read or written here.
"""

from __future__ import annotations

import numpy as np

EPS = 1e-30


def db(x: float) -> float:
    """20 log10 of a magnitude (floored so silence gives about -600 dB, not -inf)."""
    return 20.0 * float(np.log10(max(abs(x), EPS)))


def rms(x: np.ndarray) -> float:
    """Root mean square of a signal (0 for an empty one)."""
    return float(np.sqrt(np.mean(np.square(x, dtype=np.float64)))) if len(x) else 0.0


def windowed_rms(x: np.ndarray, win: int) -> np.ndarray:
    """RMS of consecutive non-overlapping windows of `win` samples (the tail is dropped)."""
    n = len(x) // win
    return np.sqrt(np.mean(np.square(x[: n * win].reshape(n, win), dtype=np.float64), axis=1))


def resample_fft(x: np.ndarray, fs_in: float, fs_out: float) -> np.ndarray:
    """Band-limited resample by spectrum truncation or zero padding (exact for periodic input)."""
    n_out = int(round(len(x) * fs_out / fs_in))
    spec = np.fft.rfft(x.astype(np.float64))
    out = np.zeros(n_out // 2 + 1, dtype=complex)
    k = min(len(spec), len(out))
    out[:k] = spec[:k]
    return np.fft.irfft(out, n_out) * (n_out / len(x))


def third_octave_edges(lo: float = 100.0, hi: float = 16_000.0) -> list[tuple[float, float]]:
    """Band `(low, high)` edges in Hz, a third of an octave wide, from `lo` while below `hi`."""
    out, f = [], lo
    while f < hi:
        out.append((f, f * 2 ** (1 / 3)))
        f *= 2 ** (1 / 3)
    return out


def band_levels_db(x: np.ndarray, fs: float, edges: list[tuple[float, float]]) -> list[float]:
    """Level of each band in dB (RMS of the Hann windowed spectrum magnitude in the band)."""
    spec = np.abs(np.fft.rfft(x * np.hanning(len(x)))) / len(x)
    freq = np.fft.rfftfreq(len(x), 1.0 / fs)
    return [
        db(float(np.sqrt(np.sum(spec[(freq >= a) & (freq < b)] ** 2))) if b <= fs / 2 else 0.0)
        for a, b in edges
    ]


def fundamental_hz(x: np.ndarray, fs: float, lo: float = 30.0, hi: float = 2_000.0) -> float:
    """Strongest spectral peak in `lo..hi` with parabolic interpolation (0 if there is none)."""
    spec = np.abs(np.fft.rfft(x * np.hanning(len(x))))
    freq = np.fft.rfftfreq(len(x), 1.0 / fs)
    idx = np.nonzero((freq >= lo) & (freq <= hi))[0]
    if len(idx) < 3 or spec[idx].max() <= 0:
        return 0.0
    k = int(idx[np.argmax(spec[idx])])
    if k < 1 or k + 1 >= len(spec):
        return float(freq[k])
    a, b, c = np.log(np.maximum(spec[k - 1 : k + 2], EPS))
    d = 0.5 * (a - c) / (a - 2 * b + c) if (a - 2 * b + c) != 0 else 0.0
    return float((k + d) * fs / len(x))


def null_depth_db(ref: np.ndarray, test: np.ndarray) -> tuple[float, float]:
    """`(depth_dB, gain)`: `20 log10(rms(ref - g test) / rms(ref))` for the best-fit gain `g`.

    Equal length is required. -inf-like values mean identical shapes; 0 dB means no relation.
    """
    if len(ref) != len(test):
        raise ValueError(f"length {len(ref)} != {len(test)}")
    tt = float(np.dot(test, test))
    g = float(np.dot(ref, test)) / tt if tt > 0 else 0.0
    r = rms(ref)
    return (db(rms(ref - g * test) / r) if r > 0 else 0.0), g


def time_to_fraction(env: np.ndarray, hop_s: float, frac: float, steady: float) -> float:
    """Time of the first window of `env` at or above `frac * steady` (-1 if never)."""
    hit = np.nonzero(env >= frac * steady)[0]
    return float(hit[0] * hop_s) if len(hit) else -1.0


def time_to_below(env: np.ndarray, hop_s: float, frac: float, steady: float) -> float:
    """Time of the first window of `env` at or below `frac * steady` (-1 if never)."""
    hit = np.nonzero(env <= frac * steady)[0]
    return float(hit[0] * hop_s) if len(hit) else -1.0


def band_delta_db(a: np.ndarray, b: np.ndarray, span_db: float = 80.0) -> float:
    """Mean absolute band *shape* difference in dB, independent of the overall level.

    Each band vector is normalised to its own peak (level offset removed), then clipped at
    `-span_db` (the floor): a band below the floor in both counts as equal, a band above the floor
    in one and below in the other counts as the distance to the floor. Bands below the floor in
    both are ignored. Returns 0.0 when no band is above the floor.
    """
    an = np.maximum(a - a.max(), -span_db)
    bn = np.maximum(b - b.max(), -span_db)
    keep = (an > -span_db) | (bn > -span_db)
    return float(np.mean(np.abs(an[keep] - bn[keep]))) if keep.any() else 0.0
