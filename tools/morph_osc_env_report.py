#!/usr/bin/env python3
"""Osc Env Amount morph: firmware dispatch and DSP word against the law (D206).

Reads the output of `tools/arm_program_morph.py --cells --dsp-all --csv X.csv` (stdout saved as
`X.log`) for programs whose synth layers carry an `Edt_Syn_kOscEnvAmt` morph, and checks per
scenario:

* dispatch: `x = clamp(base + sum d_s w(v_s), -60, 60)` with `w(v) = v / 128`, `1.0` at 127
  (`docs/research/morph-fw-vs-native.md` s2);
* DSP word: float32 `x / 60` in the layer's common 0 word 12 (`synth-knob-laws.md`).

    python tools/morph_osc_env_report.py target/morph_cap --csv out.csv

Findings: `docs/research/morph-fw-vs-native.md` s8. Prints counts only.
"""

from __future__ import annotations

import argparse
import csv
import re
import struct
import sys
from pathlib import Path

NAME = "Edt_Syn_kOscEnvAmt"
# VERIFIED (run, FM Harpies A / C, `data/morph-fw-osc-env.csv`): ORG core 2 DM word of the osc
# env amount of synth layer A and C (`0xE02E8 + 32 L + 12`, common 0 word 12).
# EMPIRICAL: layer B (Rich FM Pad Whl, two scenarios, exact `x / 60` at both) writes `0xE01FE`, not
# `0xE0314` (which stays 0); one program, cause not found.
DSP_WORDS = {0: 0xE02F4, 1: 0xE01FE, 2: 0xE0334}
FIELDS = (
    "program", "layer", "scenario", "base", "wheel", "at", "pedal",
    "law", "dispatch", "dsp_steps",
)  # fmt: skip
SOURCES = ("wheel", "at", "pedal")
CELL = re.compile(rf"CELL (\S+) (\d) synth {NAME} \((-?\d+), \((-?\d+), (-?\d+), (-?\d+)\)\)")


def weight(v: int) -> float:
    """Firmware morph weight: `v / 128`, exactly 1.0 at 127."""
    return 1.0 if v >= 127 else v / 128


def law(base: int, depths: tuple[int, int, int], events: dict[str, int]) -> float:
    """Dispatched value in amount steps (`-60..60`)."""
    x = base + sum(d * weight(events.get(s, 0)) for d, s in zip(depths, SOURCES, strict=True))
    return max(-60.0, min(60.0, x))


def parse_scenario(text: str) -> dict[str, int]:
    """`"wheel:127,at:64"` to `{"wheel": 127, "at": 64}`."""
    out: dict[str, int] = {}
    for item in text.split(","):
        src, _, val = item.partition(":")
        out[src] = int(val)
    return out


def parse_cells(log: str) -> dict[int, tuple[int, tuple[int, int, int]]]:
    """`{layer: (base, (wheel, at, pedal))}` of the osc env cells (first store of a layer)."""
    cells: dict[int, tuple[int, tuple[int, int, int]]] = {}
    for line in log.splitlines():
        m = CELL.match(line)
        if m:
            cells.setdefault(int(m[2]) - 4, (int(m[3]), (int(m[4]), int(m[5]), int(m[6]))))
    return cells


def float_of(word: str) -> float:
    """float32 of a `0x........` DSP word."""
    return struct.unpack(">f", struct.pack(">I", int(word, 16)))[0]


def rows_of(program: str, log: str, csv_text: str) -> list[dict]:
    """One row per layer and scenario with a dispatch of the osc env id."""
    cells = parse_cells(log)
    table = list(csv.DictReader(csv_text.splitlines()))
    out = []
    for layer, (base, depths) in sorted(cells.items()):
        scenarios = dict.fromkeys(r["scenario"] for r in table)
        for sc in scenarios:
            disp = [
                float(r["value"])
                for r in table
                if r["scenario"] == sc and r["name"] == NAME and r["client"] == str(layer + 4)
            ]
            if not disp:
                continue
            dsp = [
                float_of(r["value"]) * 60.0
                for r in table
                if r["scenario"] == sc
                and r["store"] == "dsp-all"
                and r["client"] == "1"
                and r["kind"] == "ORG"
                and int(r["id"], 16) == DSP_WORDS[layer]
            ]
            out.append(
                {
                    "program": program,
                    "layer": "ABC"[layer],
                    "scenario": sc,
                    "base": base,
                    "wheel": depths[0],
                    "at": depths[1],
                    "pedal": depths[2],
                    "law": f"{law(base, depths, parse_scenario(sc)):.4f}",
                    "dispatch": f"{disp[-1]:.4f}",
                    "dsp_steps": f"{dsp[-1]:.4f}" if dsp else "",
                }
            )
    return out


def verdict(row: dict) -> tuple[bool, bool | None]:
    """`(dispatch equals law, DSP word equals x / 60)`; the second is `None` without a word."""
    disp_ok = abs(float(row["dispatch"]) - float(row["law"])) < 1e-3
    if not row["dsp_steps"]:
        return disp_ok, None
    return disp_ok, abs(float(row["dsp_steps"]) - float(row["dispatch"])) < 2e-3


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("dir", type=Path, help="folder of `<program>.csv` + `<program>.log` pairs")
    ap.add_argument("--csv", type=Path)
    ns = ap.parse_args(argv)
    rows: list[dict] = []
    for path in sorted(ns.dir.glob("*.csv")):
        log = path.with_suffix(".log")
        if log.exists():
            rows += rows_of(
                path.stem.replace("_", " "),
                log.read_text(encoding="utf-8"),
                path.read_text(encoding="utf-8"),
            )
    checks = [verdict(r) for r in rows]
    print(f"rows {len(rows)}, dispatch = law {sum(c[0] for c in checks)}")
    with_word = [c[1] for c in checks if c[1] is not None]
    print(f"dsp word = x/60: {sum(with_word)} of {len(with_word)}")
    if ns.csv:
        with ns.csv.open("w", newline="", encoding="utf-8") as fh:
            w = csv.DictWriter(fh, fieldnames=FIELDS)
            w.writeheader()
            w.writerows(rows)
    return 0


if __name__ == "__main__":
    sys.exit(main())
