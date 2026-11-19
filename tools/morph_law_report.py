#!/usr/bin/env python3
"""Fits the firmware morph weight law to the single-event rows of `arm_program_morph.py` (ns4-kq1o).

For every `(store, id, source)` series of `docs/research/data/morph-fw-law.csv` the unclamped
points are fitted by `value(v) = a + b * w(v)` with the two outermost unclamped points as anchors,
once for the firmware law `w = v / 128` (full at 127) and once for the native law `w = v / 127`.
The largest residual of each law, in edit steps, is written per series; a law that matches leaves
a residual near 0.

    python tools/morph_law_report.py docs/research/data/morph-fw-law.csv \
        --out docs/research/data/morph-fw-law-fit.csv
"""

from __future__ import annotations

import argparse
import csv
import io
import math
import re
import sys
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent))

import arm_program_morph as pm

FIELDS = ("store", "id", "name", "source", "points", "span_steps", "resid_fw", "resid_native")
# VERIFIED (arm-edit-params.csv max column): every morph id spans 0..127 except the 14-bit delay
# rate, 0..16383 (`Edt_Effects_kDlyRateClkDiv`, id 0x1AF).
WIDE_IDS = {0x1AF: 16383.0}


def bounds(edit_id: int) -> tuple[float, float]:
    """Clamp range of the edit id."""
    return 0.0, WIDE_IDS.get(edit_id, 127.0)


def read_series(text: str) -> dict[tuple[str, int, str, str], dict[int, float]]:
    """`{(store, id, name, source): {v: value}}` of the single-event scenarios of a CSV."""
    out: dict[tuple[str, int, str, str], dict[int, float]] = {}
    for row in csv.DictReader(io.StringIO(text)):
        events = pm.parse_scenario(row["scenario"])
        if len(events) != 1:
            continue
        source, v = events[0]
        key = (row["store"], int(row["id"], 0), row["name"], source)
        out.setdefault(key, {})[v] = float(row["value"])
    return out


def fit_series(points: dict[int, float], lo: float, hi: float) -> tuple[int, float, float, float]:
    """`(points used, span, resid fw, resid native)`; residuals are NaN-free zeros when the series
    has fewer than two distinct unclamped points."""
    eps = 1e-3
    free = {v: x for v, x in points.items() if lo + eps < x < hi - eps}
    if len(free) < 2:
        return len(free), 0.0, 0.0, 0.0
    v0, v1 = min(free), max(free)
    span = free[v1] - free[v0]
    if abs(span) < eps:
        return len(free), 0.0, 0.0, 0.0
    resid = []
    for weight in (pm.fw_weight, pm.native_weight):
        w0, w1 = weight(v0), weight(v1)
        worst = 0.0
        for v, x in free.items():
            pred = free[v0] + span * (weight(v) - w0) / (w1 - w0)
            worst = max(worst, abs(x - pred))
        resid.append(worst)
    return len(free), span, resid[0], resid[1]


def report_rows(text: str) -> list[tuple]:
    """CSV rows `FIELDS` of every series that moves."""
    rows = []
    for (store, edit_id, name, source), pts in sorted(read_series(text).items()):
        lo, hi = bounds(edit_id)
        n, span, fw, native = fit_series(pts, lo, hi)
        if n >= 2 and span != 0.0:
            rows.append(
                (store, hex(edit_id), name, source, n, f"{span:.4f}", f"{fw:.4f}", f"{native:.4f}")
            )
    return rows


def rows_csv(rows: list[tuple]) -> str:
    """CSV text of `FIELDS` rows."""
    buf = io.StringIO()
    writer = csv.writer(buf, lineterminator="\n")
    writer.writerow(FIELDS)
    writer.writerows(rows)
    return buf.getvalue()


# VERIFIED (run, `arm_synth_freq_law.py`, docs/research/data/synth-filter-freq-law.csv): the cutoff
# word of an integer knob is `(knob - 52) << 24` (top byte `knob - 52`) up to knob 117.
CUTOFF_WORD_OFFSET = 52
CUTOFF_FIELDS = (
    "layer",
    "wheel",
    "dispatch_knob",
    "sent_knob",
    "fw_word",
    "fw_hz",
    "native_knob",
    "native_hz",
    "native_vs_fw_db",
)


