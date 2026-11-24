#!/usr/bin/env python3
r"""Edge shape of the arpeggiator gate float in the ORG core 2 layer mix (SHARC replay).

`docs/research/gate-float-consumer.md`: in Gate arp mode the ARM writes float 1.0 / 0.0 to
`DM(0xC1396, 0xC1397)`, `DM(0xC13E6, 0xC13E7)` and `DM(0xC1436, 0xC1437)`. The pass preamble reads
each pair, runs a one-pole smoother and keeps the result in `record - 8` (the word the layer mix
multiplies into the summed slot bus right before the output buffers). This tool replays the ARM
recall of a capture into `sharc-sys`, steps the gate 1 -> 0 -> 1 at fixed passes and taps the three
multiplier words after every pass, which is the gain applied to the bus after the gate.

    cargo build --release -p nord-sharc --bin sharc-sys
    python tools/gate_edge_probe.py CAPTURE.txt.gz --image DIR/core2_sram.bin \
        --sharc-sys target/release/sharc-sys --workdir target/gate_edge

Prints numbers only (one-pole coefficient, time constant, 10-90 % time, largest one-pass jump).
"""

from __future__ import annotations

import argparse
import json
import math
import subprocess
from pathlib import Path

import numpy as np

# VERIFIED (docs/research/gate-float-consumer.md s2): ARM gate words, ORG core 2 pair per list node.
GATE_WORDS = (0xC1396, 0xC1397, 0xC13E6, 0xC13E7, 0xC1436, 0xC1437)
# VERIFIED (same, s2): multiplier word = gate word + 8 (record - 8) of nodes 0, 2, 4.
GAIN_TAPS = (0xC139E, 0xC13EE, 0xC143E)
# ASSUMED (D121): a pass is 2 SY frames at 43,402.34 Hz.
FRAME_RATE = 43_402.34375
PASS_S = 2.0 / FRAME_RATE


def load_script(rows: list[dict]) -> str:
    """Flat `0 DM VALUE` lines of the ORG core 2 recall (phases load and idle), file order."""
    sel = [
        r for r in rows if r["phase"] in ("load", "idle") and r["chip"] == "ORG" and r["core"] == 2
    ]
    return "".join(f"0 {r['dm']:#x} {r['w2']:#010x}\n" for r in sel)


def gate_script(steps: list[tuple[int, float]]) -> str:
    """Gate float writes `PASS DM VALUE` for every gate word at each `(pass, level)` step."""
    lines = []
    for p, level in steps:
        bits = int(np.array([level], dtype="<f4").view("<u4")[0])
        lines += [f"{p} {dm:#x} {bits:#010x}" for dm in GATE_WORDS]
    return "\n".join(lines) + "\n"


def fit_one_pole(gain: np.ndarray, start: int, target: float) -> tuple[float, float]:
    """Least-squares `c` of `g[n+1] - g[n] = c (target - g[n])` from pass `start` on, and the
    largest absolute deviation of the one-step prediction."""
    g = gain[start:].astype(np.float64)
    x = target - g[:-1]
    y = g[1:] - g[:-1]
    keep = np.abs(x) > 1e-6
    if keep.sum() < 3:
        raise ValueError("series too short or already settled")
    c = float(np.sum(x[keep] * y[keep]) / np.sum(x[keep] ** 2))
    return c, float(np.max(np.abs(y[keep] - c * x[keep])))


def time_constant_passes(c: float) -> float:
    """Passes for the one-pole `g += c (G - g)` to cover `1 - 1/e` of a step."""
    return -1.0 / math.log(1.0 - c)


def rise_10_90(gain: np.ndarray, start: int, lo: float, hi: float) -> float:
    """Passes between the 10 % and 90 % crossings of the step lo -> hi (either direction)."""
    g = gain[start:].astype(np.float64)
    frac = (g - lo) / (hi - lo)
    i10 = int(np.argmax(frac >= 0.1))
    i90 = int(np.argmax(frac >= 0.9))
    if frac[i90] < 0.9 or frac[i10] < 0.1:
        raise ValueError("step not finished inside the series")
    return float(i90 - i10)


def max_step_fraction(gain: np.ndarray, start: int, lo: float, hi: float) -> float:
    """Largest one-pass change as a fraction of the step size (1.0 = hard step)."""
    g = gain[start:].astype(np.float64)
    return float(np.max(np.abs(np.diff(g))) / abs(hi - lo))


def measure(gain: np.ndarray, p_down: int, p_up: int) -> dict:
    """Fall (1 -> 0 at `p_down`) and rise (0 -> 1 at `p_up`) numbers for one multiplier series."""
    out = {}
    for name, series, lo, hi, target in (
        ("fall", gain[p_down:p_up], 1.0, 0.0, 0.0),
        ("rise", gain[p_up:], 0.0, 1.0, 1.0),
    ):
        start = 0
        c, dev = fit_one_pole(series, start, target)
        t = time_constant_passes(c)
        t1090 = rise_10_90(series, start, lo, hi)
        out[name] = {
            "c_per_pass": round(c, 6),
            "fit_max_dev": dev,
            "tau_passes": round(t, 2),
            "tau_ms": round(t * PASS_S * 1000, 3),
            "t10_90_passes": t1090,
            "t10_90_ms": round(t1090 * PASS_S * 1000, 3),
            "max_step_fraction": round(max_step_fraction(series, start, lo, hi), 5),
        }
    return out


def run(args: argparse.Namespace) -> dict:  # pragma: no cover - needs sharc-sys and a capture
    from arm_dsp_capture import read_rows  # local import: needs tools/ on sys.path

    work = Path(args.workdir)
    work.mkdir(parents=True, exist_ok=True)
    (work / "load.script").write_text(load_script(list(read_rows(Path(args.capture)))))
    # settle at 1.0 (smoother from 0 needs several time constants), fall, settle, rise
    p_down, p_up, total = args.settle, 2 * args.settle, 3 * args.settle
    (work / "gate.script").write_text(gate_script([(0, 1.0), (p_down, 0.0), (p_up, 1.0)]))
    cmd = [args.sharc_sys, args.image, "--passes", str(total), "--warmup", "0", "--epdma"]
    cmd += ["--script-flat", str(work / "load.script"), "--script-at", f"0:{work / 'gate.script'}"]
    for a in GAIN_TAPS:
        cmd += ["--tap", f"{a:#x}:1"]
    cmd += ["--tap-out", str(work / "gain.bin")]
    r = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if "tap:" not in r.stdout:
        raise RuntimeError(r.stdout[-600:] + r.stderr[-300:])
    words = np.fromfile(work / "gain.bin", dtype="<u4").reshape(-1, len(GAIN_TAPS))
    gain = words.view("<f4")
    return {f"{a:#x}": measure(gain[:, i], p_down, p_up) for i, a in enumerate(GAIN_TAPS)}


def main() -> None:  # pragma: no cover - CLI
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("capture", help="arm_dsp_capture / gate_dsp_trace file (.txt.gz)")
    ap.add_argument("--image", required=True, help="ORG core2_sram.bin")
    ap.add_argument("--sharc-sys", default="target/release/sharc-sys")
    ap.add_argument("--workdir", default="target/gate_edge")
    ap.add_argument("--settle", type=int, default=2000, help="passes per phase (default 2000)")
    print(json.dumps(run(ap.parse_args()), indent=1))


if __name__ == "__main__":
    main()
