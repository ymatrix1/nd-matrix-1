#!/usr/bin/env python3
# ruff: noqa: E501
"""Resonance law of the synth filter stages, measured on the ORG core 2 oracle (D214).

The pass writes the stage prologue words into the channel 0 frame every pass: `R = DM(I3-30)`,
the resonance position word `W28 = DM(I3-28)`, the input gain `cin = DM(I3-26)`, `g = DM(I3-22)`
and `k = DM(I3-20)` (`I3 = 0xC2942`, `docs/research/synth-stage-equations.md` s2 and s3.1). With
`g = 2 R N` and `k = P N` the damping the stage used is `P = 2 R k / g`, which this tool reads
for the four stages with that prologue (LP12 `0x1253BD`, LP24 `0x12566E`, BP `0x125913`,
HP `0x125A50`) and compares with the interpolated table (`B` for all but LP12, `D` for LP12) and
with the OpenNord closed form `2 (1 - v / 128)^2` (numbers only, `tools/synth_scaling_crosscheck.py`).

The smoother (`w10 <- 0.995 w10 + 0.005 w`, `tools/synth_filter_links.py`) moves `W28` every
pass, so a run with resonance knob 127 (and one with knob 0) sweeps the position over 64 .. 128
(and 64 .. 0) and every pass is a (position, P) pair; `settled` runs each knob 0 .. 127 until the
smoother has settled.

    cargo build --release -p nord-sharc
    uv run python tools/ns4_dsp_ldr.py target/DSP/dsp_image_2_ORG.bin --sram DIR
    NS4_SHARC_ORG=target/release/sharc-org NS4_ORG_IMAGE=DIR/core2_sram.bin \
        uv run python tools/synth_res_oracle.py sweep --csv docs/research/data/synth-res-sweep.csv
    ... settled --csv docs/research/data/synth-res-settled.csv
"""

from __future__ import annotations

import argparse
import csv
import struct
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import synth_adr_oracle as adr
from synth_cutoff_note_probe import knob_word, voice_word
from synth_filter_links import COMMON_RES_WORD, position, res_word

DATA = Path(__file__).resolve().parents[1] / "docs/research/data"
# VERIFIED (run, `docs/research/synth-stage-equations.md` s2): channel 0 B stage slot word and the
# frame fields; the buffer starts at `I3 - 30` and holds R, W28, cin, (-24), g, k as PEx/PEy pairs.
SLOT_WORD = 0xC0B8C
FRAME_R = 0xC2924
BUF_WORDS = 20
OFF_R, OFF_W28, OFF_CIN, OFF_G, OFF_K = 0, 2, 4, 8, 10
# Ladder24 coefficient words `DM(I3-18, -16, -14, -12)` (`tools/synth_filter_ladder_strip.py`).
OFF_LADDER = (12, 14, 16, 18)
LADDER_STAGE = 0x12549D
STAGES = {
    "LP12": 0x1253BD,
    "LP24": 0x12566E,
    "BP": 0x125913,
    "HP": 0x125A50,
}
SETTLE_PASSES = 1500


def u2f(u: int) -> float:
    """float of float32 bits."""
    return struct.unpack("<f", struct.pack("<I", u & 0xFFFFFFFF))[0]


def quadratic_k(v: float) -> float:
    """OpenNord resonance damping `2 (1 - v / 128)^2` (their closed form, numbers only)."""
    return 2.0 * (1.0 - v / 128.0) ** 2


def damping_from_frame(r: float, g: float, k: float) -> float:
    """`P = 2 R k / g` from the stage prologue words (`g = 2 R N`, `k = P N`)."""
    return 2.0 * r * k / g


def interp(table: list[float], pos: float) -> float:
    """Linear interpolation of a 129 entry table at position `pos` (`0..127.999`)."""
    i = min(int(pos), 127)
    fr = pos - i
    return table[i] + (table[i + 1] - table[i]) * fr


def read_tables() -> dict[str, list[float]]:
    """`{"A", "C", "B", "D"}` float tables from the research CSVs."""
    out: dict[str, list[float]] = {"A": [], "C": [], "B": [], "D": []}
    with (DATA / "synth-filter-tables.csv").open(encoding="utf-8") as fh:
        for row in csv.DictReader(fh):
            out["A"].append(float(row["A_0xB0D0F"]))
            out["C"].append(float(row["C_0xB0D90"]))
            out["B"].append(float(row["B_0xB1015"]))
    with (DATA / "synth-filter-table-d.csv").open(encoding="utf-8") as fh:
        for row in csv.DictReader(fh):
            out["D"].append(u2f(int(row["word"], 16)))
    return out


def table_for(stage: str) -> str:
    """Damping table of a stage: `D` for LP12, `B` otherwise."""
    return "D" if stage == "LP12" else "B"


def gain_table_for(stage: str) -> str:
    """Input gain table of a stage: `C` for the band pass, `A` otherwise."""
    return "C" if stage == "BP" else "A"


