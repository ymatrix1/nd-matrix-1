#!/usr/bin/env python3
"""FX families on live input: SHARC oracle (firmware FX image) against the native FX models.

Builds the ARM-style message script that installs one FX routine in stage 0 of the FX Core1
dispatcher (`docs/research/fx-oracle-live.md`), feeds the same stereo signal to `sharc-fxlive`
(oracle, bus tap `0xC0458`) and to `fx_live_native` (native model), then reports per family:
RMS delta, spectral centroid, tail length and null depth after the best lag and gain.

    python tools/fx_oracle_live.py FAMILY --image FX/core1_sram.bin [--signal burst] [--out DIR]

FAMILY is `reverb0` (Spring) or `chor4` (Mod 2 CHOR); `identity` runs a pass-through stage as
a self check of the harness (stage output must equal the input). Needs the built `sharc-fxlive` and
`fx_live_native` (`cargo build --release -p nord-sharc -p nord-tools`). Nothing here reads or
writes firmware data except the local FX image path you pass; no firmware bytes are committed.
"""

from __future__ import annotations

import argparse
import math
import re
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parent.parent
RATE = 43402  # DSP frame rate (Hz, rounded for the WAV header)
FRAME_RATE = 43402.34375
BUS = 0xC0458  # FX Core1 input bus 0 (sum of the three PNO 2D sections at +0, +0x28, +0x50)
INPUT = 0xC01BC  # ping input block of FX Core1 (PNO 2D block, section 0, bus 0)
# Stage 0 record of the FX dispatcher (I1 = 0xB62EE): routine, input pointer, state pointer, gain
REC = {"fn": 0xB62CE, "input": 0xB62CF, "state": 0xB62D0, "step": 0xB62D2, "level": 0xB62D3}
MASTER = 0xB6518  # master level word (0 in the image = muted output)
F_ONE, F_HALF = 0x3F800000, 0x3F000000
# The dispatcher walks 10 groups (5, 2, 5, 2, ... stages, 35 in all). Each group starts at the
# pointer DM(header) and every record continues with the link at S - 0x1D; in the image every
# header and the head record's link point back to the head record 0xB62EE, so an installed head
# stage would run once per group (10 calls per pass) and once per following stage of its group
# (35 calls per pass in total): `docs/research/fx-reverb0-chor-oracle-diff.md` s2.
GROUP_HEADERS = (
    0xB63EC,
    0xB6416,
    0xB6428,
    0xB6452,
    0xB6464,
    0xB648E,
    0xB64A0,
    0xB64CA,
    0xB64DC,
    0xB6506,
)
HEAD_LINK = 0xB62D1  # link of the head record (S - 0x1D, S = 0xB62EE)
NULL_REC = (
    0xB6600  # 8 words of free data RAM (zero in the v1.66 image): a copy of the null stub record
)
NULL_STUB = 0x125346

# Routine pcs on FX Core1 (core 2 pc + 0x974 after the pass routine): VERIFIED against
# `docs/research/fx-chain-install.md` s2 (runs 21 and 24).
ROUTINE = {"identity": 0x1231CB, "reverb0": 0x124AF2, "chor4": 0x123798, "chor5": 0x1238EB}
REVERB_I1 = 0xB27E8  # context of the Spring reverb instance (fx-rvb-words: ctx 0xB27E8)
# Buffers the reverb clear routine zeroes (`fw_rvb_core.rs`: LINE_LEN 5008, AP_LEN 3200, STATE_LEN
# 0x200); the image holds other data there at boot.
REVERB_REGIONS = ((0x8013000, 5008), (0x8014388, 5008), (0xE24E7, 3200), (0xB2849, 0x200))
CHOR_I1 = 0xB2040  # piano unit CHOR context (docs/research/data/fx-type-words.csv)
CHOR_RING = 0xB4000  # free data RAM for the 0x410 word delay ring (same address as the port)
POLY = (0x3FC90069, 0xBF245655, 0x3D92AA8A)
K = 0x3F7DA3C2
MIX_SLOPE, MIX_MAX = 0.011133859, struct.unpack("<f", struct.pack("<I", 0x3F34FDF4))[0]


def f32(x: float) -> int:
    return struct.unpack("<I", struct.pack("<f", x))[0]


def null_chain() -> list[tuple[int, int]]:
    """Words that make the pass call stage 0 once: a null stub record (a copy of the image's own)
    that the head record's link and the nine later group headers point at."""
    s = NULL_REC + 0x20
    rec = (NULL_STUB, 0xB62AA, 0, s, 0xBC23D70A, 0xBF000000, 0xBF000000, F_HALF)
    words = [(NULL_REC + k, w) for k, w in enumerate(rec)]
    words += [(h, s) for h in GROUP_HEADERS[1:]]
    return words + [(HEAD_LINK, s)]


