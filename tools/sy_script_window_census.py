#!/usr/bin/env python3
"""Census of the SY message scripts and live watch results of a bias on / off run (D224).

D211 added the opt-in FIFO index bias for the SY1SY2 chip (`SY:0=sy=IMAGE=epchain+paced`) and left
the SY census unrepeated: the default model replays stale FIFO pairs, so a message the ARM does
send could have been lost or applied twice. This tool compares the `SY0.script` files that
`tools/arm_dsp_live.py --log DIR` writes for the two models and reads the `watch` block of their
`--out` JSON:

    python tools/sy_script_window_census.py RUN_A/SY0.script RUN_B/SY0.script \
        [--json-a RUN_A.json --json-b RUN_B.json]

A script line is `core address value` (hex), one ARM to SY path 1 message. Only counts, address
ranges and equality are printed; no firmware word is quoted. Provenance tags follow
`.agents/rules/provenance-in-code.md`.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

# VERIFIED: pitch window of the SY audio ISR step 3 (`i5 = 0x9354A` at 0x1C0679), D141 / D176.
PITCH_WINDOW = (0x93500, 0x935A0)


def parse_script(text: str) -> list[tuple[int, int, int]]:
    """`core address value` lines as integer triples; blank lines and `#` comments are skipped."""
    rows = []
    for line in text.splitlines():
        parts = line.split("#", 1)[0].split()
        if not parts:
            continue
        if len(parts) != 3:
            raise ValueError(f"bad script line {line!r}")
        rows.append((int(parts[0], 0), int(parts[1], 0), int(parts[2], 0)))
    return rows


def window_hits(rows: list[tuple[int, int, int]]) -> list[int]:
    """Indexes of messages whose address (opcode bits ignored) lies in the pitch window."""
    lo, hi = PITCH_WINDOW
    return [i for i, (_c, a, _v) in enumerate(rows) if lo <= (a & 0x0FFF_FFFF) < hi]


def summarise(rows: list[tuple[int, int, int]]) -> dict[str, object]:
    """Message count, distinct addresses, address range and the window hit indexes."""
    addrs = [a for _c, a, _v in rows]
    return {
        "messages": len(rows),
        "distinct_addresses": len(set(addrs)),
        "min_address": min(addrs, default=None),
        "max_address": max(addrs, default=None),
        "window_hits": window_hits(rows),
    }


def watch_changes(result: dict, chip: str = "SY0") -> dict[str, int]:
    """Watched words of `chip` in an `arm_dsp_live` JSON: how many, how many ever changed."""
    watch = result.get("watch", {}).get(chip, {})
    changed = [a for a, v in watch.items() if v.get("first_change_pass") is not None]
    return {"watched": len(watch), "changed": len(changed)}


def compare(a: list[tuple[int, int, int]], b: list[tuple[int, int, int]]) -> dict[str, object]:
    """Equality of two message streams and the index of the first difference (or `None`)."""
    first = next((i for i, (x, y) in enumerate(zip(a, b, strict=False)) if x != y), None)
    if first is None and len(a) != len(b):
        first = min(len(a), len(b))
    return {"equal": first is None, "first_difference": first, "len_a": len(a), "len_b": len(b)}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("script_a", type=Path)
    ap.add_argument("script_b", type=Path)
    ap.add_argument("--json-a", type=Path)
    ap.add_argument("--json-b", type=Path)
    ns = ap.parse_args(argv)
    a = parse_script(ns.script_a.read_text())
    b = parse_script(ns.script_b.read_text())
    out: dict[str, object] = {"a": summarise(a), "b": summarise(b), "compare": compare(a, b)}
    for key, path in (("watch_a", ns.json_a), ("watch_b", ns.json_b)):
        if path is not None:
            out[key] = watch_changes(json.loads(path.read_text()))
    json.dump(out, sys.stdout, indent=2)
    print()
    return 0


if __name__ == "__main__":
    sys.exit(main())