def run(org: str, image: str, stage: int, knob: int, passes: int) -> list[dict]:
    """Per pass `{pos, r, cin, g, k, p}` of channel 0 with `stage` in slot 0 and resonance `knob`."""
    msgs = [(0, 3, knob_word(80)), (0, 0x2E9, knob_word(80)), (0, COMMON_RES_WORD, res_word(knob))]
    pokes = [(p, SLOT_WORD, stage) for p in range(1, 4)]
    rows = adr.run_script(
        org,
        image,
        passes,
        msgs=msgs,
        pre={adr.CONTROLLER: voice_word(60)},
        pokes=pokes,
        bufs=[f"{FRAME_R:#x}:{BUF_WORDS}"],
    )
    out = []
    for p in range(1, passes):
        w = rows[(p, 0)]
        r, g, k = u2f(w[OFF_R]), u2f(w[OFF_G]), u2f(w[OFF_K])
        out.append(
            {
                "pos": position(w[OFF_W28]),
                "r": r,
                "cin": u2f(w[OFF_CIN]),
                "g": g,
                "k": k,
                "p": damping_from_frame(r, g, k) if g > 0 else float("nan"),
                "w30": w[OFF_R],
                "w28": w[OFF_W28],
                "ladder": tuple(w[o] for o in OFF_LADDER),
            }
        )
    return out


def sweep_errors(
    rows: list[dict], tab: list[float], gain: list[float]
) -> tuple[float, float, float]:
    """`(max rel err of P, max rel err of cin, max rel deviation of the table from the quadratic)`.

    Rows whose table `P` is below 0.02 are skipped for the `P` error: `2 R k / g` loses float32
    digits there; the cin error uses all rows. The third value is the largest `table / quadratic`
    ratio deviation over the rows, the OpenNord comparison at the measured positions."""
    ep = ec = eq = 0.0
    for r in rows:
        if r["p"] != r["p"]:
            continue
        want = interp(tab, r["pos"])
        if want >= 0.02:
            ep = max(ep, abs(r["p"] / want - 1))
        ec = max(ec, abs(r["cin"] / interp(gain, r["pos"]) - 1))
        q = quadratic_k(r["pos"])
        if q > 1e-9:
            eq = max(eq, abs(r["p"] / q - 1))
    return ep, ec, eq


def ladder_mismatches(rows: list[dict]) -> tuple[int, int]:
    """`(passes whose Ladder24 coefficient words differ from the Python model, passes)`.

    The model is `tools/synth_filter_ladder.py::coefficients(R word, W28 word)`, bit exact against
    the stage; here the words come from a live pass with the smoothed resonance position."""
    import synth_filter_ladder as lad

    bad = sum(1 for r in rows if tuple(lad.coefficients(r["w30"], r["w28"])) != r["ladder"])
    return bad, len(rows)


def settled_row(org: str, image: str, knob: int) -> list[object]:
    """One settled run (LP24 and LP12): knob, positions, P, cin."""
    tail = {
        name: run(org, image, STAGES[name], knob, SETTLE_PASSES)[-1] for name in ("LP24", "LP12")
    }
    a = tail["LP24"]
    return [knob, a["pos"], tail["LP12"]["pos"], a["p"], tail["LP12"]["p"], a["cin"]]


def write_csv(path: str | None, header: list[str], rows: list[list[object]]) -> None:
    """Writes `rows` under `header` when `path` is given (derived numbers only)."""
    if not path:
        return
    with open(path, "w", newline="", encoding="utf-8") as fh:
        w = csv.writer(fh, lineterminator="\n")
        w.writerow(header)
        w.writerows(rows)


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the firmware image
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("what", choices=("sweep", "settled"))
    ap.add_argument("--csv")
    ap.add_argument("--passes", type=int, default=SETTLE_PASSES)
    ns = ap.parse_args(argv)
    org, image = adr.default_paths()
    tabs = read_tables()
    if ns.what == "sweep":
        out: list[list[object]] = []
        for name, pc in STAGES.items():
            for knob in (0, 127):
                rows = run(org, image, pc, knob, ns.passes)
                ep, ec, eq = sweep_errors(rows, tabs[table_for(name)], tabs[gain_table_for(name)])
                lo, hi = min(r["pos"] for r in rows), max(r["pos"] for r in rows)
                print(
                    f"{name:5s} knob {knob:3d} pos {lo:7.3f}..{hi:7.3f} P err {ep:.2e} cin err {ec:.2e}"
                )
                out.append([name, knob, f"{lo:.3f}", f"{hi:.3f}", f"{ep:.3e}", f"{ec:.3e}"])
        for knob in (0, 127):
            bad, n = ladder_mismatches(run(org, image, LADDER_STAGE, knob, ns.passes))
            print(f"Ladder knob {knob:3d} coefficient word mismatches {bad} of {n}")
            out.append(["Ladder", knob, "", "", f"mismatch {bad}/{n}", ""])
        write_csv(ns.csv, ["stage", "knob", "pos_min", "pos_max", "p_rel_err", "cin_rel_err"], out)
        return 0
    with ThreadPoolExecutor(max_workers=16) as ex:
        res = list(ex.map(lambda k: settled_row(org, image, k), range(128)))
    rows2 = [
        [k, f"{a:.4f}", f"{b:.4f}", f"{p:.6g}", f"{q:.6g}", f"{c:.6g}"] for k, a, b, p, q, c in res
    ]
    head = ["knob", "pos_lp24", "pos_lp12", "p_lp24", "p_lp12", "cin_lp24"]
    write_csv(ns.csv, head, rows2)
    print("wrote", len(rows2), "rows")
    return 0


if __name__ == "__main__":
    sys.exit(main())
