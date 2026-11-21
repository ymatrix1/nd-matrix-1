#!/usr/bin/env python3
r"""Terms of the ORG core 2 cutoff index sum: oracle measurement and models (D204).

The pass control block of channel 0 (pc `0x126284..0x126328` of the ORG image, listing
`sharc-disasm IMAGE 0x24C508 0x24C5DC`) builds the y-lane cutoff index sum

```text
idx = w3 + m0y + ((w5 + m1y) * lfo << 2) + (2 * w7 * FIX(F-4, 30)) + (DM(0xC1E31) * DM(F+27))
      w3, w5, w7 = strip 0 words 3, 5, 7  (smoothed toward common 0 words 1, 3, 5, tau 256 passes)
      m0y, m1y   = DM(0xC2717), DM(0xC2719)  (key tracking offset, second modulation input)
      lfo        = DM(0xC1E37)  (LFO stage output)
      F-4        = DM(0xC1184)  (float: filter envelope level times amount)
      all words Q31 with one index unit = 2^24 (a semitone); CLIP BY DM(0xC147A)
```

and maps it to `R = DM(0xC2925)`: the exponential stage law (`atan(R)` doubles every 12 index
units, `fc = 13.75 Hz 2^(knob / 12)`) for every stage but the ladder; the ladder stage `0x12549D`
reads the linear ramp table (`synth_adr_model.cutoff_r`) as its cutoff position. This tool injects
single terms into a loaded program and compares the oracle `R` in index units with the models
below; `measure` writes `docs/research/data/synth-cutoff-terms-oracle.csv`.

    cargo build --release -p nord-sharc --bin sharc-sys
    uv run python tools/synth_cutoff_terms.py measure --sharc-sys target/release/sharc-sys \
        --image DIR/core2_sram.bin --work12 target/synth_env/work_12dB_Sweep \
        --work80 target/synth_env/work_80s_Sci-Fi \
        --out docs/research/data/synth-cutoff-terms-oracle.csv

`--workdirN` hold `load.script`, `adr.script`, `key.script` of `synth_env_trace.py report`.
"""

from __future__ import annotations

import argparse
import csv
import math
import subprocess
import sys
from pathlib import Path

import numpy as np
import synth_adr_model as adr
import synth_env_trace as t

# VERIFIED (this tool, rows `lfo`): one index unit is `2^24` of a Q31 word; the LFO product is
# shifted left by 2 (`ASHIFT BY 2`) so `semitones = 512 * depth * lfo` for Q31 fractions.
LFO_SEMITONES_PER_UNIT = 512.0
# VERIFIED (ARM run, `synth_lfo_words.py`): `kLFODst` 3 (cutoff) writes common word 3 =
# `amt / 127 * 0x40000000` (amt 32, 64, 127 -> 0x10204080, 0x20408100, 0x40000000).
LFO_DEPTH_FULL_WORD = 0x40000000
# VERIFIED (this tool, rows `env`): `2 * w7 * FIX(F-4, 30)` with `w7 = 0x7F000000` (image default)
# gives `127 * F-4` index units.
ENV_W7_DEFAULT = 0x7F000000
# Common 0 word `k` smooths into strip 0 word `k + 2` (`w <- 0.99609375 w + 0.0048828 c`).
COMMON_LFO_DEPTH = 0xE02EB  # common word 3 -> strip word 5 (y lane)
COMMON_ENV_GAIN = 0xE02ED  # common word 5 -> strip word 7 (y lane)
M0Y, M1Y, LFO_Y, ENV_F4 = 0xC2717, 0xC2719, 0xC1E37, 0xC1184
STRIP_WORD1 = 0xE0001
STAGE_LP12, STAGE_LADDER = 0x861253BD, 0x8612549D
SETTLE = 2500  # strip word smoothing: tau 256 passes
KNOB_HZ0 = 13.75


def lfo_depth_word(amt: int) -> int:
    """Common word 3 for a `kLFOAmt` knob `amt` 0..127 with the destination on the cutoff."""
    return round(min(max(amt, 0), 127) / 127.0 * LFO_DEPTH_FULL_WORD)


def q31(word: int) -> float:
    """Signed Q31 value of a 32-bit word."""
    w = word & 0xFFFFFFFF
    return (w - (1 << 32) if w & 0x80000000 else w) / 2147483648.0


