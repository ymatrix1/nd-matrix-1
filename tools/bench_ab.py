#!/usr/bin/env python3
"""Interleaved A/B benchmark driver: ABAB rounds, median with min..max, band verdict.

Runs two commands alternately (A, B, A, B, ...) after a warm-up, extracts one number per run from
the output with a regular expression, and prints the median [min..max] of each side, the B/A ratio
and a verdict. A gain is "real" only when the min..max bands of A and B are disjoint; otherwise it
is "within noise" (the rule of docs/research/sharc-lowering-v2.md, section 8).

With a thread-CPU-time benchmark (`nord_sharc::cpu_clock`; `lle_sharc_bench`, `arm-lle-bench`,
`kernel-bench`) 3 to 5 rounds are enough on a loaded host; with wall-clock numbers use 7 to 9 and a
quiet host. The number of running `cargo`/`rustc` processes is recorded before every round; a round
that overlapped a compiler is reported, not hidden.

Example:
    python tools/bench_ab.py --rounds 5 \\
        --a "target/release/lle_sharc_bench --core ORG2=img,org --engine interp" \\
        --b "target/release/lle_sharc_bench --core ORG2=img,org --engine aot" \\
        --metric "mean ([0-9.]+) us"
"""

from __future__ import annotations

import argparse
import re
import shlex
import statistics
import subprocess
import sys
from collections.abc import Callable
from dataclasses import dataclass, field


@dataclass
class Side:
    """Values of one command across the rounds."""

    name: str
    values: list[float] = field(default_factory=list)

    @property
    def median(self) -> float:
        return statistics.median(self.values)

    @property
    def lo(self) -> float:
        return min(self.values)

    @property
    def hi(self) -> float:
        return max(self.values)


def parse_metric(text: str, pattern: str, which: str = "last") -> float:
    """First capture group of the `which` ("first" or "last") match of `pattern` in `text`."""
    found = re.findall(pattern, text)
    if not found:
        raise ValueError(f"metric pattern {pattern!r} did not match the output")
    pick = found[0] if which == "first" else found[-1]
    if isinstance(pick, tuple):
        pick = pick[0]
    return float(pick)


def verdict(a: Side, b: Side) -> str:
    """ "B faster", "B slower" when the bands are disjoint, else "within noise"."""
    if b.hi < a.lo:
        return "B lower (disjoint bands)"
    if b.lo > a.hi:
        return "B higher (disjoint bands)"
    return "within noise (bands overlap)"


def compiler_count(lister: Callable[[], str] | None = None) -> int:
    """Number of running cargo/rustc/link processes (0 when it cannot be told)."""
    try:
        if lister is None:
            if sys.platform == "win32":
                out = subprocess.run(["tasklist"], capture_output=True, text=True).stdout
            else:
                out = subprocess.run(
                    ["ps", "-A", "-o", "comm="], capture_output=True, text=True
                ).stdout
        else:
            out = lister()
    except OSError:
        return 0
    return len(re.findall(r"(?im)^\s*(cargo|rustc|link|lld-link)(\.exe)?\b", out))


def run_ab(
    run: Callable[[str], str],
    a_cmd: str,
    b_cmd: str,
    pattern: str,
    rounds: int,
    warmup: int,
    which: str = "last",
    load: Callable[[], int] = compiler_count,
) -> tuple[Side, Side, list[int]]:
    """Warm up both, then alternate A and B for `rounds` rounds. Returns both sides and the
    compiler count seen before each round."""
    a, b = Side("A"), Side("B")
    for _ in range(warmup):
        run(a_cmd)
        run(b_cmd)
    loads = []
    for r in range(rounds):
        loads.append(load())
        # swap the order every round so a slow first slot does not always land on the same side
        first, second = ((a, a_cmd), (b, b_cmd)) if r % 2 == 0 else ((b, b_cmd), (a, a_cmd))
        for side, cmd in (first, second):
            side.values.append(parse_metric(run(cmd), pattern, which))
    return a, b, loads


def report(a: Side, b: Side, loads: list[int], unit: str) -> str:
    lines = []
    for s in (a, b):
        lines.append(
            f"{s.name}: median {s.median:.2f} {unit} [{s.lo:.2f}..{s.hi:.2f}] n={len(s.values)}"
        )
    ratio = b.median / a.median if a.median else float("nan")
    lines.append(f"B/A median ratio {ratio:.3f}: {verdict(a, b)}")
    busy = [i + 1 for i, n in enumerate(loads) if n]
    lines.append(
        "compilers running before rounds: "
        + (f"yes in rounds {busy} (max {max(loads)})" if busy else "none")
    )
    return "\n".join(lines)


def shell_run(cmd: str) -> str:
    done = subprocess.run(shlex.split(cmd), capture_output=True, text=True, check=False)
    if done.returncode != 0:
        raise RuntimeError(f"{cmd!r} exited {done.returncode}: {done.stderr[-400:]}")
    return done.stdout + done.stderr


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    p.add_argument("--a", required=True, help="command A (baseline)")
    p.add_argument("--b", required=True, help="command B (candidate)")
    p.add_argument("--metric", required=True, help="regex; first capture group is the number")
    p.add_argument("--which", choices=["first", "last"], default="last", help="which match")
    p.add_argument("--rounds", type=int, default=5)
    p.add_argument("--warmup", type=int, default=1)
    p.add_argument("--unit", default="")
    args = p.parse_args(argv)
    a, b, loads = run_ab(
        shell_run, args.a, args.b, args.metric, args.rounds, args.warmup, args.which
    )
    print(report(a, b, loads, args.unit))
    return 0


if __name__ == "__main__":
    sys.exit(main())
