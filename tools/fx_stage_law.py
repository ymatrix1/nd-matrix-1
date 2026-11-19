#!/usr/bin/env python3
"""FX dispatcher stage law: what one stage record does to the bus (oracle probe + formula).

The pass routine of the FX cores runs 35 stage records. For one record (`S` = its state pointer)
the dispatcher reads the level word `L = DM(S-0x1B)`, the step word `s = DM(S-0x1C)` and the limit
`lim = DM(S-0x19)`, then (listing `0x1217CB..0x12183B`, core 2 pcs; core 1 is `+0x974` later):

    c = clip(L + s, -lim, +lim)     DM(S-0x1B) = c        (the level ramps by `s` per pass)
    g = c + lim                     0 .. 2 lim
    stage input  = g * bus          (written to the scratch block `0xB62BE`)
    bus <- g * y + (lim - c) * bus  (`y` = the stage routine output; `lim - c` = `2 lim - g`)

so a stage is a cross-fade: `g = 2 lim` replaces the bus by the stage output, `g = 0` leaves the
bus alone. For a stage that returns its input (the pass-through routine, `y = g * bus`) the bus
gain is `g^2 + lim - c`.

    python tools/fx_stage_law.py probe --image FX/core1_sram.bin \
        --sharc target/release/sharc-fxlive --out docs/research/data/fx-stage-law.csv

`probe` needs the local FX image and the built oracle; `law` and `bus_gain` are pure functions
(tested by `tools/tests/test_fx_stage_law.py` against the committed CSV). Evidence and limits:
`docs/research/reverb-stage-gain.md`.
"""

from __future__ import annotations

import argparse
import csv
import re
import subprocess
import sys
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent))

import fx_oracle_live as fol

#: VERIFIED (run, `docs/research/fx-oracle-live.md` s3): the Core 1 input stage gain of the bus.
INPUT_STAGE = 0.1866  # unused in the ratio; kept as the documented baseline
#: Record word addresses of stage 0 (`fx_oracle_live.REC`): step, level; limit is `S - 0x19`.
LIMIT_WORD = 0xB62D5
FIELDS = ["level", "step", "limit", "clip", "g", "expected_gain", "measured_gain"]


def clip(x: float, lim: float) -> float:
    """`CLIP x BY lim` of the dispatcher: `|x|` limited to `|lim|`."""
    return max(-abs(lim), min(abs(lim), x))


def law(level: float, step: float, lim: float) -> tuple[float, float, float]:
    """`(c, g, k)` of one pass: new level `c`, stage input gain `g`, bus keep gain `k`."""
    c = clip(level + step, lim)
    return c, c + lim, lim - c


def bus_gain(level: float, step: float, lim: float) -> float:
    """Bus gain of a pass-through stage (`y = g * bus`): `g^2 + k`."""
    _c, g, k = law(level, step, lim)
    return g * g + k


def steady_gain(level: float, step: float, lim: float) -> float:
    """Bus gain after the level has ramped to its fixed point (`step != 0` saturates at `+-lim`)."""
    c = clip(level, lim) if step == 0 else (lim if step > 0 else -lim)
    return (c + lim) ** 2 + lim - c


def probe_once(args: argparse.Namespace, level: float, step: float, lim: float) -> float:
    """Output RMS (left) of the pass-through stage with these record words, 440 Hz at 0.25."""
    words = fol.install("identity", fol.REVERB_I1)
    words = [(a, w) for a, w in words if a not in (fol.REC["level"], fol.REC["step"])]
    words += [
        (fol.REC["level"], fol.f32(level)),
        (fol.REC["step"], fol.f32(step)),
        (LIMIT_WORD, fol.f32(lim)),
    ]
    script = Path(args.tmp) / "stage_law.script"
    script.write_text(fol.script_text(words))
    amp = 0.25
    out = subprocess.run(
        [
            args.sharc, args.image, "--core", "1", "--passes", "3400", "--script-flat", str(script),
            "--sine", f"440:{amp}", "--lead", "3000",
        ],
        capture_output=True, text=True, check=True,
    ).stdout  # fmt: skip
    m = re.search(r"output L peak [\d.]+ rms ([\d.]+)", out)
    if not m:
        raise SystemExit(f"no output line in: {out!r}")
    return float(m.group(1))


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs firmware image
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("cmd", choices=["probe"])
    ap.add_argument("--image", required=True)
    ap.add_argument("--sharc", default="target/release/sharc-fxlive")
    ap.add_argument("--tmp", default=".")
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args(argv)
    grid = [(lv, 0.0, 0.5) for lv in (-0.5, -0.25, 0.0, 0.25, 0.5)]
    grid += [(lv, 0.0, 0.25) for lv in (-0.25, 0.0, 0.25)]
    grid += [(-0.5, 0.25, 0.5), (0.25, 0.25, 0.5)]
    # the printed RMS covers the silent lead too; the unity record (level = limit = 0.5,
    # g^2 + k = 1) is the reference, so `measured_gain` is the ratio to it (law: gain 1.0)
    ref = probe_once(a, 0.5, 0.0, 0.5)
    rows = []
    for lv, st, lim in grid:
        c, g, _k = law(lv, st, lim)
        rows.append(
            {
                "level": lv, "step": st, "limit": lim, "clip": round(c, 6), "g": round(g, 6),
                "expected_gain": round(steady_gain(lv, st, lim), 6),
                "measured_gain": round(probe_once(a, lv, st, lim) / ref, 6),
            }
        )  # fmt: skip
        print(rows[-1], flush=True)
    with a.out.open("w", encoding="utf-8", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=FIELDS)
        w.writeheader()
        w.writerows(rows)
    return 0


if __name__ == "__main__":
    sys.exit(main())