def lfo_semitones(depth_word: int, lfo_word: int) -> float:
    """Index units of the LFO term for a depth word (strip word 5 steady state) and LFO word."""
    return LFO_SEMITONES_PER_UNIT * q31(depth_word) * q31(lfo_word)


def env_semitones(f4: float, w7_word: int = ENV_W7_DEFAULT) -> float:
    """Index units of the envelope term for `F-4` (level times amount, float) and strip word 7."""
    return 2.0 * 128.0 * q31(w7_word) * 0.5 * f4


def knob_of_exp_r(r: float) -> float:
    """Cutoff index in knob units of an exponential-law `R` (`fc = 13.75 Hz 2^(knob / 12)`)."""
    fc = t.r_to_hz(r, t.ORACLE_STAGE_RATE_HZ)
    return 12.0 * math.log2(fc / KNOB_HZ0)


def knob_of_ramp_r(r: float) -> float:
    """Cutoff index in knob units of a ladder `R` (inverse of the linear ramp, knob = 52 + i)."""
    return 52.0 + (r - adr.R_BASE) / adr.R_STEP


def ramp_r_of_knob(knob: float) -> float:
    """Ladder `R` of a cutoff knob index: the ramp table at word `(knob - 52) << 24`."""
    return adr.cutoff_r(int(round((knob - 52.0) * (1 << 24))) & 0xFFFFFFFF)


def exp_r_of_knob(knob: float) -> float:
    """Exponential-law `R` of a cutoff knob index (`tan(pi fc / 2F)`, capped at 1.99)."""
    fc = KNOB_HZ0 * 2.0 ** (knob / 12.0)
    return min(math.tan(math.pi * fc / t.ORACLE_STAGE_RATE_HZ), 1.99)


def every_pass(pairs: list[tuple[int, int]], passes: int) -> str:
    """Script lines that write `(addr, word)` before every pass."""
    return "".join(f"{p} {a:#x} {v & 0xFFFFFFFF:#010x}\n" for p in range(passes) for a, v in pairs)


class Oracle:
    """`sharc-sys` runs of one loaded program (scripts of a `synth_env_trace report` workdir)."""

    def __init__(self, sharc_sys: str, image: str, work: Path, scratch: Path) -> None:
        self.sys, self.image, self.work, self.scratch = sharc_sys, image, Path(work), scratch
        self.scratch.mkdir(parents=True, exist_ok=True)

    def run(self, writes: list[tuple[int, int]], *, hold: list[tuple[int, int]] = (), key=False,
            passes: int = 2600, taps: tuple[int, ...] = ()) -> np.ndarray:  # fmt: skip
        """Columns `R` then `taps` as float32 per pass. `writes` once at pass 0, `hold` per pass."""
        name = f"{self.work.name}_{abs(hash((tuple(writes), tuple(hold), key, taps)))}"
        extra = self.scratch / f"{name}.ex"
        extra.write_text(
            t.inject_script(dict(writes)) + every_pass(list(hold), passes), encoding="utf-8"
        )
        keyf = self.scratch / f"{name}.key"
        text = (self.work / "key.script").read_text(encoding="utf-8")
        keyf.write_text(text if key else "", encoding="utf-8")
        out = self.scratch / f"{name}.bin"
        cmd = [self.sys, self.image, "--passes", str(passes), "--warmup", "0", "--epdma"]
        cmd += ["--script-flat", str(self.work / "load.script")]
        for f in (self.work / "adr.script", keyf, extra):
            cmd += ["--script-at", f"0:{f}"]
        addrs = (t.R_TAP, *taps)
        for a in addrs:
            cmd += ["--tap", f"{a:#x}:1"]
        cmd += ["--tap-out", str(out)]
        r = subprocess.run(cmd, capture_output=True, text=True, check=False)
        if "tap:" not in r.stdout:
            raise RuntimeError(r.stdout[-500:] + r.stderr[-300:])
        raw = np.fromfile(out, dtype="<u4").reshape(-1, len(addrs))
        return raw.view("<f4").astype(np.float64)


def row(kind: str, prog: str, stage: str, inputs: str, oracle: float, model: float) -> dict:
    return {"kind": kind, "program": prog, "stage": stage, "inputs": inputs,
            "oracle": round(oracle, 4), "model": round(model, 4),
            "error": round(oracle - model, 4)}  # fmt: skip


