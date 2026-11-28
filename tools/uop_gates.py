#!/usr/bin/env python3
"""Evaluate the structural and model gates of the SHARC uop engine (D260).

Usage: python tools/uop_gates.py [--gate G2 --gate K3 ...] REPORT.json [REPORT.json ...]
       cargo run -q -p nord-tools --bin uop-report | python tools/uop_gates.py -

Reads only the JSON that `uop-report` and `lle_sharc_bench --uop-report` print (schema 1,
`runner/uop_report/json.rs`): counts, never times, so the verdict is the same on every machine. A
report file holds one object or a list of objects. Gates (docs/research/sharc-compiler-design.md
section 12):

  G2  uops per step <= 1.35; `Interp` steps <= 6% of steps (so >= 90% of steps run as real uops
      or native code); `Slow` exits <= 20 per pass; `maintain_calls == 0`; `smc == 0` (no store
      hit a translated code page in the measured passes: a hit means data shares a 256-word page
      with code, costs a revalidation per store and is answered by a smaller page, not ignored).
      Both are STEADY-STATE counters: they are evaluated only on a report whose `window` is
      `measured` (the warm-up passes are not in the counters) or absent (a report written before
      the member existed). A report with `window: "all"` includes the warm-up, where the first
      translations and revalidations are expected, so the two checks fail on it, whatever their
      value, and say why; the other checks read the same counters whichever the window.
  K3  modeled Tier 2 cycles per pass <= 290,000, the design's "1.4 x budget = 290k" (1.4 x 207,360
      = 290,304, the 46.08 us budget at 4.5 GHz, rounded down). A Tier 1 report has no native
      steps, so its Tier 2 model equals its Tier 1 model and K3 reads it as such.

Counter contract: `steps_native` is a SUBSET of `steps_uop` (native blocks are one kind of uop
step; `UopStats` in `core/uop/config.rs` is frozen, so the contract is documented here and in
`runner/uop_report/model.rs`). The step total of a run is `steps_uop + steps_core_step`, never
plus `steps_native`, and `validate` rejects a report with `steps_native > steps_uop`. Optional
members `replay` (`checked`, `skipped` passes of the `uop-checked` engine) and `block_steps` (the
block-length histogram of design 11.3) are shape-checked and gate nothing: a check is added only
with its design row.

All thresholds are ASSUMED decision thresholds, not measurements, and each is a row of design
section 12: a check is added here only together with its design row (the test
`test_the_checks_are_exactly_the_design_rows` pins the list). Exit code: 0 every selected gate
passed on every report, 1 a gate failed, 2 unreadable or unsupported input.
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass

SCHEMA = 1
#: What the counters cover: the measured passes only, or every pass the run made (warm-up too).
WINDOWS = ("measured", "all")
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


#: Members of the `replay` and `block_steps` objects.
REPLAY_KEYS = ("checked", "skipped")
BLOCK_STEPS_KEYS = ("entries", "steps", "chained", "empty")


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


def is_count(x: object) -> bool:
    """A non-negative integer that is not a bool (JSON `true` is an int in Python)."""
    return isinstance(x, int) and not isinstance(x, bool) and x >= 0


def validate_block_steps(hist: object, stats: dict) -> None:
    """Check the shape and the arithmetic of a `block_steps` histogram against `stats`."""
    if not isinstance(hist, dict):
        raise ReportError("block_steps is not an object")
    edges, counts = hist.get("edges"), hist.get("counts")
    if not (isinstance(edges, list) and edges and all(is_count(e) and e > 0 for e in edges)):
        raise ReportError("block_steps.edges is not a list of positive counts")
    if any(a >= b for a, b in zip(edges, edges[1:])):
        raise ReportError("block_steps.edges is not strictly increasing")
    if not (
        isinstance(counts, list) and len(counts) == len(edges) + 1 and all(map(is_count, counts))
    ):
        raise ReportError("block_steps.counts is not one count per edge plus an open bucket")
    for key in BLOCK_STEPS_KEYS:
        if not is_count(hist.get(key)):
            raise ReportError(f"block_steps.{key} missing or not a count")
    if sum(counts) != hist["entries"]:
        raise ReportError("block_steps.counts do not add up to block_steps.entries")
    if hist["steps"] < hist["entries"]:
        raise ReportError("block_steps.steps is below block_steps.entries (each ran a step)")
    # a chained entry ran two or more blocks, every other entry exactly one
    blocks = hist["entries"] + hist["empty"] + 2 * hist["chained"]
    if blocks > stats["blocks_run"]:
        raise ReportError(
            f"block_steps covers {blocks} blocks, stats.blocks_run is {stats['blocks_run']}"
        )


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
        if not is_count(stats.get(key)):
            raise ReportError(f"stats.{key} missing or not a count")
    for key, size in (("exits", EXIT_COUNT), ("guard_fail", GUARD_COUNT)):
        arr = stats.get(key)
        if not (isinstance(arr, list) and len(arr) == size and all(is_count(x) for x in arr)):
            raise ReportError(f"stats.{key} is not a list of {size} counts")
    for key in ("tier1_cycles", "tier2_cycles"):
        if not is_count(model.get(key)):
            raise ReportError(f"model.{key} missing or not a count")
    if stats["steps_native"] > stats["steps_uop"]:
        raise ReportError(
            "stats.steps_native exceeds stats.steps_uop: native steps are a subset of the uop steps"
        )
    window = report.get("window")
    if window is not None and window not in WINDOWS:
        raise ReportError(f"window {window!r} is not one of {', '.join(WINDOWS)}")
    replay = report.get("replay")
    if replay is not None and not (
        isinstance(replay, dict) and all(is_count(replay.get(k)) for k in REPLAY_KEYS)
    ):
        raise ReportError("replay is not an object of checked and skipped counts")
    if report.get("block_steps") is not None:
        validate_block_steps(report["block_steps"], stats)
    return report


def steady(report: dict) -> bool:
    """Whether the counters are of the measured passes (or of a report that does not say)."""
    return report.get("window") != "all"


def steps_of(stats: dict) -> int:
    """Interpreter-equivalent steps the run covered: engine steps plus `Core::step` gaps.

    `steps_native` is a subset of `steps_uop` and is not added (see the module docstring)."""
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
    # the steady-state counters mean nothing over a window that includes the warm-up
    ok_steady = steady(report)
    why = "" if ok_steady else " (window all: includes the warm-up, not a steady state)"
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
        Check(
            "G2",
            "maintain_calls",
            s["maintain_calls"],
            "== 0" + why,
            ok_steady and s["maintain_calls"] == 0,
        ),
        Check("G2", "smc", s["smc"], "== 0" + why, ok_steady and s["smc"] == 0),
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
