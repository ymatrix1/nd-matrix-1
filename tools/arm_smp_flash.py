#!/usr/bin/env python3
"""SMP page refill: the stroke descriptor law and the flash feed of the live SMP chip (OQ41, D191).

The ARM never sends a flash address to the SMP. It writes the stroke descriptor into the voice
record (record words `-11 .. -4`, see `docs/research/smp-page-refill.md`): `rec[-11]` is the
directory index of the file, `rec[-10]` the page number of the stroke, `rec[-9]` its frame count.
The SMP page routine turns them into a flash word address through the directory `DM(0xB237B + i)`.

`stroke_page` is the law (VERIFIED: the ring content of the 2 strokes of an Astoria Grand run
equals the file bytes of the page, `data/smp-page-refill-rings.csv`):

    file byte of page v = 1022 * channels * v - 148        (offset includes the 0x2C CNSP base)
    rec[-10] of a stroke = (stroke file byte + 148) / (1022 * channels)

`SmpFlashFeed` watches the messages of the SMP link, recognises a descriptor by its frame count,
checks the law and sends `F rec[-11]` so the live chip places the file and writes the directory.
`check_rings` compares the stream rings of the chip with the file bytes of the stroke pages.
"""

from __future__ import annotations

import math
import struct
from dataclasses import dataclass, field
from pathlib import Path

CNSP_BASE = 0x2C  # VERIFIED (nsmp4-and-npno-cnsp-codec.md s3.2): file byte = 0x2C + u32 at +8
STROKE_TABLE = 0x756
STROKE_SIZE = 0x76
CHANNELS_AT, NSTROKES_AT = 0x64A, 0x64C
PAGE_BYTES = 1022  # per channel (VERIFIED, same note)
# VERIFIED (run, Astoria Grand strokes 513 and 502): the page grid of rec[-10] starts 148 bytes
# before the file: file byte of page v = PAGE_BYTES * channels * v - 148.
GRID_ORIGIN = -148


@dataclass(frozen=True)
class Stroke:
    index: int
    offset: int  # file byte of the stroke (CNSP base included)
    frames: int
    npages: int
    channels: int


def parse_strokes(npno: bytes) -> list[Stroke]:
    """The stroke records of a `.npno` (header tables only are needed, the audio can be absent)."""
    channels = struct.unpack_from(">H", npno, CHANNELS_AT)[0]
    count = struct.unpack_from(">H", npno, NSTROKES_AT)[0]
    out = []
    for k in range(count):
        at = STROKE_TABLE + k * STROKE_SIZE
        off = struct.unpack_from(">I", npno, at + 8)[0] + CNSP_BASE
        frames = struct.unpack_from(">I", npno, at + 0x0E)[0]
        npages = struct.unpack_from(">H", npno, at + 0x12)[0]
        out.append(Stroke(k, off, frames, npages, channels))
    return out


def stroke_page(stroke: Stroke) -> int | None:
    """`rec[-10]` of the stroke, `None` when the offset is not on the page grid."""
    unit = PAGE_BYTES * stroke.channels
    value, rest = divmod(stroke.offset - GRID_ORIGIN, unit)
    return value if rest == 0 else None


@dataclass
class Detection:
    """One descriptor seen in the message stream."""

    address: int  # DM word of `rec[-11]`
    rec11: int
    rec10: int
    frames: int
    stroke: int | None  # matched stroke index (None: no stroke of the file has this frame count)
    law_ok: bool  # `rec[-10]` equals `stroke_page` of the stroke
    placed: str = ""  # reply of the `F` command