def sender_knob(x: float) -> float:
    """Knob position the cutoff sender `0xC01C56CC` puts into the word for a Q16.16 value `x`.

    VERIFIED (static dispatch run, 11 fractional values of 54.0 .. 55.0 and 60.25 / 60.5): the
    word is `(x - 52) << 24` while the fraction of `x` is below 0.5 and one whole step lower at
    0.5 and above (`54.4375 -> 0x02700000`, `54.5 -> 0x01800000`, `54.75 -> 0x01C00000`)."""
    return x - (1.0 if x - math.floor(x) >= 0.5 else 0.0)


def cutoff_word(knob: float) -> int:
    """32-bit word of a (fractional) knob: `(knob - 52)` in Q8.24, two's complement."""
    return int(round((knob - CUTOFF_WORD_OFFSET) * (1 << 24))) & 0xFFFFFFFF


def cutoff_table(rust_text: str) -> list[float]:
    """The 128 Hz entries of `FILTER_CUTOFF_HZ_TABLE` in `cbin/presets.rs`."""
    m = re.search(r"pub const FILTER_CUTOFF_HZ_TABLE: \[f32; 128\] = \[(.*?)\];", rust_text, re.S)
    if not m:
        raise ValueError("FILTER_CUTOFF_HZ_TABLE not found")
    return [float(x) for x in re.findall(r"[0-9]+\.[0-9]+", m.group(1))]


def interp_hz(table: list[float], knob: float) -> float:
    """Cutoff Hz at a fractional knob, geometric between steps (`cutoff_norm_to_hz`; the firmware
    interpolation between two table entries is `ASSUMED` the same)."""
    pos = max(0.0, min(127.0, knob))
    lo = min(int(pos), 127)
    hi = min(lo + 1, 127)
    return table[lo] * (table[hi] / table[lo]) ** (pos - lo)


def cutoff_rows(
    table: list[float], layers: dict[str, tuple[int, int]], wheel_values: tuple[int, ...]
) -> list[tuple]:
    """Rows `CUTOFF_FIELDS`: the firmware dispatch `base + depth * v / 128`, the knob the sender
    turns it into, its word and Hz, against the native knob `base + depth * v / 127`."""
    rows = []
    for layer, (base, depth) in layers.items():
        for v in wheel_values:
            fw = base + depth * pm.fw_weight(v)
            sent = sender_knob(fw)
            native = base + depth * pm.native_weight(v)
            fw_hz, nat_hz = interp_hz(table, sent), interp_hz(table, native)
            rows.append(
                (
                    layer,
                    v,
                    f"{fw:.4f}",
                    f"{sent:.4f}",
                    f"{cutoff_word(sent):#010x}",
                    f"{fw_hz:.1f}",
                    f"{native:.4f}",
                    f"{nat_hz:.1f}",
                    f"{20 * math.log10(nat_hz / fw_hz):.2f}",
                )
            )
    return rows


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("csv", type=Path)
    ap.add_argument("--out", type=Path)
    ap.add_argument("--cutoff-out", type=Path, help="Horizon Whl cutoff table (needs --presets-rs)")
    ap.add_argument("--presets-rs", type=Path, help="crates/nord-core/src/cbin/presets.rs")
    ns = ap.parse_args(argv)
    if ns.cutoff_out and ns.presets_rs:
        table = cutoff_table(ns.presets_rs.read_text(encoding="utf-8"))
        # Horizon Whl cells (run): synth A knob 52 + wheel 10, synth C knob 34 + wheel 13.
        layers = {"A": (52, 10), "C": (34, 13)}
        rows = cutoff_rows(table, layers, (0, 1, 32, 64, 96, 127))
        buf = io.StringIO()
        writer = csv.writer(buf, lineterminator="\n")
        writer.writerow(CUTOFF_FIELDS)
        writer.writerows(rows)
        ns.cutoff_out.write_text(buf.getvalue(), encoding="utf-8")
    rows = report_rows(ns.csv.read_text(encoding="utf-8"))
    text = rows_csv(rows)
    if ns.out:
        ns.out.write_text(text, encoding="utf-8")
    worst_fw = max((float(r[6]) for r in rows), default=0.0)
    worst_nat = max((float(r[7]) for r in rows), default=0.0)
    print(
        f"{len(rows)} series, worst residual: firmware {worst_fw:.4f} steps, native {worst_nat:.4f}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
