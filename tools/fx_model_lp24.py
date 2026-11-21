#!/usr/bin/env python3
"""Reference model of the Amp LP24 / HP24 routine `0x124EFA` (core 2 copy `0x124586`).

Amp types 8 (LP24) and 9 (HP24) install this routine on the Amp unit's FX chain; the mode word
at `CTX-4` selects LP (0) or HP (1) (`docs/research/amp-types-789.md`). The model is the float
form read off the routine and checked against `sharc-fx`; `record` stores oracle words so the
tests replay without firmware.

    python tools/fx_model_lp24.py check  --image core2_sram.bin   # model against the oracle
    python tools/fx_model_lp24.py record --image core2_sram.bin   # rewrite the recorded cases

Context words are floats at `CTX + offset` (offset -32 .. -1); `L = c[-25]`, `c[-24]`, `c[-23]`
are template constants whose real values are not captured (the check feeds assumed ones).
"""

from __future__ import annotations

import argparse
import json
import random
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

CASES = HERE.parent / "docs/research/data/fx-lp24-oracle-cases.json"
PC, END = 0x124586, 0x124696  # core 2 copy; core 1 copy is 0x124EFA
IN, CTX = 0xB1000, 0xB2040
SETS = {"I0": IN, "I1": CTX, "M0": 0, "M2": 2, "M9": 1}
#: Coefficient smoothing factor (`0x3f7da39e`), the same K as the other FX filters.
K = struct.unpack("<f", struct.pack("<I", 0x3F7DA39E))[0]
HP, LP = 1, 0


def bits(x: float) -> int:
    return struct.unpack("<I", struct.pack("<f", x))[0]


def f32(w: int) -> float:
    return struct.unpack("<f", struct.pack("<I", w))[0]


def model(ctx: dict[int, float], frames: list[float], mode: int, lane: int = 0):
    """One channel of the routine: `(outputs, states, (A2, B2, g2, a1, b))`.

    `lane` 0 is the left channel (states at even offsets), 1 the right (odd offsets).

    `ctx` maps offset to float. Cutoff words `c[-32]`, `c[-31]` and resonance words `c[-29]`,
    `c[-27]` smooth toward their targets with K; `c[-26]` smooths toward `c[-28]`.
    """
    c = ctx.__getitem__
    ta, a, tb, b_ = c(-32), c(-31), c(-29), c(-27)
    a2 = ta + K * (a - ta)
    b2 = tb + K * (b_ - tb)
    x_c = max(-4.0, min(4.0, 16 * a2))
    c26 = c(-28) + K * (c(-26) - c(-28))
    g2 = 2 * c26
    lim, c24, c23 = c(-25), c(-24), c(-23)
    inv = 1 / (lim + 4 * x_c * b2 + x_c * x_c) ** 0.5
    a1 = 2 * x_c * inv
    b = 4 * b2 * inv
    s = {k: c(k + lane) for k in (-16, -14, -12, -10, -8, -6)}

    def clip(v: float) -> float:
        return max(-lim, min(lim, v))

    def lp_stage(x: float, g: float, k1: int, k2: int, k3: int) -> float:
        f9 = x * g
        s1, s2, s3 = s[k1], s[k2], s[k3]
        t3 = s3 + s1 * a1
        e = (f9 + s2) / 2 - t3 - s1 * b
        s[k3], s[k2], s[k1] = t3, f9, clip(s1 + e * a1)
        return (t3 + s3) / 2

    h = lim - (b * a1 * c24 + a1 * a1 * c23)

    def hp_stage(x: float, k1: int, k2: int) -> tuple[float, float]:
        s1, s2 = s[k1], s[k2]
        f13 = s2 + s1 * a1
        f9 = x * h - f13
        e = f9 - s1 * b
        s[k2], s[k1] = f13, clip(s1 + a1 * e)
        return f9, e

    out = []
    for x in frames:
        if mode == LP:
            y = lp_stage(lp_stage(x, g2, -16, -14, -12), 1.0, -10, -8, -6)
        else:
            _, e1 = hp_stage(x, -16, -14)
            y, _ = hp_stage(e1, -10, -8)
        out.append(y)
    return out, s, (a2, b2, g2, a1, b)