def install(family: "str | int", state: int) -> list[tuple[int, int]]:
    """Stage 0 record: routine, input bus, state block, level (gain = clip(level+step) + 0.5)."""
    return null_chain() + [
        (MASTER, F_ONE),
        (REC["fn"], family if isinstance(family, int) else ROUTINE[family]),
        (REC["input"], BUS),
        (REC["state"], state),
        (REC["step"], 0),
        (REC["level"], F_HALF),
    ]


def reverb_type_words(rs_text: str, rvb_type: int = 0) -> list[tuple[int, int]]:
    """`(ctx offset, word)` of `TYPE_WORDS[rvb_type]` parsed from `fw_rvb_words.rs`."""
    block = re.search(r"TYPE_WORDS: \[\[\(i8, u32\); 39\]; 2\] = \[(.*?)\n\];", rs_text, re.S)
    if not block:
        raise ValueError("TYPE_WORDS not found")
    groups = re.findall(r"\[\s*((?:\(-?\d+, 0x[0-9a-f]+\),?\s*)+)\]", block.group(1))
    pairs = re.findall(r"\((-?\d+), (0x[0-9a-f]+)\)", groups[rvb_type])
    return [(int(o), int(w, 16)) for o, w in pairs]


def reverb0_words(rs_text: str, send: float = 1.0, dry: float = 0.0) -> list[tuple[int, int]]:
    """Messages for the Spring reverb: type words and the settled `kRvbAmt` words."""
    words = [(a + k, 0) for a, n in REVERB_REGIONS for k in range(n)]  # the clear routine's job
    words += [(REVERB_I1 + o, w) for o, w in reverb_type_words(rs_text, 0)]
    for off, v in ((0x12, send), (0x13, dry), (0x14, send), (0x15, dry)):
        words.append((REVERB_I1 + off, f32(v)))
    return words + install("reverb0", REVERB_I1)


def chor4_words(rate: int = 64, amount: int = 64) -> list[tuple[int, int]]:
    """Context of the CHOR routine `0x123798` as `fw_mod2.rs` builds it (scale 1, soft kind)."""
    c = {o: 0 for o in range(-0x20, 0x26)}
    c[-0x16] = CHOR_I1 - 0x18
    c[-9] = c[-7] = CHOR_RING
    c[-0x1F], c[-0x19] = f32(35.0), f32(62.0)
    for o, w in zip((0xC, 0xD, 0xE), POLY):
        c[o] = w
    c[0xF], c[0x10], c[0x11] = f32(1.0), f32(-1.0), f32(0.5)
    c[0xA], c[0x14] = 1, K
    v = float(amount)
    c[-0x20] = 12800 + 4096 * rate
    ramp = min(max((v - 63.5) / 63.5, 0.0), 1.0)
    c[-0x1E], c[-0x1D] = f32(117.0 + 39.0 * ramp), f32(171.0 + 9.0 * ramp)
    c[-3] = f32(min(v * MIX_SLOPE, MIX_MAX))
    c[-2] = f32(min(max((v - MIX_MAX / MIX_SLOPE) * MIX_SLOPE, 0.0), MIX_MAX))
    c[1] = f32(1.0 - 0.0023070865 * v)
    for dst, src in ((-0x1B, -0x1E), (-0x1A, -0x1D), (-1, -3), (0, -2), (2, 1), (3, 1)):
        c[dst] = c[src]
    c[9] = 2  # blocks of two stereo frames per call (4 frames per pass)
    words = [(CHOR_I1 + o, w) for o, w in sorted(c.items())]
    words += [(CHOR_RING + k, 0) for k in range(0x410)]
    return words + install("chor4", CHOR_I1)


#: Context words of the CHOR variation (type 5) at rate 64, amount 64, relative to `I1`: the words
#: `FwMod2Stage::new(true, 64, 64, ..)` builds (`fw_mod2.rs`; the rate / amount laws are VERIFIED
#: there against the ARM words). The block count word `c[2]` is set per call by the port.
CHOR5_CTX = {
    -0x1E: 0x3F7F966C, -0x1D: 0x00074308, -0x1C: 0x42969FAC, -0x1B: 0x42C6C0A9,
    -0x1A: 0x42969FAC, -0x19: 0x42C6C0A9, -0x16: CHOR_I1 - 0x18, -0xF: CHOR_RING, -0xD: CHOR_RING,
    -9: 0x3F800000, -8: 0x3F800000, 2: 2, 3: 0x3F800000, 4: 0xBF800000, 5: 0x3F000000,
    6: K, 7: 0x3F7A8EEC, 8: 0x3DB09D72, 9: 0x3F751DD9, 10: 0xBF53D8A3, 15: 0x3F69EC52,
    16: 0x3F53D8A3, 19: 0x3CAE2280, 20: 0xBF751DD9,
}  # fmt: skip


