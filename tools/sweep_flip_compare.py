#!/usr/bin/env python3
"""Compares two `program_sweep` JSONL runs and lists the programs whose render changed (D166).

Usage (the 384-program numbers of the arp / morph default flip):

    cargo build --release -p nord-tools --bin program_sweep
    S=target/release/program_sweep.exe; B=".reference/<backup>.ns4b"
    $S "$B" --legacy-arp --legacy-morph > old.jsonl     # engine before D166
    $S "$B" > new.jsonl                                  # new defaults
    uv run python tools/sweep_flip_compare.py old.jsonl new.jsonl --csv out.csv

The CSV holds derived numbers only: program name, its arpeggiator layers (`layer:mode:range raw`),
whether it carries a morph slot, and whether the default / firmware-all render changed.
"""

from __future__ import annotations

import argparse
import csv
import json
import sys
from pathlib import Path

MORPH_KEYS = {"aftertouch", "ctrl_pedal", "wheel"}


def load(path: Path) -> dict[str, dict]:
    """`entry -> {"renders": ..., "program": ...}` of a sweep JSONL file."""
    out: dict[str, dict] = {}
    with path.open(encoding="utf-8") as f:
        for line in f:
            if line.strip():
                d = json.loads(line)
                out[d["entry"]] = {"renders": d["renders"], "program": d["program"]}
    return out


def arp_layers(program: dict) -> list[str]:
    """`A:0:47` = layer A, decoded arp mode 0, raw RANGE 47, for each layer that runs the arp."""
    synth = program.get("synth", {})
    out = []
    for letter in "abc":
        layer = synth.get("layer_" + letter) or {}
        if layer.get("arp_enabled"):
            arp = layer.get("arp", {})
            out.append(f"{letter.upper()}:{arp.get('mode')}:{arp.get('range')}")
    return out


def has_morph_slot(node: object) -> bool:
    """True when any morph delta (wheel / aftertouch / pedal) in the decoded program is non-zero."""
    if isinstance(node, dict):
        if MORPH_KEYS <= set(node) and any(node[k] for k in MORPH_KEYS):
            return True
        return any(has_morph_slot(v) for v in node.values())
    if isinstance(node, list):
        return any(has_morph_slot(v) for v in node)
    return False


def compare(old: dict[str, dict], new: dict[str, dict]) -> list[dict]:
    """One row per program present in both runs."""
    rows = []
    for entry in sorted(old.keys() & new.keys()):
        o, n = old[entry]["renders"], new[entry]["renders"]
        rows.append(
            {
                "program": entry.removeprefix("Program/").removesuffix(".ns4p"),
                "arp_layers": " ".join(arp_layers(new[entry]["program"])),
                "morph_slot": has_morph_slot(new[entry]["program"]),
                "default_changed": o.get("default") != n.get("default"),
                "firmware_changed": o.get("firmware") != n.get("firmware"),
            }
        )
    return rows


def summarize(rows: list[dict]) -> dict:
    changed = [r for r in rows if r["default_changed"] or r["firmware_changed"]]
    return {
        "programs": len(rows),
        "changed": len(changed),
        "changed_with_arp": sum(1 for r in changed if r["arp_layers"]),
        "changed_with_morph_slot": sum(1 for r in changed if r["morph_slot"]),
        "changed_without_arp_or_morph": sum(
            1 for r in changed if not r["arp_layers"] and not r["morph_slot"]
        ),
        "arp_programs": sum(1 for r in rows if r["arp_layers"]),
        "morph_programs": sum(1 for r in rows if r["morph_slot"]),
    }


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("old", type=Path)
    ap.add_argument("new", type=Path)
    ap.add_argument("--csv", type=Path, help="write the per-program rows here")
    args = ap.parse_args(argv)
    rows = compare(load(args.old), load(args.new))
    if args.csv:
        with args.csv.open("w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=list(rows[0]) if rows else [])
            w.writeheader()
            w.writerows(rows)
    print(json.dumps(summarize(rows), indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
