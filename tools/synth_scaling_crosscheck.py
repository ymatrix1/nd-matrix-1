#!/usr/bin/env python3
# ruff: noqa: E501
r"""Cross-check of the oracle-settled tables against published closed forms (D212).

The OpenNord project (AGPL-3.0, `src/lib/ns4/synth-scaling.ts`) publishes fitted closed forms of
displayed times and laws "recovered from OS v1.66". Only the numbers are used here, as an
independent cross-check of our measured tables (corroboration, not VERIFIED; no code is copied).
Forms: attack time `0.0005 (1 + v / 40.2)^8` s, decay / release 40 dB fall time
`0.003 (1 + v / 54.1)^7.958` s, cutoff index `v - 52` up to 117, `65 + 1.7 (v - 117)` above and 106
at 127, resonance damping `k = 2 (1 - v / 128)^2`.

    uv run python tools/synth_scaling_crosscheck.py
"""

from __future__ import annotations

import csv
import math
import statistics
import sys
from pathlib import Path

import synth_env_trace as t

DATA = Path(__file__).resolve().parent.parent / "docs/research/data"


def attack_seconds(v: float) -> float:
    return 0.0005 * (1 + v / 40.2) ** 8


def decay_seconds(v: float) -> float:
    return 0.003 * (1 + v / 54.1) ** 7.958


def cutoff_index(v: int) -> float:
    if v >= 127:
        return 106.0
    return 65 + 1.7 * (v - 117) if v > 117 else float(v - 52)


def resonance_k(v: int) -> float:
    return 2 * (1 - v / 128) ** 2


def fall40_seconds(a: float) -> float:
    """Seconds a per-pass decay coefficient `a` needs to fall 40 dB (`a^n = 0.01`)."""
    return math.log(0.01) / math.log(a) * t.PASS_S


def log_ratio_stats(ours: list[float], theirs: list[float]) -> tuple[float, int, float]:
    """`(max relative error, knob of the max, median relative error)` of two series."""
    errs = [abs(math.exp(abs(math.log(o / h))) - 1) for o, h in zip(ours, theirs, strict=True)]
    m = max(errs)
    return m, errs.index(m), statistics.median(errs)


def load(name: str) -> list[dict]:
    with (DATA / name).open(encoding="utf-8") as f:
        return list(csv.DictReader(f))


def attack_check(table: list[dict], col: str = "attack_coeff") -> tuple[float, int, float]:
    ours = [t.PASS_S / float(table[v][col]) for v in range(128)]
    return log_ratio_stats(ours, [attack_seconds(v) for v in range(128)])


def decay_check(table: list[dict], col: str) -> tuple[float, int, float]:
    ours = [fall40_seconds(float(table[v][col])) for v in range(127)]
    return log_ratio_stats(ours, [decay_seconds(v) for v in range(127)])


def cutoff_max_error() -> float:
    ours = [k - 52 if k <= 117 else t.CUTOFF_TOP[k - 118] / 2**24 for k in range(128)]
    return max(abs(o - cutoff_index(k)) for k, o in enumerate(ours))


def resonance_agreement(b: list[float], tol: float = 1e-4) -> tuple[int, float]:
    """Number of leading indexes where our `B` table equals the quadratic within `tol` (relative)
    and the maximum relative error over those indexes."""
    n = 0
    while n < 128 and abs(b[n] / resonance_k(n) - 1) <= tol:
        n += 1
    return n, max(abs(b[v] / resonance_k(v) - 1) for v in range(n))


def main() -> int:  # pragma: no cover - CLI wiring
    s, o = load("synth-env-rates-settled.csv"), load("synth-env-rate-table.csv")
    print("attack settled", attack_check(s), "stale", attack_check(o))
    print(
        "decay settled",
        decay_check(s, "decay_coeff"),
        "stale",
        decay_check(o, "decay_release_coeff"),
    )
    print("cutoff index max abs error", cutoff_max_error())
    b = [float(r["B_0xB1015"]) for r in load("synth-filter-tables.csv")]
    print("resonance B leading agreement", resonance_agreement(b))
    return 0


if __name__ == "__main__":
    sys.exit(main())
