#!/usr/bin/env python3
"""Delay of the Mod 2 CHOR type 4 copy in the SHARC oracle chain and in the native port.

Feeds an impulse train (one impulse every 600 DSP frames, longer than the 256 frame delay ring)
through the live FX chain of `tools/fx_oracle_live.py` and through `fx_live_native`, and reads the
offset of the delayed copy per impulse (`docs/research/fx-chorus-hw-clip.md` s8, D171).

    python tools/fx_chorus_oracle_delay.py IMAGE/core1_sram.bin OUTDIR --rate 30 --amount 64

Needs `cargo build --release -p nord-sharc -p nord-tools` and the local FX image
(`tools/ns4_dsp_ldr.py <FX image> --sram DIR`). Prints min, max and mean delay in ms.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Sequence

import fx_oracle_live as fo
import numpy as np

SPACING = 600  # frames between impulses
DIRECT_WINDOW = 8  # frames in which the dry impulse is searched
COPY_END = 400  # frames after the impulse in which the copy is searched


def impulse_train(kind: str, frames: int) -> np.ndarray:
    """Stereo impulse train in the shape `fx_oracle_live.signal` returns (`kind` is ignored)."""
    x = np.zeros(frames)
    x[::SPACING] = 0.5
    return np.stack([x, x], axis=1).astype(np.float32)


def copy_delays(y: np.ndarray) -> np.ndarray:
    """Rows `(time s, delay frames, copy / direct peak)` per impulse of a mono output."""
    rows = []
    for i in range(0, len(y) - SPACING, SPACING):
        seg = np.abs(y[i : i + SPACING])
        direct = int(np.argmax(seg[:DIRECT_WINDOW]))
        copy = int(np.argmax(seg[DIRECT_WINDOW:COPY_END])) + DIRECT_WINDOW
        rows.append((i / fo.FRAME_RATE, copy - direct, seg[copy] / max(seg[direct], 1e-12)))
    return np.array(rows)


def summary(rows: np.ndarray) -> dict[str, float]:
    ms = rows[:, 1] / fo.FRAME_RATE * 1000.0
    return {
        "min_ms": float(ms.min()),
        "max_ms": float(ms.max()),
        "mean_ms": float(ms.mean()),
        "copy_over_direct": float(np.median(rows[:, 2])),
        "impulses": float(len(ms)),
    }


def main(argv: Sequence[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("image")
    ap.add_argument("out")
    ap.add_argument("--rate", type=int, default=30)
    ap.add_argument("--amount", type=int, default=64)
    ap.add_argument("--seconds", type=float, default=6.0)
    a = ap.parse_args(argv)
    fo.signal = impulse_train
    ns = argparse.Namespace(
        family="chor4",
        image=a.image,
        signal="train",
        seconds=a.seconds,
        lead=3000,
        rate=a.rate,
        amount=a.amount,
        out=a.out,
        sharc=str(fo.REPO / "target/release/sharc-fxlive"),
        native=str(fo.REPO / "target/release/fx_live_native"),
    )
    fo.family_run(ns)
    skip = ns.lead * 4
    for name in ("oracle", "native"):
        y = fo.read_wav(Path(a.out) / f"{name}.wav")[skip:, 0]
        print(name, summary(copy_delays(y)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