@dataclass
class SmpFlashFeed:
    """Hook for the SMP link: places the loaded file when a descriptor appears."""

    strokes: list[Stroke]
    link: object
    last: dict[int, int] = field(default_factory=dict)
    detections: list[Detection] = field(default_factory=list)
    rec11: int | None = None

    def __post_init__(self) -> None:
        self.by_frames: dict[int, list[Stroke]] = {}
        for s in self.strokes:
            self.by_frames.setdefault(s.frames, []).append(s)

    def observe(self, address: int, value: int) -> Detection | None:
        """Called for every message; a descriptor is `rec[-11], rec[-10], rec[-9]` in a row."""
        self.last[address] = value
        if value not in self.by_frames or address - 2 not in self.last:
            return None
        rec11, rec10 = self.last.get(address - 2), self.last.get(address - 1)
        if rec11 is None or rec10 is None:
            return None
        match = next((s for s in self.by_frames[value] if stroke_page(s) == rec10), None)
        found = match or self.by_frames[value][0]
        det = Detection(address - 2, rec11, rec10, value, found.index, match is not None)
        if match is not None and rec11 != self.rec11:
            self.rec11 = rec11
            det.placed = self.link.command(f"F {rec11}")  # type: ignore[attr-defined]
        self.detections.append(det)
        return det

    def summary(self) -> dict[str, object]:
        return {
            "rec11": self.rec11,
            "descriptors": len(self.detections),
            "law_ok": sum(d.law_ok for d in self.detections),
            "first": [d.__dict__ for d in self.detections[:6]],
        }


def attach(link, npno: bytes, path: Path, tap: Path | None = None) -> SmpFlashFeed:  # noqa: ANN001  # pragma: no cover
    """Loads `npno` as the flash image of the live chip behind `link` and hooks its messages;
    `tap` also captures the tapped frames (f32 little endian L R, local audio, never committed)."""
    path.write_bytes(npno)
    if tap is not None:
        link.command(f"C {tap.as_posix()}")
    reply = link.command(f"L {path.as_posix()}")
    if not reply.startswith("L words="):
        raise RuntimeError(f"flash load failed: {reply}")
    feed = SmpFlashFeed(parse_strokes(npno), link)
    link.on_send = feed.observe
    return feed


RING_BASE, RING_STRIDE, RING_COUNT = 0x9B000, 0x100, 32  # VERIFIED (run): `rec[-26]` of the voices
RING_PROBE_WORDS = 8


def expected_head(npno: bytes, stroke: Stroke) -> list[int]:
    """First payload words of page 0 of `stroke`: the 16-bit big-endian words after the header."""
    at = stroke.offset + 2
    return [struct.unpack_from(">H", npno, at + 2 * k)[0] for k in range(RING_PROBE_WORDS)]


def check_rings(link, npno: bytes, strokes: list[Stroke]) -> list[dict]:  # noqa: ANN001  # pragma: no cover
    """For every ring whose first words equal the head of a stroke page 0: `{ring, stroke}`."""
    heads = {tuple(expected_head(npno, s)): s.index for s in strokes}
    out = []
    for k in range(RING_COUNT):
        base = RING_BASE + k * RING_STRIDE
        words = tuple(w & 0xFFFF for w in link.dump(base, RING_PROBE_WORDS))
        if words in heads:
            out.append({"ring": f"{base:#x}", "stroke": heads[words]})
    return out


def tap_stats(data: bytes) -> dict[str, object]:
    """Level and shape of a captured tap (f32 little endian, `L R` per frame, `sharc-live` `C`).

    `kind` tells silence, a stuck-sample plateau (D191: one value repeated) and audio apart:
    audio needs a peak above -90 dBFS and more than 64 distinct values in the loudest window.
    """
    n = len(data) // 4
    x = struct.unpack(f"<{n}f", data[: 4 * n]) if n else ()
    peak = max((abs(v) for v in x if math.isfinite(v)), default=0.0)
    win = 4096
    best = max(
        (x[i : i + win] for i in range(0, max(n, 1), win)),
        key=lambda w: max(map(abs, w), default=0.0),
        default=(),
    )
    distinct = len(set(best))
    if peak < 3e-5:
        kind = "silent"
    elif distinct <= 64:
        kind = "plateau"
    else:
        kind = "audio"
    return {
        "words": n,
        "peak": peak,
        "peak_dbfs": 20 * math.log10(peak) if peak > 0 else None,
        "distinct_in_loudest_window": distinct,
        "kind": kind,
    }