CSV_FIELDS = ("kind", "program", "stage", "inputs", "oracle", "model", "error")


def measure_rows(o12: Oracle, o80: Oracle) -> list[dict]:  # pragma: no cover - needs firmware
    """Every term row; `oracle` and `model` are index units (knob semitones) of the term."""
    rows: list[dict] = []
    base = knob_of_exp_r(o12.run([])[SETTLE, 0])
    for amt in (32, 64, 127):
        for lfo in (0x0CCCCCCD, 0xF3333333, 0x1999999A, 0xE6666666):
            w = lfo_depth_word(amt)
            r = o12.run([(COMMON_LFO_DEPTH, w)], hold=[(LFO_Y, lfo)])[SETTLE, 0]
            model = lfo_semitones(w, lfo)
            rows.append(row("lfo", "12dB Sweep", "lp12", f"amt={amt} lfo={lfo:#010x}",
                            knob_of_exp_r(r) - base, model))  # fmt: skip
    for m0 in (-24, -12, -5, 0, 7, 12):
        r = o12.run([], hold=[(M0Y, (m0 << 24) & 0xFFFFFFFF)])[SETTLE, 0]
        rows.append(row("m0", "12dB Sweep", "lp12", f"m0={m0}", knob_of_exp_r(r) - base, m0))
    for w7 in (ENV_W7_DEFAULT, 0x3F000000, 0x1F800000):
        for f4 in (0.25, 0.5):
            f4w = int(np.float32(f4).view(np.uint32))
            r = o12.run([(COMMON_ENV_GAIN, w7)], hold=[(ENV_F4, f4w)])[SETTLE, 0]
            rows.append(row("env", "12dB Sweep", "lp12", f"w7={w7:#010x} F-4={f4}",
                            knob_of_exp_r(r) - base, env_semitones(f4, w7)))  # fmt: skip
    for byte in (0, 16, 64, 127):
        r = o12.run([(STRIP_WORD1, ((0x80 | byte) << 24) | 0x1253BD)])[SETTLE, 0]
        rows.append(row("drive", "12dB Sweep", "lp12", f"word1_byte={byte}",
                        knob_of_exp_r(r) - base, 0.0))  # fmt: skip
    for knob in (30, 60, 81, 100):
        word = t.cutoff_word(knob)
        for stage, name, key in ((STAGE_LADDER, "ladder", False), (STAGE_LP12, "lp12", False)):
            r = o80.run([(t.COMMON_CUTOFF, word), (STRIP_WORD1, stage)], key=key)[SETTLE, 0]
            ladder = stage == STAGE_LADDER
            model = ramp_r_of_knob(knob) if ladder else exp_r_of_knob(knob)
            rows.append(row("r_law", "80s Sci-Fi", name, f"knob={knob}", r, model))
        r = o80.run([(t.COMMON_CUTOFF, word), (t.COMMON_FLT_ENV_AMOUNT, 0)], key=True)[2400, 0]
        rows.append(row("r_law_key", "80s Sci-Fi", "ladder", f"knob={knob} note_offset=-12", r,
                        ramp_r_of_knob(knob - 12)))  # fmt: skip
    return rows


def measure(ns: argparse.Namespace) -> int:  # pragma: no cover - needs local firmware images
    scratch = Path(ns.scratch)
    o12 = Oracle(ns.sharc_sys, ns.image, Path(ns.work12), scratch)
    o80 = Oracle(ns.sharc_sys, ns.image, Path(ns.work80), scratch)
    rows = measure_rows(o12, o80)
    with Path(ns.out).open("w", encoding="utf-8", newline="") as f:
        w = csv.DictWriter(f, fieldnames=CSV_FIELDS, lineterminator="\n")
        w.writeheader()
        w.writerows(rows)
    worst = max(abs(r["error"]) for r in rows if r["kind"] not in ("r_law", "r_law_key"))
    print(f"{len(rows)} rows, worst index error {worst:.3f}", file=sys.stderr)
    return 0


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - CLI wiring
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("measure")
    for a in ("--sharc-sys", "--image", "--work12", "--work80", "--out"):
        p.add_argument(a, required=True)
    p.add_argument("--scratch", default="target/synth_cutoff_terms")
    ns = ap.parse_args(argv)
    return measure(ns)


if __name__ == "__main__":
    sys.exit(main())
