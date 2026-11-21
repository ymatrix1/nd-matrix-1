#!/usr/bin/env python3
"""Bit-exact Python model of the Core2 synth filter stage `0x1257B2` (raw filter type 3).

Written from the listing of ORG core 2 (`0x1257B2..0x125913`, bytes `0x24AF64..0x24B226`, ADI
`elfdump` via `tools/adi_dis.py`) and checked against `sharc-org` word for word
(`python tools/synth_filter_t3.py --image SRAM_ORG/core2_sram.bin`;
`docs/research/synth-filter-t3.md`).

The stage is a SIMD pair: the two lanes (words `k` and `k + 1` of every frame field) are two
independent channels with identical coefficients (the register swaps `rA<->sB` only move table
values between lanes while the coefficients are built); the model is one lane. Per call it
**builds its own coefficients** (frame words `-24`, `-22`, `-20`, `+10`, `+12`, `+14`) from the
cutoff word `R` (`I3 - 30`) and the resonance position word `W` (`I3 - 28`), then filters 4 samples.

Tables (`DM(0xB0A9F)`, `DM(0xB0C0D)`, `DM(0xB0C8E)`, 129 words each,
`docs/research/data/synth-filter-t3-tables.csv`):

    P  = FIX(R / 2, 31)            i = P >> 24 (8 bit signed)   fr = (P & 0xFFFFFF) / 2^24
    U  = T1[i] + (T1[i+1] - T1[i]) fr          V = T2[i] + (T2[i+1] - T2[i]) fr
    g  = 2 U                                   (frame -22)      V is frame -20
    Wv = T0[j] + (T0[j+1] - T0[j]) fw          j = W >> 24      fw = (W & 0xFFFFFF) / 2^24
    Q  = FIX(Wv / 2, 31)           i2 = Q >> 24   f2 = (Q & 0xFFFFFF) / 2^24
    X1 = T1 lerp (i2, f2)          X2 = T2 lerp (i2, f2)
    kc = CLIP(g 4.5 + 0.25, 1)     (frame +10)      2 X1 (frame +12)      X2 (frame +14)
    h  = 1 - (X1^2 + X1 X2)        (frame -24)      kh = kc h

Every coefficient is stored to the frame (32 bit truncation) and read back, the states too. Per
sample (states `A` (-10), `D` (-8), `B` (-6), `C` (-4), `E` (-2); `cin` = 1.0 at frame -26):

    p  = B + g A                      B' = p
    e  = E + 2 X1 C                   E' = e         D' = x
    n  = (x kh - e) - X2 C
    m  = ((D + x) / 2 - p) - V A
    y  = n + (B + p) / 2              output
    C' = CLIP(C + 2 X1 n, 1)          A' = CLIP(A + g m, 1)

The ladder-like reading (a 4 pole low pass with real poles, `synth-adr-and-strip-law.md` s5) is the
impulse response of this coupled form.
"""

from __future__ import annotations

import argparse
import csv
import json
import random
import struct
import sys
import tempfile
from fractions import Fraction
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent))
import sharc40 as s
import synth_stage_oracle as o

DATA = Path(__file__).resolve().parents[1] / "docs/research/data"
STAGE = 0x1257B2
# VERIFIED (image words `DM(0xC28FA + k)`, operands of the listing): common block constants.
QUARTER = s.ld(0x3E800000)  # DM(I5 - 32)
HALF = s.ld(0x3F000000)  # DM(I5 - 30)
ONE = s.ld(0x3F800000)  # DM(I5 - 28)
K_GAIN = s.ld(0x40900000)  # DM(I5 - 20) = 4.5
TABLE_ADDRS = (0xB0A9F, 0xB0C0D, 0xB0C8E)
TABLE_LEN = 129
STATE_OFFS = (-10, -8, -6, -4, -2)  # a, d, b, c, e
COEF_KEYS = ("h", "g", "v", "kc", "x1x2", "x2")
COEF_OFFS = (-24, -22, -20, 10, 12, 14)
PASSES = 3
NL = chr(10)


def read_tables() -> list[list[int]]:
    """`[T0, T1, T2]` as float32 words (129 each) from the derived CSV."""
    cols: list[list[int]] = [[], [], []]
    with (DATA / "synth-filter-t3-tables.csv").open(newline="", encoding="utf-8") as fh:
        for r in csv.DictReader(fh):
            for k, name in enumerate(("t0_0xB0A9F", "t1_0xB0C0D", "t2_0xB0C8E")):
                cols[k].append(int(r[name], 16))
    return cols