# EMPIRICAL (D230, `piano-smp-pitch-outliers.md`): the autocorrelation window, the lag range and the
# peak threshold were chosen on 10 taps (9 piano programs the old harmonic-sum detector got wrong
# plus Astoria Grand); all 10 give the octave of the program within 1.2 %.
PITCH_WINDOW = 4096  # frames from the onset
PITCH_LAGS = (33, 1000)  # cycles per frame 0.03 .. 0.001 (1.3 kHz .. 43 Hz at 43.4 kHz)
PITCH_ONSET = 0.05  # onset: first frame above this fraction of the peak
PITCH_PEAK = 0.85  # the first autocorrelation maximum above this fraction of the best one


def tap_fundamental(data: bytes) -> float | None:
    """Fundamental of a captured tap in cycles per tap frame (autocorrelation of the left lane).

    The tap holds one frame per audio interrupt (D205); the pitch in Hz is `cycles * frame_rate`
    and the frame rate of the SMP chip is the DSP frame rate 43,402.34 Hz (D211, VERIFIED: ARM
    literal, not 48 kHz). The window starts at the onset (first frame above `PITCH_ONSET` of the
    peak) and spans `PITCH_WINDOW` frames. The pitch is the **first** normalised autocorrelation
    maximum within `PITCH_PEAK` of the best one, refined by the harmonic sum inside that octave.
    D230: the earlier harmonic-sum detector (4 harmonics, bins 40..4000 of a 2^17 FFT) read its
    lowest candidate (0.0003, three programs), half the pitch (Gradient Whl) or twice (French
    Harpsi, strong even harmonics). `None` for a silent or too short tap.
    """
    import numpy as np  # local: only the pitch check needs it

    x = np.frombuffer(data[: len(data) // 8 * 8], dtype="<f4").reshape(-1, 2)[:, 0].astype(float)
    if len(x) < 1024 or not np.isfinite(x).all() or float(np.abs(x).max()) < 3e-5:
        return None
    start = int(np.argmax(np.abs(x) >= PITCH_ONSET * float(np.abs(x).max())))
    seg = x[start : start + PITCH_WINDOW]
    seg = seg - seg.mean()
    lo, hi = PITCH_LAGS[0], min(PITCH_LAGS[1], len(seg) // 2)
    if hi <= lo + 2:
        return None
    ac = np.fft.irfft(np.abs(np.fft.rfft(seg, n=1 << 14)) ** 2)[: len(seg)]
    lags = np.arange(lo - 1, hi + 2)
    energy = np.concatenate(([0.0], np.cumsum(seg**2)))
    head = energy[len(seg) - lags]  # sum of seg[:n - lag] squared
    tail = energy[-1] - energy[lags]  # sum of seg[lag:] squared
    r = ac[lags] / np.sqrt(np.maximum(head * tail, 1e-30))
    best = float(r[1:-1].max())
    for i in range(1, len(r) - 1):
        if r[i] >= PITCH_PEAK * best and r[i] >= r[i - 1] and r[i] >= r[i + 1]:
            return _refine_pitch(seg, 1.0 / lags[i])
    return None


def _refine_pitch(seg, coarse: float, harmonics: int = 4) -> float:  # noqa: ANN001
    """Harmonic-sum refinement of the autocorrelation pitch `coarse` within +-3 % (the lag has
    1 % resolution at 0.006 cycles per frame; the harmonic sum is the D205 detector, accurate to
    0.1 %, but only trusted inside the octave the autocorrelation chose)."""
    import numpy as np

    size = 1 << 17
    mag = np.abs(np.fft.rfft(seg * np.hanning(len(seg)), n=size))
    cand = np.arange(int(0.97 * coarse * size), int(1.03 * coarse * size) + 1)
    score = [sum(mag[min(k * c, len(mag) - 1)] for k in range(1, harmonics + 1)) for c in cand]
    return float(cand[int(np.argmax(score))] / size)
