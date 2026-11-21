#!/usr/bin/env python3
r"""Amp attack velocity scaling and decay sustain level of the ORG core 2 oracle (D207).

Extends `synth_env_trace.py` (same capture, same `sharc-sys` taps of the three envelope chains of
slot 0). A `grid` run injects one ADR word triple `attack,decay,release` (the same index in all
three chains), keys one of the captured keys (36 / 60 / 84) with key velocity byte `0x80 + g` and
records, per chain, the attack slope, the peak, the level at key off and the release ratio.

The filter chain does not see `g`, so every run also measures the unscaled chain at the same word;
that gives the reference the amp chain is compared with, with the same slope method.

    uv run python tools/synth_env_law.py grid CAPTURE.txt.gz --sharc-sys target/release/sharc-sys \
        --image DIR/core2_sram.bin --workdir target/w --out grid.csv \
        --keys 60 --gs 10,64,127 --cases=-40:-40:-40 --jobs 4
    uv run python tools/synth_env_law.py fit grid_even.csv grid_odd.csv
"""

from __future__ import annotations

import argparse
import csv
import math
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import synth_env_trace as t

FIELDS = (
    "key", "g", "attack", "decay", "release", "chain",
    "attack_slope", "peak", "level_at_off", "release_ratio",
)  # fmt: skip
LAWS = ("g127-trunc", "g127-round", "g127-floor", "g127-ceil", "g128-floor")


def parse_cases(text: str) -> list[tuple[int, int, int]]:
    """`a:d:r` triples separated by spaces or `;` (word indexes, each -64..63)."""
    out = []
    for tok in text.replace(";", " ").split():
        a, d, r = (int(x) for x in tok.split(":"))
        out.append((a, d, r))
    return out


def case_words(a: int, d: int, r: int, cutoff_knob: int = 100) -> dict[int, int]:
    """ADR words (amp, filter, osc) with the triple repeated, plus a fixed cutoff word."""
    words = {}
    for chain in range(3):
        for i, v in enumerate((a, d, r)):
            words[t.ADR_BASE + 3 * chain + i] = (v << 24) & 0xFFFFFFFF
    words[t.COMMON_CUTOFF] = t.cutoff_word(cutoff_knob)
    return words


def scaled_index(word: int, g: int, law: str) -> int:
    """Candidate effective index of the amp attack for a negative word (law name `g127-<rounding>`
    scales by `g / 127`, `g128-floor` is `(word * G) >> 7` with `G = 128` for `g = 127`).

    Positive words are not scaled (oracle: identical slope at every `g`). The winning law is
    `g128-floor` (D207).
    """
    if word >= 0:
        return word
    if law == "g128-floor":
        return (word * (128 if g >= 127 else g)) >> 7
    x = word * g / 127.0
    return {
        "g127-trunc": math.trunc, "g127-round": round,
        "g127-floor": math.floor, "g127-ceil": math.ceil,
    }[law](x)  # fmt: skip


def fit_laws(rows: list[dict]) -> dict:
    """Score each law: rows (negative word, `g < 127`) whose amp slope equals the slope of the
    filter chain at the law's index within 1 %. The filter rows (unscaled) are the reference, so
    pass rows of runs that cover every integer word.

    Returns `{law: (hits, total)}`.
    """
    ref: dict[int, float] = {}
    for r in rows:
        if r["chain"] == "filter" and r["attack_slope"]:
            ref.setdefault(int(r["attack"]), float(r["attack_slope"]))
    out = {}
    for law in LAWS:
        hits = total = 0
        for r in rows:
            if r["chain"] != "amp" or not r["attack_slope"]:
                continue
            if int(r["attack"]) >= 0 or int(r["g"]) >= 127:
                continue
            total += 1
            k = scaled_index(int(r["attack"]), int(r["g"]), law)
            if k in ref and abs(math.log(float(r["attack_slope"]) / ref[k])) < 0.01:
                hits += 1
        out[law] = (hits, total)
    return out


def run_case(ns: argparse.Namespace, rows: list[dict], key: int, g: int, case: tuple, tag: str):
    """One oracle run; returns the CSV rows of the three chains."""
    import synth_e2e_oracle as o

    work = Path(ns.workdir)
    on = 300
    off = on + ns.hold
    passes = off + 400
    (work / f"key_{tag}.script").write_text(t.key_byte_script(o.key_script(rows, on, off, key), g))
    (work / f"adr_{tag}.script").write_text(t.inject_script(case_words(*case)))
    env, _ = t.run_oracle(
        ns.sharc_sys,
        ns.image,
        [work / "load.script", work / f"adr_{tag}.script", work / f"key_{tag}.script"],
        passes,
        work / f"trace_{tag}.bin",
    )
    att, dec = t.load_rate_table()
    out = []
    for k, name in enumerate(t.CHAIN_NAMES):
        f = t.chain_facts(env[:, k], on, off, att, dec)
        out.append(
            {
                "key": key, "g": g, "attack": case[0], "decay": case[1], "release": case[2],
                "chain": name, "attack_slope": f.get("attack_slope", ""),
                "peak": f["peak"], "level_at_off": f["level_at_off"],
                "release_ratio": f.get("release_ratio") or "",
            }
        )  # fmt: skip
    return out


def grid(ns: argparse.Namespace) -> int:  # pragma: no cover - needs local firmware images
    import arm_dsp_capture as cap
    import synth_e2e_oracle as o

    rows = list(cap.read_rows(Path(ns.capture)))
    work = Path(ns.workdir)
    work.mkdir(parents=True, exist_ok=True)
    (work / "load.script").write_text(o.core2_load_script(rows))
    jobs = [
        (int(k), int(g), c)
        for k in ns.keys.split(",")
        for g in ns.gs.split(",")
        for c in parse_cases(ns.cases)
    ]
    with ThreadPoolExecutor(max_workers=ns.jobs) as pool:
        futs = [
            pool.submit(run_case, ns, rows, k, g, c, f"{i}") for i, (k, g, c) in enumerate(jobs)
        ]
        results = [f.result() for f in futs]
    with Path(ns.out).open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=FIELDS, lineterminator="\n")
        w.writeheader()
        for res in results:
            w.writerows(res)
    return 0


def fit(ns: argparse.Namespace) -> int:  # pragma: no cover - CLI wiring
    rows = []
    for path in ns.csv:
        with Path(path).open(encoding="utf-8") as f:
            rows += list(csv.DictReader(f))
    for law, (h, n) in fit_laws(rows).items():
        print(f"{law:11s} hits {h}/{n}")
    return 0


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - CLI wiring
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    g = sub.add_parser("grid")
    g.add_argument("capture")
    g.add_argument("--sharc-sys", required=True)
    g.add_argument("--image", required=True)
    g.add_argument("--workdir", required=True)
    g.add_argument("--out", required=True)
    g.add_argument("--keys", default="60")
    g.add_argument("--gs", default="10,32,64,98,127")
    g.add_argument("--cases", required=True, help="a:d:r triples, space separated")
    g.add_argument("--hold", type=int, default=2600, help="passes between key on and off")
    g.add_argument("--jobs", type=int, default=4)
    f = sub.add_parser("fit")
    f.add_argument("csv", nargs="+", help="grid CSVs; together they must cover every integer word")
    ns = ap.parse_args(argv)
    return grid(ns) if ns.cmd == "grid" else fit(ns)


if __name__ == "__main__":
    sys.exit(main())
