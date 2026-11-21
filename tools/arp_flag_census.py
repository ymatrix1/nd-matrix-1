#!/usr/bin/env python3
"""Fold the `arm_arp_flag_trace.py` run files into one committed census (OQ32, D168).

One run file per program (or per patched variant) holds, for the six pattern objects, the flag
`[obj + 0xC]` after boot, program load, idle and the held arpeggio, every store the write hook
saw on the flag, the hits of the 37 candidate store sites and the gate of each layer. This tool
joins them into `docs/research/data/arp-flag-census.json`: one row per run and layer, plus the
invariants the claim rests on (`summary`):

* `flag_writes`: stores to the flag after boot over all runs (the constructor runs only in boot);
* `flags_not_one`: runs where a flag differs from 1 at any phase (not counting `force_flag` runs);
* `site_hits_on_pattern_object`: candidate-site executions whose base register is a pattern object;
* `gate_fractions`: distinct gate fractions of all layers that played, with run counts.

    python tools/arp_flag_census.py target/arpflag --out docs/research/data/arp-flag-census.json

Only numbers are written. The run files come from the local firmware and are not committed.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path

import arp_fw_compare as cmp


def run_rows(run: dict) -> list[dict]:
    """One row per layer of a run file."""
    base = {
        "program": run["program"],
        "patches": run.get("patches", []),
        "force_flag": run.get("force_flag"),
    }
    return [
        {**base, "layer": layer, **{k: v for k, v in stats.items() if k != "step_spread_ms"}}
        for layer, stats in sorted(run["layers"].items())
    ]


def native_stats(doc: dict, layer: str) -> dict:
    """`arp_fw_compare.summarize` of one layer of an `arp_trace` document."""
    return cmp.summarize(*cmp.native_edges(doc, layer))


def with_native(rows: list[dict], native: dict[str, dict]) -> list[dict]:
    """Adds `native_<tag>` = `{notes_on, step_ms, gate_ms, gate_fraction}` per layer to `rows`."""
    out = []
    for row in rows:
        extra = {}
        for tag, doc in native.items():
            stats = native_stats(doc, row["layer"])
            extra[f"native_{tag}"] = {
                k: stats[k] for k in ("notes_on", "step_ms", "gate_ms", "gate_fraction")
            }
        out.append({**row, **extra})
    return out


def run_flags_all_one(run: dict) -> bool:
    return all(v == 1 for phase in run["flags"].values() for v in phase.values())


def summarize(runs: list[dict]) -> dict:
    """The invariants over all runs (see the module doc)."""
    natural = [r for r in runs if r.get("force_flag") is None]
    gates: Counter[float] = Counter()
    for run in natural:
        for stats in run["layers"].values():
            if stats["gate_fraction"] is not None:
                gates[stats["gate_fraction"]] += 1
    return {
        "runs": len(runs),
        "natural_runs": len(natural),
        "flag_writes": sum(len(r["flag_writes"]) for r in natural),
        "flags_not_one": sum(not run_flags_all_one(r) for r in natural),
        "constructor_calls_after_boot": sum(r["constructor_calls"]["after_boot"] for r in runs),
        "site_hits": sum(s["hits"] for r in natural for s in r["store_sites"].values()),
        "site_hits_on_pattern_object": sum(
            s["on_pattern_object"] for r in natural for s in r["store_sites"].values()
        ),
        "sites_reached": sorted(
            {a for r in natural for a, s in r["store_sites"].items() if s["hits"]}
        ),
        "gate_fractions": {str(k): v for k, v in sorted(gates.items())},
    }


def census(paths: list[Path], native_dirs: dict[str, Path] | None = None) -> dict:
    """Run files joined with the `arp_trace` file of the same name in each `native_dirs` entry."""
    rows = []
    runs = []
    for path in sorted(paths):
        run = json.loads(path.read_text(encoding="utf-8"))
        runs.append(run)
        native = {}
        for tag, folder in (native_dirs or {}).items():
            file = folder / path.name
            if file.exists():
                native[tag] = json.loads(file.read_text(encoding="utf-8"))
        rows.extend(with_native(run_rows(run), native))
    return {"summary": summarize(runs), "rows": rows}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("directory", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--native-off", type=Path, help="folder of arp_trace files, engine gate")
    ap.add_argument("--native-on", type=Path, help="folder of arp_trace files, --arp-fw-gate")
    ns = ap.parse_args(argv)
    native = {t: d for t, d in (("off", ns.native_off), ("on", ns.native_on)) if d}
    paths = list(ns.directory.glob("*.json"))
    if not paths:
        print("no run files", file=sys.stderr)
        return 1
    ns.out.write_text(json.dumps(census(paths, native), indent=1) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