def chor5_words() -> list[tuple[int, int]]:
    c = {o: 0 for o in range(-0x20, 0x26)} | CHOR5_CTX
    words = [(CHOR_I1 + o, w) for o, w in sorted(c.items())]
    words += [(CHOR_RING + k, 0) for k in range(0x410)]
    return words + install("chor5", CHOR_I1)


def script_text(words: list[tuple[int, int]]) -> str:
    """`PASS WORD1 WORD2` lines (all before pass 0, in order)."""
    return "".join(f"0 0x{a:X} 0x{w:08X}\n" for a, w in words)


def signal(kind: str, frames: int) -> np.ndarray:
    """Stereo test signal `(frames, 2)`: `burst` = 0.25 s 440 Hz tone then silence (tail),
    `sine` = steady 440 Hz, `noise` = deterministic noise burst."""
    t = np.arange(frames)
    if kind == "sine":
        x = 0.25 * np.sin(2 * math.pi * 440 * t / FRAME_RATE)
    elif kind == "burst":
        x = 0.25 * np.sin(2 * math.pi * 440 * t / FRAME_RATE)
        x[int(0.25 * FRAME_RATE) :] = 0.0
    elif kind == "impulse":
        x = np.zeros(frames)
        x[0] = 0.5
    elif kind == "noise":
        x = 0.2 * np.random.default_rng(7).uniform(-1, 1, frames)
        x[int(0.1 * FRAME_RATE) :] = 0.0
    else:
        raise ValueError(f"unknown signal {kind!r}")
    return np.stack([x, 0.8 * x], axis=1).astype(np.float32)


def write_wav(path: Path, data: np.ndarray, rate: int = RATE) -> None:
    n, ch = data.shape
    body = data.astype("<f4").tobytes()
    hdr = b"RIFF" + struct.pack("<I", 36 + len(body)) + b"WAVEfmt "
    hdr += struct.pack("<IHHIIHH", 16, 3, ch, rate, rate * ch * 4, ch * 4, 32)
    path.write_bytes(hdr + b"data" + struct.pack("<I", len(body)) + body)


def read_wav(path: Path) -> np.ndarray:
    b = path.read_bytes()
    ch = struct.unpack("<H", b[22:24])[0]
    return np.frombuffer(b[44:], dtype="<f4").reshape(-1, ch)


def rms(x: np.ndarray) -> float:
    return float(np.sqrt(np.mean(np.square(x.astype(np.float64))))) if x.size else 0.0


def db(x: float) -> float:
    return 20 * math.log10(max(x, 1e-30))


def centroid(x: np.ndarray, rate: float) -> float:
    """Spectral centroid (Hz) of a mono signal."""
    s = np.abs(np.fft.rfft(x.astype(np.float64) * np.hanning(len(x))))
    f = np.fft.rfftfreq(len(x), 1 / rate)
    return float((s * f).sum() / s.sum()) if s.sum() > 0 else 0.0


def tail_ms(x: np.ndarray, end: int, rate: float, floor_db: float = -60.0) -> float:
    """Time after sample `end` until the 1 ms RMS envelope stays below `floor_db` of its peak."""
    w = max(1, int(rate * 0.001))
    env = np.sqrt(np.convolve(np.square(x.astype(np.float64)), np.ones(w) / w, "same"))
    peak = env[: end + 1].max() if env[: end + 1].size else 0.0
    if peak <= 0:
        return 0.0
    above = np.nonzero(env[end:] > peak * 10 ** (floor_db / 20))[0]
    return float(above[-1] / rate * 1000) if above.size else 0.0


def best_lag(a: np.ndarray, b: np.ndarray, span: int = 16) -> int:
    """Lag `k` in `-span..span` with `a[n + k]` closest to `b[n]` (least squares gain applied)."""
    best, arg = None, 0
    for k in range(-span, span + 1):
        x, y = (a[k:], b[: len(b) - k]) if k >= 0 else (a[: len(a) + k], b[-k:])
        n = min(len(x), len(y))
        x, y = x[:n].astype(np.float64), y[:n].astype(np.float64)
        d = float(np.dot(y, y))
        if d == 0:
            continue
        g = float(np.dot(x, y)) / d
        e = float(np.sum((x - g * y) ** 2))
        if best is None or e < best:
            best, arg = e, k
    return arg


