#!/usr/bin/env python3
# ruff: noqa: E501
r"""Per-program native against oracle envelope error, stale against settled rate tables (D212).

For a sample of synth layers of the factory programs (`synth_env_census` rows), the stored amp and
filter rate knobs are injected into the ORG core 2 oracle as ADR words (same ARM-word injection as
`synth_env_trace.py`, key velocity byte 127 so the amp attack is unscaled), the note is held
`--hold` passes and released, and the amp and filter chain levels are tapped every pass. The
native chains (`synth_env_native`, `--sustain-half` so both decay records stop at the oracle's 0.5)
are run with the stale tables (`--rates legacy`) and the settled tables (`--rates settled`).

Per chain and variant the CSV holds the mean and max absolute level error over the window and the
times (passes) the level needs to reach 90 % of the oracle peak after the key on and to fall to
half of its level after the key off, oracle and native.

    uv run python tools/synth_env_programs.py run CAPTURE.txt.gz --sharc-sys target/release/sharc-sys \
        --image DIR/core2_sram.bin --native target/release/synth_env_native \
        --programs factory_programs/Program --census census.jsonl --out programs.csv
"""

from __future__ import annotations

import argparse
import csv
import json
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np
import synth_env_trace as t

FIELDS = (
    "program", "layer", "chain", "variant", "attack_idx", "decay_idx", "release_idx",
    "oracle_peak", "mean_abs_err", "max_abs_err",
    "t90_oracle", "t90_native", "thalf_oracle", "thalf_native",
)  # fmt: skip
VARIANTS = ("legacy", "settled")
ON = 300
PEAK_CLEAN = 1.0001  # amp chains that overshoot 1 (oracle limit above 1, D212) are flagged


def mean_max_error(oracle: np.ndarray, native: np.ndarray) -> tuple[float, float]:
    """Mean and max absolute difference of two equally long level traces."""
    d = np.abs(np.asarray(oracle, dtype=np.float64) - np.asarray(native, dtype=np.float64))
    return float(d.mean()), float(d.max())


def pass_to_reach(y: np.ndarray, start: int, level: float, stop: int | None = None) -> int | None:
    """Passes after `start` until `y` first reaches `level` (None if it never does before `stop`)."""
    seg = np.asarray(y[start:stop])
    hit = np.nonzero(seg >= level)[0]
    return int(hit[0]) if hit.size else None


def pass_to_fall(y: np.ndarray, start: int, frac: float = 0.5) -> int | None:
    """Passes after `start` until `y` first drops to `frac` of `y[start - 1]` (None: never)."""
    ref = float(y[start - 1])
    if ref <= 0.0:
        return None
    seg = np.asarray(y[start:])
    hit = np.nonzero(seg <= frac * ref)[0]
    return int(hit[0]) if hit.size else None