TABLES = read_tables()


def lerp(table: list[int], idx: int, fr: Fraction) -> Fraction:
    """`T[i] + (T[i+1] - T[i]) fr` in the listing's operation order."""
    lo, hi = s.ld(table[idx]), s.ld(table[idx + 1])
    return s.fadd(lo, s.fmul(s.fsub(hi, lo), fr))


def split(value: Fraction) -> tuple[int, Fraction]:
    """`FIX(value, 31)`, then the signed top byte and the 24 bit fraction of the word."""
    p = s.fix(value, 31)
    idx = (p >> 24) & 0xFF
    return idx - 256 if idx >> 7 else idx, Fraction(p & 0xFFFFFF, 1 << 24)


def call_coefficients(w_r: int, w_pos: int) -> tuple[dict[str, int], Fraction]:
    """Frame words written by the prologue (`h g v kc x1x2 x2`) and the register `kh = kc h`."""
    i, fr = split(s.fmul(s.ld(w_r), HALF))
    t0, t1, t2 = TABLES
    u, v = lerp(t1, i, fr), lerp(t2, i, fr)
    j, fw = (w_pos >> 24) & 0xFF, Fraction(w_pos & 0xFFFFFF, 1 << 24)
    i2, f2 = split(s.fmul(lerp(t0, j, fw), HALF))
    x1, x2 = lerp(t1, i2, f2), lerp(t2, i2, f2)
    g = s.fadd(u, u)
    kc = s.clip(s.fadd(s.fmul(s.ld(s.st(g)), K_GAIN), QUARTER), ONE)
    x1x2 = s.fadd(x1, x1)
    h = s.fsub(ONE, s.fadd(s.fmul(x1x2, x1x2) * QUARTER, s.fmul(x2, x1x2) * HALF))
    kh = s.fmul(s.ld(s.st(kc)), h)
    words = {
        k: s.st(v_) for k, v_ in {"h": h, "g": g, "v": v, "kc": kc, "x1x2": x1x2, "x2": x2}.items()
    }
    return words, kh


class Lane:
    """The five state words of one lane (`a d b c e` = frame -10, -8, -6, -4, -2)."""

    def __init__(self, words: list[int] | None = None) -> None:
        self.a, self.d, self.b, self.c, self.e = words or [0] * 5

    def words(self) -> list[int]:
        return [self.a, self.d, self.b, self.c, self.e]


def process(lane: Lane, w_r: int, w_pos: int, xs: list[int]) -> tuple[list[int], dict[str, int]]:
    """One call: `(output words, coefficient words)` for the input words `xs`."""
    words, kh = call_coefficients(w_r, w_pos)
    g, v = s.ld(words["g"]), s.ld(words["v"])
    x1x2, x2 = s.ld(words["x1x2"]), s.ld(words["x2"])
    out = []
    for xw in xs:
        x = s.ld(xw)
        a, d0, b, c, e = (s.ld(w) for w in lane.words())
        p = s.fadd(b, s.fmul(a, g))
        e_new = s.fadd(e, s.fmul(c, x1x2))
        avg = s.fadd(d0, x) / 2
        m = s.fsub(s.fsub(avg, p), s.fmul(a, v))
        n = s.fsub(s.fsub(s.fmul(x, kh), e_new), s.fmul(c, x2))
        y = s.fadd(n, s.fadd(b, p) / 2)
        c_new = s.clip(s.fadd(c, s.fmul(n, x1x2)), ONE)
        a_new = s.clip(s.fadd(s.fmul(m, g), a), ONE)
        lane.a, lane.d, lane.b, lane.c, lane.e = (s.st(w) for w in (a_new, x, p, c_new, e_new))
        out.append(s.st(y))
    return out, words


def random_case(seed: int) -> dict:
    """Random frame words and input samples of one trial (own generator per seed)."""
    rng = random.Random(seed)
    amp = rng.choice([0.3, 1.0, 3.0])
    return {
        "w_r": o.f2u(rng.uniform(0.02, 1.2)),
        "w_pos": (rng.randrange(0, 128) << 24) | rng.randrange(0, 1 << 24),
        "states": [[o.f2u(rng.uniform(-1, 1)) for _ in STATE_OFFS] for _ in range(2)],
        "xs": [[o.f2u(rng.uniform(-amp, amp)) for _ in range(4 * PASSES)] for _ in range(2)],
    }