def compare(oracle: np.ndarray, native: np.ndarray, rate: float, input_end: int) -> dict:
    """Per-family numbers on the left channel (both aligned by sample index)."""
    n = min(len(oracle), len(native))
    o, m = oracle[:n, 0].astype(np.float64), native[:n, 0].astype(np.float64)
    lag = best_lag(o, m)
    x, y = (o[lag:], m[: n - lag]) if lag >= 0 else (o[: n + lag], m[-lag:])
    k = min(len(x), len(y))
    x, y = x[:k], y[:k]
    gain = float(np.dot(x, y) / np.dot(y, y)) if np.dot(y, y) > 0 else 0.0
    res_unity = rms(x - y)
    res_fit = rms(x - gain * y)
    return {
        "frames": n,
        "lag_frames": lag,
        "rms_oracle_db": db(rms(x)),
        "rms_native_db": db(rms(y)),
        "rms_delta_db": db(rms(x)) - db(rms(y)),
        "fit_gain": gain,
        "centroid_oracle_hz": centroid(o, rate),
        "centroid_native_hz": centroid(m, rate),
        "tail_oracle_ms": tail_ms(o, input_end, rate),
        "tail_native_ms": tail_ms(m, input_end, rate),
        "null_unity_db": db(res_unity) - db(rms(y)),
        "null_fit_db": db(res_fit) - db(rms(y)),
    }


def run(cmd: list[str]) -> str:
    return subprocess.run(cmd, capture_output=True, text=True, check=True).stdout


def family_run(args: argparse.Namespace) -> dict:
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    frames = int(args.seconds * FRAME_RATE) // 4 * 4
    lead = args.lead * 4  # silence so the master stage fade-in (about 2000 passes) has settled
    sig = np.concatenate([np.zeros((lead, 2), np.float32), signal(args.signal, frames)])
    in_wav, ora_wav, nat_wav = out / "in.wav", out / "oracle.wav", out / "native.wav"
    write_wav(in_wav, sig)
    if args.family == "reverb0":
        text = (REPO / "crates/nord-core/src/fx/fw_rvb_words.rs").read_text()
        words = reverb0_words(text)
    elif args.family == "chor4":
        words = chor4_words(args.rate, args.amount)
    elif args.family == "chor5":
        words = chor5_words()
    elif args.family == "identity":
        words = install("identity", REVERB_I1)
    else:
        raise SystemExit(f"unknown family {args.family!r}")
    script = out / f"{args.family}.script"
    script.write_text(script_text(words))
    print(
        run(
            [
                args.sharc, args.image, "--core", "1", "--passes", str((lead + frames) // 4),
                "--in-wav", str(in_wav), "--script-flat", str(script),
                "--wav", str(ora_wav),
            ]
        ).strip()
    )  # fmt: skip
    nat_family = "off" if args.family == "identity" else args.family
    extra = (
        ["--rate", str(args.rate), "--amount", str(args.amount)]
        if args.family in ("chor4", "chor5")
        else []
    )
    run([args.native, nat_family, str(in_wav), str(nat_wav), *extra])
    res = compare(
        read_wav(ora_wav)[lead:], read_wav(nat_wav)[lead:], FRAME_RATE,
        int(0.25 * FRAME_RATE) if args.signal != "sine" else frames - 1,
    )  # fmt: skip
    return res


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("family", choices=["identity", "reverb0", "chor4", "chor5"])
    ap.add_argument(
        "--image", required=True, help="FX core1_sram.bin (tools/ns4_dsp_ldr.py --sram)"
    )
    ap.add_argument("--signal", default="burst", choices=["burst", "sine", "noise", "impulse"])
    ap.add_argument("--seconds", type=float, default=1.5)
    ap.add_argument("--lead", type=int, default=3000, help="silent passes before the signal")
    ap.add_argument("--rate", type=int, default=64)
    ap.add_argument("--amount", type=int, default=64)
    ap.add_argument("--out", default=tempfile.mkdtemp(prefix="fx_live_"))
    ap.add_argument("--sharc", default=str(REPO / "target/release/sharc-fxlive"))
    ap.add_argument("--native", default=str(REPO / "target/release/fx_live_native"))
    a = ap.parse_args(argv)
    a.sharc, a.native = str(Path(a.sharc).resolve()), str(Path(a.native).resolve())
    res = family_run(a)
    for k, v in res.items():
        print(f"{k:22s} {v:.4f}" if isinstance(v, float) else f"{k:22s} {v}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
