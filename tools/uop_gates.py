#!/usr/bin/env python3
"""Evaluate the structural and model gates of the SHARC uop engine (D260).

Usage: python tools/uop_gates.py [--gate G2 --gate K3 ...] REPORT.json [REPORT.json ...]
       cargo run -q -p nord-tools --bin uop-report | python tools/uop_gates.py -

Reads only the JSON that `uop-report` prints (schema 1, `runner/uop_report.rs`): counts, never
times, so the verdict is the same on every machine. A report file holds one object or a list of
objects. Gates (docs/research/sharc-compiler-design.md section 12):

  G2  uops per step <= 1.35; `Interp` steps <= 6% of steps (so >= 90% of steps run as real uops
      or native code); `Slow` exits <= 20 per pass; `maintain_calls == 0`; `smc == 0` (no store
      hit a translated code page in the measured passes: a hit means data shares a 256-word page
      with code, costs a revalidation per store and is answered by a smaller page, not ignored).
  K3  modeled Tier 2 cycles per pass <= 1.4 x 207,360 = 290,304 (the 46.08 us budget at 4.5 GHz).

All thresholds are ASSUMED decision thresholds, not measurements. Exit code: 0 every selected gate
passed on every report, 1 a gate failed, 2 unreadable or unsupported input.
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass

SCHEMA = 1
#: Index of `Exit::Slow` in the report's `exits` array (Ok, Jump, Fault, Guard, Smc, Slow).
EXIT_SLOW = 5
EXIT_COUNT = 6
GUARD_COUNT = 9

#: G2 thresholds.
MAX_UOPS_PER_STEP = 1.35
MAX_INTERP_SHARE = 0.06
MIN_FAST_SHARE = 0.90
MAX_SLOW_EXITS_PER_PASS = 20
#: K3 threshold: 1.4 x one pass of budget, in host cycles (46.08 us x 4.5 GHz = 207,360).
K3_MAX_CYCLES_PER_PASS = 290_000

GATES = ("G2", "K3")

STATS_KEYS = (
    "passes",
    "blocks_run",
    "uops",
    "steps_uop",
    "steps_interp_uop",
    "steps_core_step",
    "steps_native",
    "smc",
    "translations",
    "poisoned",
    "maintain_calls",
    "shadow_checks",
    "shadow_mismatch",
    "shadow_skipped",
)


class ReportError(ValueError):
    """The input is not a schema 1 `uop-report` object."""


@dataclass(frozen=True)
class Check:
    """One condition of a gate: its name, the measured value, the limit and the verdict."""

    gate: str
    name: str
    value: float
    limit: str
    ok: bool

    def line(self, label: str) -> str:
        mark = "PASS" if self.ok else "FAIL"
        return f"{mark} {self.gate} {self.name} = {self.value:g} (limit {self.limit}) [{label}]"


def validate(report: object) -> dict:
    """Return `report` when it is a well-formed schema 1 object, else raise `ReportError`."""
    if not isinstance(report, dict):
        raise ReportError("report is not a JSON object")
    if report.get("schema") != SCHEMA:
        raise ReportError(f"unsupported schema {report.get('schema')!r}, want {SCHEMA}")
    stats = report.get("stats")
    model = report.get("model")
    if not isinstance(stats, dict) or not isinstance(model, dict):
        raise ReportError("missing stats or model object")
    for key in STATS_KEYS:
        if not isinstance(stats.get(key), int) or stats[key] < 0:
            raise ReportError(f"stats.{key} missing or not a count")
    for key, size in (("exits", EXIT_COUNT), ("guard_fail", GUARD_COUNT)):
        arr = stats.get(key)
        if not (
            isinstance(arr, list) and len(arr) == size and all(isinstance(x, int) for x in arr)
        ):
            raise ReportError(f"stats.{key} is not a list of {size} counts")
    for key in ("tier1_cycles", "tier2_cycles"):
        if not isinstance(model.get(key), int) or model[key] < 0:
            raise ReportError(f"model.{key} missing or not a count")
    return report


def steps_of(stats: dict) -> int:
    """Interpreter-equivalent steps the run covered: engine steps plus `Core::step` gaps."""
    return stats["steps_uop"] + stats["steps_core_step"]


def gate_g2(report: dict) -> list[Check]:
    """Structure gate: how much of the run the fast tiers cover and how often they bail out."""
    s = report["stats"]
    steps = steps_of(s)
    passes = max(s["passes"], 1)
    if steps == 0:
        return [Check("G2", "steps", 0, "> 0", False)]
    interp = s["steps_interp_uop"] + s["steps_core_step"]
    fast = max(s["steps_uop"] - s["steps_interp_uop"], 0)
    uops_per_step = s["uops"] / s["steps_uop"] if s["steps_uop"] else float("inf")
    return [
        Check(
            "G2",
            "uops_per_step",
            uops_per_step,
            f"<= {MAX_UOPS_PER_STEP}",
            uops_per_step <= MAX_UOPS_PER_STEP,
        ),
        Check(
            "G2",
            "fast_step_share",
            fast / steps,
            f">= {MIN_FAST_SHARE}",
            fast / steps >= MIN_FAST_SHARE,
        ),
        Check(
            "G2",
            "interp_step_share",
            interp / steps,
            f"<= {MAX_INTERP_SHARE}",
            interp / steps <= MAX_INTERP_SHARE,
        ),
        Check(
            "G2",
            "slow_exits_per_pass",
            s["exits"][EXIT_SLOW] / passes,
            f"<= {MAX_SLOW_EXITS_PER_PASS}",
            s["exits"][EXIT_SLOW] / passes <= MAX_SLOW_EXITS_PER_PASS,
        ),
        Check("G2", "maintain_calls", s["maintain_calls"], "== 0", s["maintain_calls"] == 0),
        Check("G2", "smc", s["smc"], "== 0", s["smc"] == 0),
    ]


def gate_k3(report: dict) -> list[Check]:
    """Model gate: modeled Tier 2 cycles per pass against 1.4 x the real-time budget."""
    passes = max(report["stats"]["passes"], 1)
    per_pass = report["model"]["tier2_cycles"] / passes
    return [
        Check(
            "K3",
            "modeled_cycles_per_pass",
            per_pass,
            f"<= {K3_MAX_CYCLES_PER_PASS}",
            per_pass <= K3_MAX_CYCLES_PER_PASS,
        ),
    ]


EVALUATORS = {"G2": gate_g2, "K3": gate_k3}


def evaluate(report: dict, gates: tuple[str, ...] = GATES) -> list[Check]:
    """All checks of the selected gates for one validated report."""
    return [c for g in gates for c in EVALUATORS[g](report)]


def load(text: str) -> list[dict]:
    """Parse one report object or a list of them and validate each."""
    try:
        data = json.loads(text)
    except json.JSONDecodeError as e:
        raise ReportError(f"not JSON: {e}") from e
    items = data if isinstance(data, list) else [data]
    if not items:
        raise ReportError("empty report list")
    return [validate(x) for x in items]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("reports", nargs="+", help="uop-report JSON files, or - for stdin")
    ap.add_argument("--gate", action="append", choices=GATES, help="gate to run (default: all)")
    args = ap.parse_args(argv)
    gates = tuple(args.gate) if args.gate else GATES
    failed = False
    for path in args.reports:
        try:
            text = sys.stdin.read() if path == "-" else open(path, encoding="utf-8").read()
            reports = load(text)
        except (OSError, ReportError) as e:
            print(f"uop_gates: {path}: {e}", file=sys.stderr)
            return 2
        for report in reports:
            label = str(report.get("label", path))
            for check in evaluate(report, gates):
                print(check.line(label))
                failed |= not check.ok
    print("verdict: " + ("FAIL" if failed else "PASS"))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
