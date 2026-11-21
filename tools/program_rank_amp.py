#!/usr/bin/env python3
"""Before / after of the LP24 / HP24 flip for the programs that use those amps.

Reads the JSON lines of `program_rank --amp-filters --flip fx_amp_lp24` and prints, per program,
the score of the engine default against the firmware render (before) and of the default with
`FirmwareFx::lp24` on against the firmware render (after), plus a summary
(`docs/research/amp-types-789.md` s7). Score as in `docs/research/program-ranking.md`.

    python tools/program_rank_amp.py rank_amp.jsonl [--csv out.csv]
"""

from __future__ import annotations

import argparse
import csv
import json
import statistics as st
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import program_rank as pr  # noqa: E402

KEY = "fx_amp_lp24"
WORSE_MARGIN = 0.5  # ASSUMED, as in program_rank_combo.py


def rows(path: str) -> list[dict]:
    out = []
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        r = json.loads(line)
        before = pr.distance(r["default"], r["firmware"])["score"]
        after = pr.distance(r["flip"][KEY], r["firmware"])["score"]
        out.append(
            {
                "entry": r["entry"],
                "name": r["name"],
                "lp24": r["amp_filters"]["lp24"],
                "hp24": r["amp_filters"]["hp24"],
                "before": before,
                "after": after,
            }
        )
    return out


def summary(rs: list[dict]) -> dict:
    d = [r["after"] - r["before"] for r in rs]
    b = [r["before"] for r in rs]
    a = [r["after"] for r in rs]
    return {
        "programs": len(rs),
        "mean_before": st.mean(b) if rs else 0.0,
        "mean_after": st.mean(a) if rs else 0.0,
        "median_before": st.median(b) if rs else 0.0,
        "median_after": st.median(a) if rs else 0.0,
        "improved": sum(x < -WORSE_MARGIN for x in d),
        "unchanged": sum(abs(x) <= WORSE_MARGIN for x in d),
        "worse": sum(x > WORSE_MARGIN for x in d),
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("jsonl")
    ap.add_argument("--csv")
    a = ap.parse_args()
    rs = sorted(rows(a.jsonl), key=lambda r: -r["before"])
    for r in rs:
        print(
            f"{r['entry'][:40]:40} lp24 {r['lp24']} hp24 {r['hp24']}  "
            f"{r['before']:6.2f} -> {r['after']:6.2f}"
        )
    print(json.dumps(summary(rs), indent=1))
    if a.csv:
        with open(a.csv, "w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=list(rs[0]))
            w.writeheader()
            w.writerows(
                {k: (round(v, 3) if isinstance(v, float) else v) for k, v in r.items()} for r in rs
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
