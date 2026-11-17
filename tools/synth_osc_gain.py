#!/usr/bin/env python3
r"""Oscillator level per wave: SHARC oracle stage output against native `nord-core` (D136).

Runs the oscillator path only (ORG core 2, B stage replaced by the null stage, the real ARM
stream of `arm_dsp_capture.py`) once per program, caches the samples, then renders the same note
natively in four unison-1 variants and derives the row gain the native table would need to land on
the oracle level. Layer A only, files with unison 0 (one voice on both sides). Numbers only: no
audio leaves `--workdir`.

    uv run python tools/synth_osc_gain.py run CAPTURE.txt.gz PROGRAM.ns4p \
        --sharc-sys target/release/sharc-sys --image DIR/core2_sram.bin \
        --native target/release/synth_e2e_native --workdir target/synth_osc_gain > row.json
    uv run python tools/synth_osc_gain.py csv row1.json row2.json ... > table.csv

Variants (native, filter off): `default` (program envelope, velocity 100, fitted table row),
`g1` (`--unity`, table row gain 1: the oscillator stage times the fixed 0.25 voice constant),
`verified` (`--unity`, `--synth-p verified`: row gain `1.6 P[16] weight`), `unity` (`--unity`,
default table). `needed_row_gain` is the row gain at which the `--unity` native level equals the
oracle level.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np
import synth_e2e_oracle as e2e

# VERIFIED (render.rs `env_gain`): the fixed per-voice constant after the filter slot.
VOICE_CONST = 0.25
VARIANTS = ("default", "unity", "verified", "g1", "g1u0")


def row_arg(p_wave: str) -> str:
    """`--synth-p` rows: `p_wave` at gain 1 (shape knob scale 1 on the SHAPE rows)."""
    shape = 1.0 if p_wave in ("shape", "shapesaw", "shapesq", "shapesine") else 0.5
    return f"table:{p_wave}=1/0/1/{shape}/1"


def variant_args(name: str, p_wave: str) -> list[str]:
    """Native arguments of one variant (all filter off)."""
    base = ["--no-filter"]
    if name == "default":
        return base
    base = base + ["--unity"]
    if name == "unity":
        return base
    if name == "verified":
        return base + ["--synth-stages", "--synth-p", "verified"]
    if name == "g1":
        return base + ["--synth-p", row_arg(p_wave)]
    if name == "g1u0":  # one voice on the native side whatever the file unison says
        return base + ["--synth-p", row_arg(p_wave), "--unison", "0"]
    raise ValueError(name)


def needed_row_gain(oracle_db: float, g1_db: float) -> float:
    """Row gain (linear) that moves the native `g1` level onto the oracle level."""
    return float(10.0 ** ((oracle_db - g1_db) / 20.0))


def oracle_osc(ns: argparse.Namespace, rows: list[dict], work: Path) -> np.ndarray:
    """Oscillator path samples of channel 0 (cached as `osc.npy` in `work`)."""
    cache = work / "osc.npy"
    if cache.exists():
        return np.load(cache)
    off = e2e.KEY_ON_PASS + int(e2e.HOLD_S / e2e.PASS_S)
    passes = off + int(e2e.TAIL_S / e2e.PASS_S)
    files = {
        "load": e2e.core2_load_script(rows),
        "key": e2e.key_script(rows, e2e.KEY_ON_PASS, off),
        "nullb": e2e.null_b_script(passes),
    }
    paths = []
    for k, text in files.items():
        p = work / f"{k}.script"
        p.write_text(text)
        paths.append(p)
    osc, _ = e2e.run_oracle(ns.sharc_sys, ns.image, paths, passes, work / "osc.bin")
    np.save(cache, osc)
    return osc


def report(ns: argparse.Namespace) -> int:  # pragma: no cover - needs local firmware images
    import arm_dsp_capture as cap

    work = Path(ns.workdir) / Path(ns.program).stem.replace(" ", "_")
    work.mkdir(parents=True, exist_ok=True)
    rows = list(cap.read_rows(Path(ns.capture)))
    osc = oracle_osc(ns, rows, work)
    on_s = e2e.KEY_ON_PASS * e2e.PASS_S
    fr = (
        int((on_s + e2e.HOLD_S + e2e.TAIL_S) * e2e.NATIVE_FS),
        int(on_s * e2e.NATIVE_FS),
        int((on_s + e2e.HOLD_S) * e2e.NATIVE_FS),
    )
    nat, info = {}, {}
    _, probe = e2e.run_native(ns.native, ns.program, work / "n.f32", fr, ["--no-filter"])
    p_wave = probe["p_wave"]
    for v in VARIANTS:
        nat[v], info[v] = e2e.run_native(
            ns.native, ns.program, work / f"{v}.f32", fr, variant_args(v, p_wave)
        )
    stages = {v: e2e.stage_report(osc, nat[v], on_s, e2e.HOLD_S) for v in VARIANTS}
    o_db = stages["g1"]["oracle_rms_db"]
    out = {
        "program": Path(ns.program).stem,
        "p_wave": p_wave,
        "table_gain": probe["table_gain"],
        "oracle_rms_db": o_db,
        "native_rms_db": {v: stages[v]["native_rms_db"] for v in VARIANTS},
        "delta_db": {v: stages[v]["level_delta_db"] for v in VARIANTS},
        "band_delta_db": {v: stages[v]["band_delta_db_mean_abs"] for v in VARIANTS},
        "f0_oracle_hz": stages["g1"]["f0_oracle_hz_at_assumed_rate"],
        "f0_native_hz": stages["g1"]["f0_native_hz"],
        "needed_row_gain": needed_row_gain(o_db, stages["g1"]["native_rms_db"]),
        "needed_row_gain_u0": needed_row_gain(o_db, stages["g1u0"]["native_rms_db"]),
        "unison": probe["unison"],
        "layer": probe["layer"] if "layer" in probe else probe,
    }
    print(json.dumps(out, indent=1))
    return 0


CSV_FIELDS = (
    "program", "p_wave", "table_gain", "oracle_rms_db", "default_delta_db", "unity_delta_db",
    "verified_delta_db", "g1_delta_db", "needed_row_gain", "unison", "needed_row_gain_u0",
    "band_default", "band_verified", "f0_oracle_hz", "f0_native_hz",
)  # fmt: skip


def csv_row(r: dict) -> dict:
    """Flat CSV row of one `report` JSON document."""
    d, b = r["delta_db"], r["band_delta_db"]
    return {
        "program": r["program"],
        "p_wave": r["p_wave"],
        "table_gain": "" if r["table_gain"] is None else round(r["table_gain"], 3),
        "oracle_rms_db": round(r["oracle_rms_db"], 2),
        "default_delta_db": round(d["default"], 2),
        "unity_delta_db": round(d["unity"], 2),
        "verified_delta_db": round(d["verified"], 2),
        "g1_delta_db": round(d["g1"], 2),
        "needed_row_gain": round(r["needed_row_gain"], 3),
        "unison": r.get("unison", ""),
        "needed_row_gain_u0": round(r.get("needed_row_gain_u0", r["needed_row_gain"]), 3),
        "band_default": round(b["default"], 2),
        "band_verified": round(b["verified"], 2),
        "f0_oracle_hz": round(r["f0_oracle_hz"], 1),
        "f0_native_hz": round(r["f0_native_hz"], 1),
    }


def summarise(ns: argparse.Namespace) -> int:  # pragma: no cover - CLI wiring
    import csv
    import sys

    w = csv.DictWriter(sys.stdout, fieldnames=CSV_FIELDS, lineterminator=chr(10))
    w.writeheader()
    for p in ns.reports:
        w.writerow(csv_row(json.loads(Path(p).read_text(encoding="utf-8"))))
    return 0


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - CLI wiring
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("run")
    p.add_argument("capture")
    p.add_argument("program")
    p.add_argument("--sharc-sys", required=True)
    p.add_argument("--image", required=True)
    p.add_argument("--native", required=True)
    p.add_argument("--workdir", required=True)
    q = sub.add_parser("csv")
    q.add_argument("reports", nargs="+")
    ns = ap.parse_args(argv)
    return summarise(ns) if ns.cmd == "csv" else report(ns)


if __name__ == "__main__":
    raise SystemExit(main())