def model_run(case: dict) -> dict:
    """Model result for a case: per lane outputs, final states and coefficient words."""
    lanes = []
    for lane_i in (0, 1):
        ln = Lane(list(case["states"][lane_i]))
        words: list[int] = []
        coefs: dict[str, int] = {}
        for p in range(PASSES):
            out, coefs = process(
                ln, case["w_r"], case["w_pos"], case["xs"][lane_i][4 * p : 4 * p + 4]
            )
            words += out
        lanes.append({"out": words, "states": ln.words(), "coefs": [coefs[k] for k in COEF_KEYS]})
    return {"lanes": lanes}


def oracle_run(sharc_org: str, image: str, case: dict) -> dict:  # pragma: no cover - needs firmware
    """Oracle result of a case, same shape as `model_run`."""
    patched = Path(tempfile.gettempdir(), "core2_t3_patched.bin")
    patched.write_bytes(o.patch_controls(Path(image).read_bytes()))
    frame = {-30: (case["w_r"], case["w_r"]), -28: (case["w_pos"], case["w_pos"])}
    for k, off in enumerate(STATE_OFFS):
        frame[off] = (case["states"][0][k], case["states"][1][k])
    xs = [o.u2f(w) for w in case["xs"][0]]
    ys = [o.u2f(w) for w in case["xs"][1]]
    res = o.run_stage(sharc_org, str(patched), STAGE, frame, xs, ys, watch_frame=(-32, 54))
    fr = res["frames"][-1]
    lanes = []
    for lane, series in ((0, res["x"]), (1, res["y"])):
        lanes.append(
            {
                "out": [o.f2u(v) for v in series],
                "states": [fr[off][lane] for off in STATE_OFFS],
                "coefs": [fr[off][lane] for off in COEF_OFFS],
            }
        )
    return {"lanes": lanes}


def compare(sharc_org: str, image: str, trials: int) -> tuple[int, int]:  # pragma: no cover
    """`(equal, total)` words of model against oracle."""
    equal = total = 0
    for seed in range(1, trials + 1):
        case = random_case(seed)
        want, got = model_run(case), oracle_run(sharc_org, image, case)
        for lane in (0, 1):
            for key in ("out", "states", "coefs"):
                for a, b in zip(want["lanes"][lane][key], got["lanes"][lane][key], strict=True):
                    total += 1
                    equal += a == b
    return equal, total


def emit_tables(image: str) -> str:  # pragma: no cover - needs firmware
    """The tables CSV text read from the ORG core 2 image."""
    data = Path(image).read_bytes()
    lines = ["index,t0_0xB0A9F,t1_0xB0C0D,t2_0xB0C8E"]
    for i in range(TABLE_LEN):
        words = [struct.unpack_from("<I", data, (a + i) * 4)[0] for a in TABLE_ADDRS]
        lines.append(f"{i}," + ",".join(f"0x{w:08x}" for w in words))
    return NL.join(lines) + NL


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - thin CLI
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--image", required=True)
    ap.add_argument("--sharc-org", default="target/release/sharc-org")
    ap.add_argument("--trials", type=int, default=40)
    ap.add_argument("--record", type=Path, help="write recorded cases (seeds 1..6) to this file")
    ap.add_argument("--emit-tables", type=Path, help="write the derived tables CSV and exit")
    ns = ap.parse_args(argv)
    if ns.emit_tables:
        ns.emit_tables.write_text(emit_tables(ns.image), encoding="utf-8", newline=NL)
        return 0
    if ns.record:
        cases = []
        for seed in range(1, 7):
            case = random_case(seed)
            case["oracle"] = oracle_run(ns.sharc_org, ns.image, case)
            cases.append(case)
        ns.record.write_text(
            json.dumps({"stage": "0x1257b2", "cases": cases}) + NL, encoding="utf-8"
        )
        return 0
    equal, total = compare(ns.sharc_org, ns.image, ns.trials)
    print(f"{equal} of {total} words equal")
    return 0 if equal == total else 1


if __name__ == "__main__":
    sys.exit(main())
