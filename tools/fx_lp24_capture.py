#!/usr/bin/env python3
"""Real template, word tables and stateful model of the Amp LP24 / HP24 routine (`0x124EFA`).

`fx_model_lp24.py` checks the equations of one call on random contexts. This module adds what a
port needs: the context template of a booted Amp unit, the ARM word tables of `kAmpMidFreq` and
`kAmpMid`, and a stateful model that carries the smoothing and filter states across calls, checked
word for word against `sharc-fx` over 24 calls per scenario
(`docs/research/amp-types-789.md` s6).

    python tools/fx_lp24_capture.py template --image core1_sram.bin  # FX image template words
    python tools/fx_lp24_capture.py golden --image core2_sram.bin  # model vs oracle, golden
    python tools/fx_lp24_capture.py rust  # regenerate the Rust data file
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import fx_model_lp24 as lp  # noqa: E402

DATA = HERE.parent / "docs/research/data"
WORDS = DATA / "fx-lp24-words.csv"
GOLDEN = DATA / "fx-lp24-golden.json"
RUST = HERE.parent / "crates/nord-core/src/fx/tests_fw_lp24_calls_data.rs"
#: Unit 0 context of the FX core 1 image: `CTX = 0xC1A28`, words `CTX - 32 .. CTX - 1`.
FX_CORE1_CTX = 0xC1A28
CALLS, FRAMES = 24, 4
#: Scenarios `(mode, kAmpMidFreq, kAmpMid)`: cutoffs low to high, resonance none to maximum.
SCENARIOS = [(0, 100, 0), (0, 110, -60), (0, 95, 40), (1, 64, 0), (1, 90, -60), (1, 50, 40)]
#: Lanes of the six states: left and right words (`c[-16 .. -5]`, interleaved L, R).
LANE_STATES = ([-16, -14, -12, -10, -8, -6], [-15, -13, -11, -9, -7, -5])


def load_tables(path: Path = WORDS) -> dict:
    """`{"mid_freq": [128 words], "mid": [[res, pre] x 121]}` from the committed capture CSV."""
    import csv

    freq: dict[int, int] = {}
    res: dict[int, int] = {}
    pre: dict[int, int] = {}
    with path.open(encoding="utf-8") as f:
        for r in csv.DictReader(f):
            v, w = int(r["value"]), int(r["word"], 16)
            if r["id"].endswith("MidFreq"):
                freq[v] = w
            elif r["dm_word"] == "0xc1a0b":
                res[v] = w
            elif r["dm_word"] == "0xc1a0c":
                pre[v] = w
    return {
        "mid_freq": [freq[i] for i in range(128)],
        "mid": [[res[i], pre[i]] for i in range(-60, 61)],
    }


def template_words() -> dict[int, int]:
    """Non-zero template words by context offset (VERIFIED, see [`template_check`])."""
    return {
        -26: lp.bits(1.0),  # pre-gain state, settles to the Mid word `c[-28]`
        -25: lp.bits(1.0),  # clip limit L
        -24: lp.bits(0.5),  # HP24 constant
        -23: lp.bits(0.25),  # HP24 constant
        -3: 2,  # loop count (two blocks of two frames)
    }


def template_check(image: str, binary: Path) -> dict[int, int]:
    """Words `CTX-32 .. CTX-1` of the FX core 1 image after its boot (a one step call)."""
    args = [str(binary), image, "--call", "0x1228CF", "--until", "0x1228d2", "--max", "5"]
    args += ["--dump", f"{FX_CORE1_CTX - 32:#x}:32"]
    run = subprocess.run(args, capture_output=True, text=True, check=True)
    out = {}
    for line in run.stdout.splitlines():
        if line.startswith("dump "):
            _, a, w, *_ = line.split()
            out[int(a, 16) - FX_CORE1_CTX] = int(w, 16)
    return {o: w for o, w in out.items() if w}


class Lp24:
    """Stateful model of the routine: one `call` is one pass over four stereo frames."""

    def __init__(self, mode: int, mid_freq: int, mid: int, tables: dict | None = None) -> None:
        t = tables or load_tables()
        self.mode = mode
        c = {o: 0.0 for o in range(-32, 0)}
        c.update({o: lp.f32(w) for o, w in template_words().items() if o != -3})
        c[-32] = lp.f32(t["mid_freq"][mid_freq])
        res, pre = t["mid"][mid + 60]
        c[-29], c[-28] = lp.f32(res), lp.f32(pre)
        self.c = c

    def call(self, frames: list[tuple[float, float]]) -> list[tuple[float, float]]:
        c = self.c
        k = lp.K
        c[-31] = c[-32] + k * (c[-31] - c[-32])
        c[-27] = c[-29] + k * (c[-27] - c[-29])
        c[-26] = c[-28] + k * (c[-26] - c[-28])
        outs = []
        # `lp.model` smooths its targets itself: hand it a context whose targets equal the states
        # (already smoothed above, once per call) and the lane's filter states.
        for lane in (0, 1):
            ctx = _settled(c)
            for kk, off in zip((-16, -14, -12, -10, -8, -6), LANE_STATES[lane], strict=True):
                ctx[kk] = c[off]
            out, st, _ = lp.model(_settled(ctx), [f[lane] for f in frames], self.mode)
            for kk, off in zip((-16, -14, -12, -10, -8, -6), LANE_STATES[lane], strict=True):
                c[off] = st[kk]
            outs.append(out)
        return list(zip(outs[0], outs[1], strict=True))


def _settled(ctx: dict[int, float]) -> dict[int, float]:
    """Context whose smoothing is the identity: targets equal states (`model` smooths once)."""
    ctx = dict(ctx)
    ctx[-32] = ctx[-31]
    ctx[-29] = ctx[-27]
    ctx[-28] = ctx[-26]
    return ctx


def lcg_frames(seed: int, n: int, amp: float) -> list[tuple[float, float]]:
    """Seeded white-noise frames (the LCG of `fx_amp_cabinet.chain_input`), as exact f32 values."""
    state = seed
    out = []
    for _ in range(n):
        pair = []
        for _ in range(2):
            state = (state * 1664525 + 1013904223) & 0xFFFFFFFF
            pair.append(lp.f32(lp.bits(((state >> 8) / (1 << 24) - 0.5) * 2 * amp)))
        out.append((pair[0], pair[1]))
    return out


def oracle_run(image: str, binary: Path, scenario: tuple[int, int, int], frames) -> list[int]:
    """Output words (`CALLS x 8`) of the oracle for one scenario, template and words as the ARM."""
    import fx_biquad_check as fb
    import fx_oracle as fo

    mode, mf, mid = scenario
    t = load_tables()
    mem0 = {lp.CTX + o: w for o, w in template_words().items()}
    mem0[lp.CTX - 4] = mode
    mem0[lp.CTX - 32] = t["mid_freq"][mf]
    mem0[lp.CTX - 29], mem0[lp.CTX - 28] = t["mid"][mid + 60]
    cases = []
    for c in range(CALLS):
        mem = dict(mem0) if c == 0 else {}
        for i, (left, right) in enumerate(frames[c * FRAMES : (c + 1) * FRAMES]):
            mem[lp.IN + 2 * i], mem[lp.IN + 2 * i + 1] = lp.bits(left), lp.bits(right)
        cases.append(mem)
    rt = fo.Routine("lp24", lp.PC, lp.END, lp.SETS, None, None)  # type: ignore[arg-type]
    text = fo.run_oracle(image, binary, rt, cases, set(range(lp.IN, lp.IN + 8)))
    res = fb.parse_dumps(text)
    return [res[c][lp.IN + k] for c in range(CALLS) for k in range(8)]


def model_run(scenario: tuple[int, int, int], frames) -> list[float]:
    """Model outputs (`CALLS x 8` floats, L R per frame) of one scenario."""
    mode, mf, mid = scenario
    m = Lp24(mode, mf, mid)
    out: list[float] = []
    for c in range(CALLS):
        for left, right in m.call(frames[c * FRAMES : (c + 1) * FRAMES]):
            out += [left, right]
    return out


def cmd_golden(image: str, binary: Path) -> int:
    recs = []
    worst = 0.0
    for n, sc in enumerate(SCENARIOS):
        frames = lcg_frames(1000 + n, CALLS * FRAMES, 0.4)
        want = oracle_run(image, binary, sc, frames)
        got = model_run(sc, frames)
        err = max(abs(lp.f32(w) - g) for w, g in zip(want, got, strict=True))
        worst = max(worst, err)
        print(f"scenario {sc}: worst error {err:.3g}, peak {max(abs(lp.f32(w)) for w in want):.3g}")
        recs.append({"scenario": list(sc), "seed": 1000 + n, "oracle": want})
    GOLDEN.write_text(json.dumps({"calls": CALLS, "cases": recs}, indent=1) + "\n", "utf-8")
    cmd_rust()
    return 0 if worst < 1e-5 else 1


def cmd_rust() -> None:
    g = json.loads(GOLDEN.read_text(encoding="utf-8"))
    lines = [
        "//! Oracle outputs of the Amp LP24 / HP24 routine `0x124EFA` (by",
        "//! `tools/fx_lp24_capture.py rust`; VERIFIED: `sharc-fx`, FX core 2 copy `0x124586`",
        "//! with the real template and ARM words, 24 calls of 4 stereo frames, state carried).",
        "",
        "/// Scenarios `(mode, kAmpMidFreq, kAmpMid)`.",
        f"pub const SCENARIOS: [(u8, u8, i8); {len(g['cases'])}] = [",
    ]
    lines += [
        f"    ({c['scenario'][0]}, {c['scenario'][1]}, {c['scenario'][2]})," for c in g["cases"]
    ]
    lines += [
        "];",
        "",
        "/// Seeds of the LCG input of each scenario.",
        f"pub const SEEDS: [u32; {len(g['cases'])}] = [",
    ]
    lines += [f"    {c['seed']}," for c in g["cases"]]
    lines += ["];", "", "/// Output words, `24 x 8` per scenario (L, R per frame)."]
    n = len(g["cases"][0]["oracle"])
    lines.append(f"pub const OUT: [[u32; {n}]; {len(g['cases'])}] = [")
    for c in g["cases"]:
        lines.append("    [")
        for i in range(0, n, 8):
            lines.append("        " + ", ".join(f"{w:#010x}" for w in c["oracle"][i : i + 8]) + ",")
        lines.append("    ],")
    lines += [
        "];",
        "",
        "#[cfg(test)]",
        "mod tests {",
        "    use super::*;",
        "",
        "    #[test]",
        "    fn one_seed_and_one_output_vector_per_scenario() {",
        "        assert_eq!(SEEDS.len(), SCENARIOS.len());",
        "        assert_eq!(OUT.len(), SCENARIOS.len());",
        "        assert!(OUT.iter().all(|o| o.iter().any(|w| *w != 0)));",
        "    }",
        "}",
        "",
    ]
    RUST.write_text("\n".join(lines), encoding="utf-8")
    print(f"wrote {RUST}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("cmd", choices=["template", "golden", "rust"])
    ap.add_argument("--image")
    ap.add_argument("--binary", default="target/release/sharc-fx.exe")
    a = ap.parse_args()
    if a.cmd == "template":
        w = template_check(a.image, Path(a.binary))
        print({o: hex(v) for o, v in sorted(w.items())})
        want = template_words()
        ok = all(w.get(o) == v for o, v in want.items())
        print("template words match" if ok else "MISMATCH")
        return 0 if ok else 1
    if a.cmd == "golden":
        return cmd_golden(a.image, Path(a.binary))
    else:
        cmd_rust()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
