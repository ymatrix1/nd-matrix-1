#!/usr/bin/env python3
"""Native minus all-firmware band levels per engine class, from ``program_rank`` JSON lines.

``program_rank <backup.ns4b> --note 60`` renders every factory program with the engine defaults
(``default``) and with the firmware-derived options (``firmware``) and prints one JSON line per
program with eight band shares (dB re total power). This script groups the programs by the engines
enabled in either scene and prints, per group, the mean and the largest absolute difference of the
bands from 1 kHz up and of the combined 2..20 kHz share
(``docs/research/treble-excess-by-engine.md``).
Numbers only; no audio.
"""

from __future__ import annotations

import argparse
import json
import math
import statistics
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any, Iterable

BANDS = ("20-100", "100-250", "250-500", "500-1k", "1-2k", "2-4k", "4-8k", "8-20k")
TREBLE = slice(5, 8)  # 2..4k, 4..8k, 8..20k


def engine_class(row: dict[str, Any]) -> str:
    """Engines enabled in scene I or II, joined with ``+`` (``none`` when no section is on)."""
    on = {k for scene in ("scene_1", "scene_2") for k, v in row.get(scene, {}).items() if v}
    return "+".join(sorted(on)) or "none"


def share_db(bands_db: Iterable[float]) -> float:
    """Combined power share in dB of band shares given in dB."""
    return 10.0 * math.log10(sum(10.0 ** (x / 10.0) for x in bands_db))


def group_rows(rows: Iterable[dict[str, Any]]) -> dict[str, list[dict[str, Any]]]:
    """Rows with both renders measured, by engine class."""
    out: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for r in rows:
        d, f = r.get("default", {}), r.get("firmware", {})
        if "bands_db" in d and "bands_db" in f:
            out[engine_class(r)].append(r)
    return out


def summarize(rows: list[dict[str, Any]]) -> dict[str, Any]:
    """Mean band differences, mean and worst treble difference of a group."""
    diffs = [
        [a - b for a, b in zip(r["default"]["bands_db"], r["firmware"]["bands_db"])] for r in rows
    ]
    treble = [
        share_db(r["default"]["bands_db"][TREBLE]) - share_db(r["firmware"]["bands_db"][TREBLE])
        for r in rows
    ]
    return {
        "n": len(rows),
        "band_mean_db": {
            b: round(statistics.mean(d[i] for d in diffs), 2) for i, b in enumerate(BANDS)
        },
        "treble_mean_db": round(statistics.mean(treble), 2),
        "treble_worst_db": round(max(treble, key=abs), 2),
        "over_1db": sum(abs(t) > 1.0 for t in treble),
    }


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("jsonl", help="program_rank output (one JSON object per line).")
    p.add_argument("--out", help="Write the summary JSON here.")
    args = p.parse_args(argv)
    rows = [
        json.loads(line)
        for line in Path(args.jsonl).read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    result = {
        k: summarize(v) for k, v in sorted(group_rows(rows).items(), key=lambda kv: -len(kv[1]))
    }
    for k, s in result.items():
        line = f"{k:20s} n={s['n']:3d} treble mean {s['treble_mean_db']:+.2f}"
        print(f"{line} worst {s['treble_worst_db']:+.2f} |d|>1: {s['over_1db']}")
    if args.out:
        Path(args.out).write_text(json.dumps(result, indent=1) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