def sample_rows(rows: list[dict], n: int) -> list[dict]:
    """About `n` layer rows spread evenly over the census order (rows without `program` skipped)."""
    rows = [r for r in rows if "program" in r]
    step = max(1, len(rows) // n)
    return rows[::step][:n]


def chain_row(
    prog: str, layer: int, chain: str, variant: str, idx: tuple, ora: np.ndarray, nat: np.ndarray,
    off: int,
) -> dict:  # fmt: skip
    mean, mx = mean_max_error(ora, nat)
    peak = float(ora[ON:off].max())
    return {
        "program": prog, "layer": layer, "chain": chain, "variant": variant,
        "attack_idx": idx[0], "decay_idx": idx[1], "release_idx": idx[2],
        "oracle_peak": f"{peak:.4f}", "mean_abs_err": f"{mean:.5f}", "max_abs_err": f"{mx:.5f}",
        "t90_oracle": pass_to_reach(ora, ON, 0.9 * min(peak, 1.0), off),
        "t90_native": pass_to_reach(nat, ON, 0.9 * min(peak, 1.0), off),
        "thalf_oracle": pass_to_fall(ora, off),
        "thalf_native": pass_to_fall(nat, off),
    }  # fmt: skip


def run_layer(ns: argparse.Namespace, rows: list[dict], row: dict, work: Path) -> list[dict]:
    import synth_e2e_oracle as o

    prog, layer = row["program"], int(row["layer"])
    path = Path(ns.programs) / f"{prog}.ns4p"
    off = ON + ns.hold
    passes = off + ns.tail
    tag = f"{prog.replace('/', '_').replace(' ', '_')}_{layer}"
    extra = ("--layer", str(layer), "--sustain-half", "--rates")
    nats = {
        v: t.run_native(ns.native, str(path), ON, off, passes, 0, (*extra, v)) for v in VARIANTS
    }
    knobs = nats["legacy"]["knobs"]
    words = t.program_words(knobs, nats["legacy"]["cutoff_knob"], int(nats["legacy"]["filter_env_amount_knob"]))  # fmt: skip
    (work / f"adr_{tag}.script").write_text(t.inject_script(words))
    (work / f"key_{tag}.script").write_text(t.key_byte_script(o.key_script(rows, ON, off), 127))
    env, _ = t.run_oracle(
        ns.sharc_sys, ns.image,
        [work / "load.script", work / f"adr_{tag}.script", work / f"key_{tag}.script"],
        passes, work / f"trace_{tag}.bin",
    )  # fmt: skip
    out = []
    for k, chain in enumerate(("amp", "filter")):
        e = row[chain]
        idx = (e["attack"], e["decay"], e["release"])
        for v in VARIANTS:
            out.append(chain_row(prog, layer, chain, v, idx, env[:, k], np.array(nats[v][chain]), off))  # fmt: skip
    return out


def summarize(rows: list[dict]) -> dict:
    """Per chain and variant: layer count, mean / median / max of the mean absolute error, the
    number of layers with a mean error above 0.01 and the median |log2| ratio of the release
    half time (native over oracle). Chains whose oracle peak exceeds 1 are counted apart."""
    out: dict = {}
    for chain in ("amp", "filter"):
        for v in VARIANTS:
            sel = [r for r in rows if r["chain"] == chain and r["variant"] == v]
            clean = [r for r in sel if float(r["oracle_peak"]) <= PEAK_CLEAN]
            err = np.array([float(r["mean_abs_err"]) for r in clean])
            ratios = [
                abs(np.log2(float(r["thalf_native"]) / float(r["thalf_oracle"])))
                for r in clean
                if r["thalf_native"] and r["thalf_oracle"]
            ]
            out[f"{chain}.{v}"] = {
                "layers": len(sel),
                "peak_above_1": len(sel) - len(clean),
                "mean_err_mean": float(err.mean()) if err.size else None,
                "mean_err_median": float(np.median(err)) if err.size else None,
                "mean_err_max": float(err.max()) if err.size else None,
                "layers_err_above_0.01": int((err > 0.01).sum()),
                "thalf_log2_median": float(np.median(ratios)) if ratios else None,
                "thalf_n": len(ratios),
            }
    return out


def summary_cmd(ns: argparse.Namespace) -> int:  # pragma: no cover - CLI wiring
    rows = list(csv.DictReader(Path(ns.csv).open(encoding="utf-8")))
    print(json.dumps(summarize(rows), indent=1, sort_keys=True))
    return 0


def run(ns: argparse.Namespace) -> int:  # pragma: no cover - needs local firmware images
    import arm_dsp_capture as cap
    import synth_e2e_oracle as o

    rows = list(cap.read_rows(Path(ns.capture)))
    work = Path(ns.workdir)
    work.mkdir(parents=True, exist_ok=True)
    (work / "load.script").write_text(o.core2_load_script(rows))
    census = [json.loads(x) for x in Path(ns.census).read_text(encoding="utf-8").splitlines()]
    sample = sample_rows(census, ns.n)
    with ThreadPoolExecutor(max_workers=ns.jobs) as pool:
        futs = [pool.submit(run_layer, ns, rows, r, work) for r in sample]
        res = [x for f in futs for x in f.result()]
    with Path(ns.out).open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=FIELDS, lineterminator="\n")
        w.writeheader()
        w.writerows(res)
    return 0


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - CLI wiring
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sm = sub.add_parser("summary")
    sm.add_argument("csv")
    r = sub.add_parser("run")
    r.add_argument("capture")
    for a in ("--sharc-sys", "--image", "--native", "--programs", "--census", "--out", "--workdir"):
        r.add_argument(a, required=True)
    r.add_argument("--n", type=int, default=30)
    r.add_argument("--hold", type=int, default=12000)
    r.add_argument("--tail", type=int, default=6000)
    r.add_argument("--jobs", type=int, default=4)
    ns = ap.parse_args(argv)
    return summary_cmd(ns) if ns.cmd == "summary" else run(ns)


if __name__ == "__main__":
    sys.exit(main())
