#!/usr/bin/env python3
# ruff: noqa: E501
r"""Amp chain level limit above 1 of the ORG core 2 oracle (D212).

The amp chain of slot 0 (`F - 20`, envelope loop `0x71EB..`) clamps its level to `L = DM(F - 22)`;
the image holds 4.0 there before any key and the oracle shows `L = 1.0` for every key below
`g = 99`, and up to 4.0 for a hard key with a fast attack and a fast decay. This tool taps `L`
20 passes after the key on for a grid of `(attack word, decay word, g)` and checks the law

    k = max(0, ((G - 97) >> 1) - (e - e0) - (d + 64)),    L = 1 + 0.2 k

with `G` the effective key byte (128 for `g = 127`), `e = (w * G) >> 7` the velocity scaled amp
attack index (`fw_adr_law`), `e0 = (-64 * G) >> 7` its lowest value and `d` the decay word.

    uv run python tools/synth_env_limit.py grid CAPTURE.txt.gz --sharc-sys target/release/sharc-sys \
        --image DIR/core2_sram.bin --workdir target/w --out docs/research/data/synth-env-amp-limit.csv
    uv run python tools/synth_env_limit.py check docs/research/data/synth-env-amp-limit.csv
"""

from __future__ import annotations

import argparse
import csv
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np
import synth_env_law as law
import synth_env_trace as t

LIMIT_TAP = (
    t.ENV_FRAME - 22
)  # DM(F - 22), VERIFIED (synth-adr-and-strip-law s2): chain 1 level limit
STEP = 0.2
FIELDS = ("key", "g", "attack", "decay", "limit")


def effective_g(g: int) -> int:
    """`G`: the key byte 127 acts as 128 (VERIFIED, synth-velocity)."""
    return 128 if g >= 127 else g


def limit_k(attack: int, decay: int, g: int) -> int:
    """Number of `0.2` steps of the limit above 1 (empirical law, see the module doc)."""
    big = effective_g(g)
    e = (attack * big) >> 7 if attack < 0 else attack
    e0 = (-64 * big) >> 7
    return max(0, ((big - 97) >> 1) - (e - e0) - (decay + 64))


def amp_limit(attack: int, decay: int, g: int) -> float:
    """Level limit of the amp chain for an attack word, a decay word and key byte `g`."""
    return round(1.0 + STEP * limit_k(attack, decay, g), 6)


def run_one(ns: argparse.Namespace, rows: list[dict], key: int, g: int, a: int, d: int, tag: str):
    import synth_e2e_oracle as o

    work = Path(ns.workdir)
    on, off = 300, 500
    (work / f"k{tag}.script").write_text(t.key_byte_script(o.key_script(rows, on, off, key), g))
    (work / f"a{tag}.script").write_text(t.inject_script(law.case_words(a, d, -64)))
    out = work / f"t{tag}.bin"
    cmd = [ns.sharc_sys, ns.image, "--passes", str(off + 50), "--warmup", "0", "--epdma"]
    cmd += ["--script-flat", str(work / "load.script")]
    cmd += [
        "--script-at",
        f"0:{work / f'a{tag}.script'}",
        "--script-at",
        f"0:{work / f'k{tag}.script'}",
    ]
    cmd += ["--tap", f"{LIMIT_TAP:#x}:1", "--tap-out", str(out)]
    subprocess.run(cmd, capture_output=True, text=True, check=True)
    y = np.fromfile(out, dtype="<u4").view("<f4")
    return {"key": key, "g": g, "attack": a, "decay": d, "limit": f"{float(y[on + 20]):.6f}"}


def grid(ns: argparse.Namespace) -> int:  # pragma: no cover - needs local firmware images
    import arm_dsp_capture as cap
    import synth_e2e_oracle as o

    rows = list(cap.read_rows(Path(ns.capture)))
    work = Path(ns.workdir)
    work.mkdir(parents=True, exist_ok=True)
    (work / "load.script").write_text(o.core2_load_script(rows))
    jobs = [
        (k, g, a, d)
        for k in (60, 36, 84)
        for g in (60, 98, 99, 104, 110, 118, 126, 127)
        for a in (-64, -63, -62, -60, -56, -52, -50, -40, 0)
        for d in (-64, -63, -62, -58, -54, -50, -30, 0)
        if k == 60 or (a in (-64, -60) and d in (-64, -58) and g in (110, 127))
    ]
    with ThreadPoolExecutor(max_workers=ns.jobs) as pool:
        futs = [pool.submit(run_one, ns, rows, *j, str(i)) for i, j in enumerate(jobs)]
        res = [f.result() for f in futs]
    with Path(ns.out).open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=FIELDS, lineterminator="\n")
        w.writeheader()
        w.writerows(res)
    return 0


def check(ns: argparse.Namespace) -> int:  # pragma: no cover - CLI wiring
    rows = list(csv.DictReader(Path(ns.csv).open(encoding="utf-8")))
    bad = [
        r
        for r in rows
        if abs(amp_limit(int(r["attack"]), int(r["decay"]), int(r["g"])) - float(r["limit"])) > 1e-4
    ]
    print(f"law matches {len(rows) - len(bad)}/{len(rows)} rows")
    for r in bad[:10]:
        print("miss", r)
    return 0


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - CLI wiring
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    g = sub.add_parser("grid")
    g.add_argument("capture")
    g.add_argument("--sharc-sys", required=True)
    g.add_argument("--image", required=True)
    g.add_argument("--workdir", required=True)
    g.add_argument("--out", required=True)
    g.add_argument("--jobs", type=int, default=4)
    c = sub.add_parser("check")
    c.add_argument("csv")
    ns = ap.parse_args(argv)
    return grid(ns) if ns.cmd == "grid" else check(ns)


if __name__ == "__main__":
    sys.exit(main())
