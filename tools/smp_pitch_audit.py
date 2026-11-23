#!/usr/bin/env python3
"""Audit `smp_cycles` of the piano ranking against the octave of the played piano layer (D230).

The SMP chain tap of key 60 has a fixed pitch in cycles per frame, `BASE_CYCLES` (D205, step law
`0.80618 * 2^(tuning / 1200)`, 0.00601 to 0.00605 over the census); a program whose piano layer
stores octave +1 / -1 must read twice / half of it. `audit` compares the measured ratio with
`2^octave` and returns a verdict per row, so a detector fault (sub-octave, harmonic) or a wrong
layer binding stands out from a real octave shift.

    python tools/smp_pitch_audit.py --csv docs/research/data/program-ranking.csv \
        --ns4 target/release/ns4.exe --programs factory_programs
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import subprocess
from collections.abc import Callable
from pathlib import Path

# EMPIRICAL (D205, `smp-voice-pitch.md` s4): median of the 143 census programs with layer octave 0
# (0.00601 to 0.00605); the step word of key 60 is `0.80618 * 2^(tuning / 1200)`.
BASE_CYCLES = 0.00603
TOLERANCE = 0.1  # octaves: 7 % in pitch; tuning of key 60 spreads the census by 0.6 %


def active_octave(inspected: dict) -> int:
    """Octave of the piano layer the loader plays: the first enabled, else layer A (D216)."""
    piano = inspected["piano"]
    layer = next(
        (piano[k] for k in ("layer_a", "layer_b") if piano[k]["enabled"]), piano["layer_a"]
    )
    return int(layer["octave"])


def audit(rows: list[dict], octave_of: Callable[[str], int | None]) -> list[dict]:
    """`{entry, cycles, octaves, expected, verdict}` per row that has `smp_cycles`.

    `octaves` is `log2(cycles / BASE_CYCLES)`; `verdict` is `ok` within `TOLERANCE` octaves of
    the expected octave, `octave_shift` when it matches a non-zero expected octave, else `mismatch`.
    A row whose octave is unknown reads `unknown`.
    """
    out = []
    for row in rows:
        if not row.get("smp_cycles"):
            continue
        cycles = float(row["smp_cycles"])
        octaves = math.log2(cycles / BASE_CYCLES)
        expected = octave_of(row["entry"])
        if expected is None:
            verdict = "unknown"
        elif abs(octaves - expected) <= TOLERANCE:
            verdict = "octave_shift" if expected else "ok"
        else:
            verdict = "mismatch"
        out.append(
            {
                "entry": row["entry"],
                "cycles": cycles,
                "octaves": round(octaves, 3),
                "expected": expected,
                "verdict": verdict,
            }
        )
    return out


def inspect_octave(ns4: Path, programs: Path) -> Callable[[str], int | None]:  # pragma: no cover
    """Octave lookup through `ns4 inspect --json` of the factory program of an entry."""

    def octave_of(entry: str) -> int | None:
        run = subprocess.run(  # noqa: S603
            [str(ns4), "inspect", str(programs / entry), "--json"],
            capture_output=True,
            text=True,
            check=False,
        )
        return active_octave(json.loads(run.stdout)) if run.returncode == 0 else None

    return octave_of


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local programs
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--csv", type=Path, required=True)
    ap.add_argument("--ns4", type=Path, required=True)
    ap.add_argument("--programs", type=Path, required=True, help="folder holding `Program/...`")
    ns = ap.parse_args(argv)
    with ns.csv.open(newline="", encoding="utf-8") as fh:
        rows = list(csv.DictReader(fh))
    results = audit(rows, inspect_octave(ns.ns4, ns.programs))
    for r in results:
        if r["verdict"] != "ok":
            print(json.dumps(r))
    counts: dict[str, int] = {}
    for r in results:
        counts[r["verdict"]] = counts.get(r["verdict"], 0) + 1
    print(json.dumps(counts))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
