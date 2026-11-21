#!/usr/bin/env python3
r"""Combined FX ablation of the program ranking: which FX group combination explains each gap.

Input: `program_rank --ablate-fx` lines (numbers only), one per program. Each line holds the
metrics of the default render, of the firmware-derived render and of the firmware render with every
non-empty combination of the six FX groups put back to the default (63 renders, keys such as
`fx_mod1+fx_mod2`). Score = `program_rank.distance(default, render)`, so a reverted set explains
the gap when it leaves a small score.

    python tools/program_rank_combo.py --ablate-fx fx.jsonl --md \
        --csv docs/research/data/program-ranking-fx-combos.csv

`--flip` reads `program_rank --flip` lines instead: the engine default with a set of FX groups
switched to the firmware value (what a default flip would sound like), scored against the
firmware render. It gives the before / after table of a candidate flip.

Per program: the full score, the best single group, the best pair, the greedy chain (add the group
that lowers the score most, until all six), the exact best set (all 63 are known, so the greedy
chain is checked against it), the floor (all six FX groups reverted: what the organ tone wheels and
the synth filter engine leave), and the smallest set whose score is at most `EXPLAINED_SCORE`.
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import statistics as st
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import program_rank as pr  # noqa: E402

FX_GROUPS = ("fx_delay", "fx_compressor", "fx_reverb", "fx_eq", "fx_mod1", "fx_mod2")
# ASSUMED: a set explains the gap when it leaves at most this score (ranking median 3.9, 60 of
# the 384 programs are below 1)
EXPLAINED_SCORE = 2.0
WORSE_MARGIN = 0.5  # ASSUMED: a program counts as worse when its score rises by more than this
NL = chr(10)


def key_of(groups) -> str:
    """Key of a combination as `program_rank` writes it: names in group order joined by `+`."""
    return "+".join(g for g in FX_GROUPS if g in set(groups))


def scores(row: dict) -> dict[str, float | None]:
    """Score left per combination key; the empty set is the full firmware score."""
    out = {"": pr.distance(row["default"], row["firmware"])["score"]}
    for k, m in row["ablate_fx"].items():
        out[k] = pr.distance(row["default"], m)["score"]
    return out


def best(sc: dict, size: int | None = None) -> tuple[str, float | None]:
    """Combination with the smallest score (of `size` groups when given, never the empty set)."""
    cand = {
        k: v
        for k, v in sc.items()
        if k and v is not None and (size is None or k.count("+") == size - 1)
    }
    if not cand:
        return "", None
    k = min(cand, key=lambda x: (cand[x], x.count("+"), x))
    return k, cand[k]


def greedy(sc: dict) -> list[tuple[str, float | None]]:
    """Add the group that lowers the score most, one at a time: (combination, score left) steps."""
    chosen: list[str] = []
    steps = []
    while len(chosen) < len(FX_GROUPS):
        rest = [g for g in FX_GROUPS if g not in chosen]
        opts = {g: sc.get(key_of(chosen + [g])) for g in rest}
        opts = {g: v for g, v in opts.items() if v is not None}
        if not opts:
            break
        g = min(opts, key=lambda x: (opts[x], x))
        chosen.append(g)
        steps.append((key_of(chosen), opts[g]))
    return steps


def smallest_explaining(sc: dict, limit: float = EXPLAINED_SCORE) -> tuple[str, float | None]:
    """Smallest combination whose score is at most `limit` (lowest score among equals)."""
    ok = {k: v for k, v in sc.items() if k and v is not None and v <= limit}
    if not ok:
        return "none", None
    k = min(ok, key=lambda x: (x.count("+"), ok[x], x))
    return k, ok[k]


def summarize(row: dict) -> dict:
    """One program: full score, single / pair / best / greedy / floor / smallest explaining set."""
    sc = scores(row)
    full = sc[""]
    s1, s2, sb = best(sc, 1), best(sc, 2), best(sc)
    steps = greedy(sc)
    floor = sc.get(key_of(FX_GROUPS))
    expl = smallest_explaining(sc)
    return {
        "entry": row["entry"], "score": full, "single": s1, "pair": s2, "best": sb,
        "greedy": steps, "floor": floor, "explain": expl,
        "pairs": {k: v for k, v in sc.items() if k.count("+") == 1},
    }  # fmt: skip


def f(x, nd=2):
    return "" if x is None else round(x, nd)


def write_csv(rows: list[dict], path: Path, prefix: str = "left_without_") -> None:
    """Committed table: score per FX combination of every program (numbers only)."""
    keys = sorted({k for r in rows for k in r["all"] if k}, key=lambda k: (k.count("+"), k))
    with path.open("w", newline="", encoding="utf-8") as fh:
        w = csv.writer(fh, lineterminator=NL)
        w.writerow(["entry", "score", *[f"{prefix}{k}" for k in keys]])
        for r in rows:
            w.writerow([r["entry"], f(r["score"], 3), *[f(r["all"].get(k), 3) for k in keys]])


def md(rows: list[dict]) -> str:
    """Markdown table for the research note."""
    head = [
        "#",
        "Program",
        "Score",
        "Best single",
        "Best pair",
        "Best set",
        "Floor (all FX)",
        "Smallest set <= 2",
    ]
    lines = ["| " + " | ".join(head) + " |", "|" + "---|" * len(head)]
    for i, r in enumerate(rows):
        name = r["entry"].split("/", 1)[1][:-5]

        def cell(t):
            return "-" if t[1] is None else f"{t[0]} ({t[1]:.2f})"

        lines.append(
            f"| {i + 1} | {name} | {f(r['score'])} | {cell(r['single'])} | {cell(r['pair'])} | "
            f"{cell(r['best'])} | {f(r['floor'])} | {cell(r['explain'])} |"
        )
    return NL.join(lines)


def flip_scores(row: dict) -> dict[str, float | None]:
    """Score of the default (`before`) and of each flipped set against the firmware render."""
    out = {"": pr.distance(row["default"], row["firmware"])["score"]}
    for k, m in row["flip"].items():
        out[k] = pr.distance(m, row["firmware"])["score"]
    return out


def flip_moves(rows: list[dict], key: str) -> dict:
    """How far flipping `key` moves each program from the default render (not from the firmware
    render): the distance `pr.distance(default, flipped)`; used for the rate match, which no
    firmware render contains."""
    moves = [
        (r["entry"], pr.distance(r["default"], r["flip"][key])["score"])
        for r in rows
        if key in r["flip"]
    ]
    moves = [(e, m) for e, m in moves if m is not None]
    vals = [m for _, m in moves]
    return {
        "n": len(vals),
        "mean": st.mean(vals) if vals else 0.0,
        "median": st.median(vals) if vals else 0.0,
        "over1": sum(v > 1.0 for v in vals),
        "over3": sum(v > 3.0 for v in vals),
        "largest": sorted(moves, key=lambda x: -x[1])[:10],
    }


def quantile(xs: list[float], q: float) -> float:
    """Nearest-rank quantile (deterministic, no interpolation)."""
    s = sorted(xs)
    return s[min(len(s) - 1, max(0, math.ceil(q * len(s)) - 1))]


def flip_stats(rows: list[dict], key: str) -> dict:
    """Distribution of the score before and after flipping the set `key` over `rows`."""
    pairs = [
        (r["sc"][""], r["sc"][key])
        for r in rows
        if r["sc"][""] is not None and r["sc"].get(key) is not None
    ]
    before, after = [p[0] for p in pairs], [p[1] for p in pairs]
    return {
        "n": len(pairs), "mean_before": st.mean(before), "mean_after": st.mean(after),
        "median_before": st.median(before), "median_after": st.median(after),
        "p90_before": quantile(before, 0.9), "p90_after": quantile(after, 0.9),
        "max_before": max(before), "max_after": max(after),
        "over5_before": sum(x > 5 for x in before), "over5_after": sum(x > 5 for x in after),
        "over10_before": sum(x > 10 for x in before), "over10_after": sum(x > 10 for x in after),
        "worse": sum(a > b + WORSE_MARGIN for b, a in pairs),
    }  # fmt: skip


def flip_md(rows: list[dict], keys: list[str], worst_k: int = 20) -> str:
    """Markdown: distribution table over all rows and a per-program table of the worst `worst_k`."""
    head = ["Flipped set", "n", "Mean", "Median", "p90", "Max", "> 5", "> 10", "Worse by > 0.5"]
    lines = ["| " + " | ".join(head) + " |", "|" + "---|" * len(head)]
    b = flip_stats(rows, keys[0])
    lines.append(
        f"| none (default) | {b['n']} | {b['mean_before']:.2f} | {b['median_before']:.2f} | "
        f"{b['p90_before']:.2f} | {b['max_before']:.2f} | {b['over5_before']} | "
        f"{b['over10_before']} | - |"
    )
    for k in keys:
        a = flip_stats(rows, k)
        lines.append(
            f"| {k} | {a['n']} | {a['mean_after']:.2f} | {a['median_after']:.2f} | "
            f"{a['p90_after']:.2f} | "
            f"{a['max_after']:.2f} | {a['over5_after']} | {a['over10_after']} | {a['worse']} |"
        )
    top = sorted(
        (r for r in rows if r["sc"][""] is not None), key=lambda r: (-r["sc"][""], r["entry"])
    )[:worst_k]
    lines += [
        "",
        "| # | Program | Before | " + " | ".join(keys) + " |",
        "|" + "---|" * (3 + len(keys)),
    ]
    for i, r in enumerate(top):
        name = r["entry"].split("/", 1)[1][:-5]
        cells = [f(r["sc"].get(k)) for k in keys]
        lines.append(
            f"| {i + 1} | {name} | {f(r['sc'][''])} | " + " | ".join(str(c) for c in cells) + " |"
        )
    return NL.join(lines)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--ablate-fx", type=Path)
    ap.add_argument("--flip", type=Path, help="program_rank --flip lines")
    ap.add_argument("--flip-sets", help="comma list of flipped sets to tabulate (default: all)")
    ap.add_argument("--csv", type=Path)
    ap.add_argument("--md", action="store_true")
    ap.add_argument(
        "--moves", help="with --flip: print how far this flipped set moves each program"
    )
    ns = ap.parse_args(argv)
    if bool(ns.ablate_fx) == bool(ns.flip):
        ap.error("give exactly one of --ablate-fx and --flip")
    if ns.flip and ns.moves:
        print(json.dumps(flip_moves(list(pr.load_jsonl(ns.flip)), ns.moves), indent=1))
        return 0
    if ns.flip:
        rows = [{"entry": r["entry"], "sc": flip_scores(r)} for r in pr.load_jsonl(ns.flip)]
        rows.sort(key=lambda r: r["entry"])
        keys = (
            ns.flip_sets.split(",")
            if ns.flip_sets
            else sorted({k for r in rows for k in r["sc"] if k})
        )
        if ns.csv:
            write_csv(
                [{"entry": r["entry"], "score": r["sc"][""], "all": r["sc"]} for r in rows],
                ns.csv,
                "after_flip_",
            )
        if ns.md:
            print(flip_md(rows, keys))
        print(f"{len(rows)} programs", file=sys.stderr)
        return 0
    rows = []
    for line in pr.load_jsonl(ns.ablate_fx):
        s = summarize(line)
        s["all"] = scores(line)
        rows.append(s)
    rows.sort(key=lambda r: (-(r["score"] or 0), r["entry"]))
    if ns.csv:
        write_csv(rows, ns.csv)
    if ns.md:
        print(md(rows))
    print(f"{len(rows)} programs", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
