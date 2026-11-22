#!/usr/bin/env python3
"""Merge the SMP chain-tap census of the piano programs into the committed ranking CSV (D206).

The full chain census of the 384 programs (`tools/program_rank_chain.py`) predates the SMP tap
(D205). Re-running it, and the 70 min native renders, only to fill two columns is waste: this tool
reads a `chain.jsonl` that holds just the piano programs run with `--smp`, and rewrites in the
existing `program-ranking.csv` the columns `smp_peak_dbfs`, `smp_cycles` and the flags of those
rows. Everything else in a row is kept byte for byte. A row without a census record keeps its flags.

    python tools/program_rank_smp.py --chain smp/chain.jsonl \
        --csv docs/research/data/program-ranking.csv
"""

from __future__ import annotations

import argparse
import csv
import json
from pathlib import Path

NO_TAP = "piano_no_chain_tap"
NO_AUDIO = "piano_chain_no_audio"
RUN_FAILED = "smp_run_failed"
OWN_FLAGS = {NO_TAP, NO_AUDIO, RUN_FAILED}


def smp_columns(record: dict) -> tuple[str, str, list[str]]:
    """`(peak, cycles, flags)` strings for one chain record (`census` null: run failed)."""
    census = record.get("census")
    if not census:
        return "", "", [NO_TAP, RUN_FAILED]
    smp = census.get("smp")
    if smp is None:
        return "", "", [NO_TAP]
    peak, cycles = smp.get("peak_dbfs"), smp.get("fundamental_cycles")
    out = [
        "" if peak is None else str(round(peak, 2)),
        "" if cycles is None else str(round(cycles, 5)),
    ]
    return out[0], out[1], [] if smp.get("kind") == "audio" else [NO_AUDIO]


def merge(rows: list[dict], chain: list[dict]) -> list[dict]:
    """Rows with the SMP columns and own flags replaced for every entry in `chain`."""
    by_entry = {r["entry"]: r for r in chain}
    out = []
    for row in rows:
        rec = by_entry.get(row["entry"])
        if rec is None or "piano" not in row["engines"].split("+"):
            out.append(row)
            continue
        peak, cycles, own = smp_columns(rec)
        keep = [f for f in row["flags"].split(";") if f and f not in OWN_FLAGS]
        out.append(
            row | {"smp_peak_dbfs": peak, "smp_cycles": cycles, "flags": ";".join(keep + own)}
        )
    return out


def with_columns(header: list[str]) -> list[str]:
    """Header with `smp_peak_dbfs` and `smp_cycles` before `flags` (the order of `program_rank`)."""
    if "smp_peak_dbfs" in header:
        return header
    at = header.index("flags")
    return [*header[:at], "smp_peak_dbfs", "smp_cycles", *header[at:]]


def summary(rows: list[dict]) -> dict[str, int]:
    """Counts of piano programs by chain-tap outcome."""
    piano = [r for r in rows if "piano" in r["engines"].split("+")]
    flags = [set(r["flags"].split(";")) for r in piano]
    return {
        "piano_programs": len(piano),
        "tap_audio": sum(
            1 for r, f in zip(piano, flags, strict=True) if r["smp_peak_dbfs"] and not f & OWN_FLAGS
        ),
        "no_audio": sum(1 for f in flags if NO_AUDIO in f),
        "no_tap": sum(1 for f in flags if NO_TAP in f),
        "run_failed": sum(1 for f in flags if RUN_FAILED in f),
    }


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--chain", type=Path, required=True)
    ap.add_argument("--csv", type=Path, required=True)
    ns = ap.parse_args(argv)
    chain = [json.loads(x) for x in ns.chain.read_text("utf-8").splitlines() if x.strip()]
    with ns.csv.open(newline="", encoding="utf-8") as fh:
        reader = csv.DictReader(fh)
        header, rows = list(reader.fieldnames or []), list(reader)
    rows = [{"smp_peak_dbfs": "", "smp_cycles": ""} | r for r in merge(rows, chain)]
    header = with_columns(header)
    tmp = ns.csv.with_suffix(".tmp")  # write aside, so a failure never empties the table
    with tmp.open("w", newline="", encoding="utf-8") as fh:
        w = csv.DictWriter(fh, fieldnames=header, lineterminator="\n")
        w.writeheader()
        w.writerows(rows)
    tmp.replace(ns.csv)
    print(json.dumps(summary(rows)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