def random_case(seed: int, mode: int) -> dict:
    """Scenario `{mem: {addr: word}, ctx, frames}` (left lane frames only matter for the model)."""
    rng = random.Random(seed)
    ctx = {o: rng.uniform(-0.5, 0.5) for o in range(-32, 0)}
    ctx[-32], ctx[-31] = rng.uniform(0.0, 0.05), rng.uniform(0.0, 0.05)
    ctx[-29], ctx[-27] = rng.uniform(0.0, 0.5), rng.uniform(0.0, 0.5)
    ctx[-26] = rng.uniform(0.3, 1.0)
    ctx[-25], ctx[-24], ctx[-23] = 1.0, 0.5, 0.25  # ASSUMED template constants
    frames = [(rng.uniform(-0.5, 0.5), rng.uniform(-0.5, 0.5)) for _ in range(4)]
    mem = {CTX + o: bits(v) for o, v in ctx.items()}
    mem[CTX - 3], mem[CTX - 4] = 2, mode
    for i, (left, right) in enumerate(frames):
        mem[IN + 2 * i], mem[IN + 2 * i + 1] = bits(left), bits(right)
    return {"mode": mode, "mem": mem, "ctx": ctx, "frames": frames}


def model_from_mem(mem: dict, mode: int) -> dict[int, float]:
    """Model outputs (both channels, 4 frames) as floats keyed by address, from a word image."""
    mem = {int(a): int(w) for a, w in mem.items()}
    ctx = {o: f32(mem[CTX + o]) for o in range(-32, 0)}
    exp = {}
    for lane in (0, 1):
        frames = [f32(mem[IN + 2 * i + lane]) for i in range(4)]
        out, _, _ = model(ctx, frames, mode, lane)
        exp.update({IN + 2 * i + lane: v for i, v in enumerate(out)})
    return exp


def run_oracle(image: str, binary: Path, cases: list[dict]) -> list[dict[int, int]]:
    import fx_biquad_check as fb
    import fx_oracle as fo

    rt = fo.Routine("lp24", PC, END, SETS, None, None)  # type: ignore[arg-type]
    dumps = set(range(IN, IN + 8)) | set(range(CTX - 32, CTX))
    out = []
    for mode in (LP, HP):
        sel = [c for c in cases if c["mode"] == mode]
        text = fo.run_oracle(
            image, binary, rt, [{int(a): w for a, w in c["mem"].items()} for c in sel], dumps
        )
        out.extend(fb.parse_dumps(text))
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("cmd", choices=["check", "record"])
    ap.add_argument("--image", required=True)
    ap.add_argument("--binary", default="target/release/sharc-fx.exe")
    ap.add_argument("--seeds", type=int, default=4)
    args = ap.parse_args()
    cases = [random_case(s, m) for m in (LP, HP) for s in range(args.seeds)]
    got = run_oracle(args.image, Path(args.binary), cases)
    worst = 0.0
    recs = []
    for case, g in zip(cases, got, strict=True):
        exp = model_from_mem(case["mem"], case["mode"])
        err = max(abs(f32(g[a]) - v) for a, v in exp.items())
        worst = max(worst, err)
        recs.append({"mode": case["mode"], "mem": {str(a): w for a, w in case["mem"].items()},
                     "oracle": {str(a): g[a] for a in sorted(g) if IN <= a < IN + 8}})  # fmt: skip
    print(f"{len(cases)} cases, worst output error {worst:.3g}")
    if args.cmd == "record":
        CASES.write_text(json.dumps({"cases": recs}, indent=1) + "\n", encoding="utf-8")
    return 0 if worst < 1e-5 else 1


if __name__ == "__main__":
    raise SystemExit(main())
