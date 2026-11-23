#!/usr/bin/env python3
r"""Level link of the firmware filter envelope law: oracle cutoff peak against key velocity (D231).

`--synth-fw-env-law` (D200 / D207) is exact at the oracle for the index sum, but the engine feeds
it `level = filter chain * velocity peak`; D223 named that link ASSUMED. This tool measures it on
the ORG core 2 oracle: one run per key velocity byte `g` and per filter envelope velocity setting
(`kFltEnvVel` off: strip 0 word 19 = 1.0; on: strip 0 word 19 = 0.0 and strip 6 word 19 = 1.0, the
words the ARM writes, `synth-velocity.md` s1), with the program's own ADR / cutoff / amount words
(`synth_env_trace.program_words`), and reads per pass

* the filter chain `F - 8`, the slot value `F - 4 = level * amount` (`DM(0xC1184)`), the slot
  velocity factor `DM(0xC2723)`,
* the cutoff coefficient `R = DM(0xC2925)` and from it the cutoff index in knob semitones.

    uv run python tools/synth_env_level_link.py measure WORKDIR \
        --sharc-sys target/release/sharc-sys \
        --image DIR/core2_sram.bin --program "Smooth Lead" --amount-knob 31 --out link.csv

`WORKDIR` is the workdir of `synth_env_trace.py report` (it holds `load.script`, `adr.script`,
`key.script` of the program). Tags follow `.agents/rules/provenance-in-code.md`.
"""

from __future__ import annotations

import argparse
import csv
import subprocess
import sys
from pathlib import Path

import numpy as np
import synth_env_trace as t

# VERIFIED (synth-velocity s1): strip 0 word 19 and strip 6 word 19 (offset 391) hold the
# filter envelope velocity pair; `DM(0xC2723) = G / 128` when the pair is (0.0, 1.0).
STRIP0_W19 = 0xE0013
STRIP6_W19 = 0xE0000 + 391
SLOT_F_WORD = 0xC2723
F4_TAP = 0xC1184  # filter envelope level times amount (synth_cutoff_terms.py)
FILTER_CHAIN_TAP = t.ENV_FRAME - 8
FIELDS = (
    "program",
    "amount_knob",
    "vel_setting",
    "g",
    "chain_peak",
    "f4_peak",
    "slot_f",
    "index_before",
    "index_peak",
    "semitones_peak",
)


def velocity_words(on: bool) -> dict[int, int]:
    """Words the ARM writes for `kFltEnvVel` (on: 0.0 and 1.0; off: strip 0 word 19 = 1.0)."""
    if on:
        return {STRIP0_W19: 0x00000000, STRIP6_W19: t.f32_bits(1.0)}
    return {STRIP0_W19: t.f32_bits(1.0), STRIP6_W19: 0x00000000}


def effective_key_byte(g: int) -> int:
    """`G` of the velocity laws: the key byte `g`, but 128 for the saturated `g = 127`."""
    return 128 if g >= 127 else g


def predicted_factor(g: int, on: bool) -> float:
    """Engine `fenv_peak` at key byte `g`: `G / 128` with the filter velocity on, else 1.0."""
    return effective_key_byte(g) / 128.0 if on else 1.0


def window_peak(y: np.ndarray, on: int, off: int) -> float:
    """Largest value of `y` between the key on and off passes."""
    return float(np.max(y[on:off]))


def row(
    vel: bool, g: int, env: np.ndarray, extra: np.ndarray, r: np.ndarray, on: int, off: int
) -> dict:
    """One CSV row of a run: `env[:, 0]` filter chain, `extra[:, 0]` F-4, `extra[:, 1]` slot F."""
    idx = np.array([t.knob_of_r(x, False) for x in r])
    before = float(idx[on - 1])
    peak = float(idx[on:off].max())
    return {
        "vel_setting": int(vel),
        "g": g,
        "chain_peak": window_peak(env[:, 0], on, off),
        "f4_peak": window_peak(extra[:, 0], on, off),
        "slot_f": float(extra[off - 1, 1]),
        "index_before": before,
        "index_peak": peak,
        "semitones_peak": peak - before,
    }


def run(
    sharc_sys: str, image: str, scripts: list[Path], passes: int, out: Path
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:  # pragma: no cover - needs the local oracle
    """One `sharc-sys` run: `(filter chain [p, 1], (F-4, slot F) [p, 2], R [p])`."""
    taps = (FILTER_CHAIN_TAP, F4_TAP, SLOT_F_WORD, t.R_TAP)
    cmd = [sharc_sys, image, "--passes", str(passes), "--warmup", "0", "--epdma"]
    cmd += ["--script-flat", str(scripts[0])]
    for s in scripts[1:]:
        cmd += ["--script-at", f"0:{s}"]
    for a in taps:
        cmd += ["--tap", f"{a:#x}:1"]
    cmd += ["--tap-out", str(out)]
    res = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if "tap:" not in res.stdout:
        raise RuntimeError(res.stdout[-800:] + res.stderr[-400:])
    f = np.fromfile(out, dtype="<u4").reshape(-1, len(taps)).view("<f4").astype(np.float64)
    return f[:, :1], f[:, 1:3], f[:, 3]


def measure(ns: argparse.Namespace) -> int:  # pragma: no cover - needs the local oracle
    work = Path(ns.workdir)
    on = t.KEY_ON_PASS
    off = on + int(t.HOLD_S / t.PASS_S)
    passes = off + 200
    key_text = (work / "key.script").read_text()
    rows = []
    for vel in (False, True):
        (work / "vel.script").write_text(t.inject_script(velocity_words(vel)))
        for g in (int(x) for x in ns.gs.split(",")):
            (work / f"key_g{g}.script").write_text(t.key_byte_script(key_text, g))
            env, extra, r = run(
                ns.sharc_sys,
                ns.image,
                [
                    work / "load.script",
                    work / "adr.script",
                    work / "vel.script",
                    work / f"key_g{g}.script",
                ],
                passes,
                work / "link.bin",
            )
            rows.append(
                {"program": ns.program, "amount_knob": ns.amount_knob}
                | row(vel, g, env, extra, r, on, off)
            )
            print(rows[-1], file=sys.stderr)
    with Path(ns.out).open("w", newline="", encoding="utf-8") as fh:
        w = csv.DictWriter(fh, fieldnames=FIELDS, lineterminator="\n")
        w.writeheader()
        w.writerows(rows)
    return 0


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - CLI wiring
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    m = sub.add_parser("measure")
    m.add_argument("workdir")
    m.add_argument("--sharc-sys", required=True)
    m.add_argument("--image", required=True)
    m.add_argument("--program", required=True, help="name written to the CSV")
    m.add_argument("--amount-knob", type=int, required=True, help="kFltEnvAmt knob of the program")
    m.add_argument("--gs", default="10,32,64,98,127")
    m.add_argument("--out", required=True)
    ns = ap.parse_args(argv)
    return measure(ns)


if __name__ == "__main__":
    sys.exit(main())
