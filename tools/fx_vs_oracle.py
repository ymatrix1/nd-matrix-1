#!/usr/bin/env python3
"""Every wired native FX port against the firmware FX chip in the SHARC oracle, on live audio.

For each case (FX slot, type, rate, amount, stimulus) the routine's context comes from the port
(`fx_live_install`, i.e. `nord_core::fx::live_install`), is installed in stage 0 of the FX Core 1
dispatcher (`docs/research/fx-oracle-live.md`), and `sharc-fxlive` (oracle) and `fx_live_native`
(port through `LayerFxChain`) run the same deterministic stimulus. Numbers per case: level (fit gain
against the chain gain of the identity stage), null depth, spectral centroid, tail, octave band
gains. Method and results: `docs/research/fx-vs-oracle.md`.

    python tools/fx_vs_oracle.py --image FX/core1_sram.bin --out docs/research/data/fx-vs-oracle.csv

Needs the built `sharc-fxlive`, `fx_live_native`, `fx_live_install`
(`cargo build --release -p nord-sharc -p nord-tools`) and the local FX image
(`tools/ns4_dsp_ldr.py --sram`). Only derived numbers are written; no firmware or audio content.
"""

from __future__ import annotations

import argparse
import csv
import math
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fx_oracle_live as live  # noqa: E402

FRAME_RATE = live.FRAME_RATE
CHAIN_GAIN = 0.1866  # identity stage through the live chain (docs/research/fx-oracle-live.md)
BANDS = [(31, 63), (63, 125), (125, 250), (250, 500), (500, 1000), (1000, 2000), (2000, 4000),
         (4000, 8000), (8000, 16000)]  # fmt: skip
STIMULI = ("impulse", "sweep", "noise", "notes")
#: Mod 2 types (kFx2Type) and Mod 1 types (kFx1Type) with an install export, with their names.
MOD2 = {0: "PHAS0", 1: "PHAS1", 2: "FLANG0", 3: "FLANG1", 4: "CHOR0", 5: "CHOR1", 6: "VIBE0",
        7: "VIBE1", 8: "ENS0", 9: "ENS1", 10: "SPIN0", 11: "SPIN1"}  # fmt: skip
MOD1 = {
    0: "APAN0",
    1: "APAN1",
    2: "TREM0",
    3: "TREM1",
    4: "RM0",
    5: "RM1",
    6: "AWAH0",
    7: "AWAH1",
    8: "WAH0",
    9: "WAH1",
    10: "PUMP0",
    11: "PUMP1",
}
#: Parameter steps run on noise besides the default (rate, amount) = (64, 64) on every stimulus.
STEPS = ((0, 64), (127, 64), (64, 0), (64, 127))


def stimulus(kind: str, frames: int) -> tuple[np.ndarray, int]:
    """Stereo `(frames, 2)` deterministic stimulus and the frame its energy ends at."""
    t = np.arange(frames) / FRAME_RATE
    if kind == "sweep":  # log sweep 30 Hz .. 16 kHz in 0.5 s, then silence
        d = 0.5
        k = math.log(16000 / 30)
        ph = 2 * math.pi * 30 * d / k * (np.exp(np.minimum(t, d) / d * k) - 1)
        x = np.where(t < d, 0.25 * np.sin(ph), 0.0)
        return np.stack([x, 0.8 * x], 1).astype(np.float32), int(d * FRAME_RATE)
    if kind == "notes":  # held-note stream: 220 Hz at 0, 330 Hz at 0.2 s, both held, released 0.5 s
        x = np.zeros(frames)
        for f0, t0, g in ((220.0, 0.0, 0.12), (330.0, 0.2, 0.09)):
            on = np.clip((t - t0) / 0.005, 0, 1) * (t >= t0)
            rel = np.where(t < 0.5, 1.0, np.exp(-(t - 0.5) / 0.08))
            tone = sum(np.sin(2 * math.pi * f0 * h * t) / h for h in range(1, 7))
            x += g * on * rel * tone
        return np.stack([x, 0.9 * x], 1).astype(np.float32), int(0.5 * FRAME_RATE)
    sig = live.signal(kind, frames)
    return sig, int(0.25 * FRAME_RATE) if kind != "impulse" else 1


def band_gains(o: np.ndarray, m: np.ndarray, gain: float) -> list[float]:
    """Octave band level of the oracle against the native output (dB), chain gain removed."""
    so, sm = np.abs(np.fft.rfft(o)) ** 2, np.abs(np.fft.rfft(m)) ** 2
    f = np.fft.rfftfreq(len(o), 1 / FRAME_RATE)
    out = []
    for lo, hi in BANDS:
        sel = (f >= lo) & (f < hi)
        a, b = so[sel].sum(), sm[sel].sum()
        out.append(round(10 * math.log10(max(a, 1e-30) / max(b, 1e-30)) - 20 * math.log10(gain), 2))
    return out


def run(cmd: list[str]) -> str:
    return subprocess.run(cmd, capture_output=True, text=True, check=True).stdout


def metrics(oracle: np.ndarray, native: np.ndarray, end: int, gain: float) -> dict:
    n = min(len(oracle), len(native))
    o, m = oracle[:n, 0].astype(np.float64), native[:n, 0].astype(np.float64)
    lag = live.best_lag(o, m)
    x, y = (o[lag:], m[: n - lag]) if lag >= 0 else (o[: n + lag], m[-lag:])
    k = min(len(x), len(y))
    x, y = x[:k], y[:k]
    g = float(np.dot(x, y) / np.dot(y, y)) if np.dot(y, y) > 0 else 0.0
    ry = live.rms(y)
    return {
        "lag": lag,
        "rms_oracle_db": round(live.db(live.rms(x)), 2),
        "rms_native_db": round(live.db(ry), 2),
        "level_delta_db": round(20 * math.log10(max(g, 1e-9) / gain), 2),
        "null_fit_db": round(live.db(live.rms(x - g * y)) - live.db(ry), 1),
        "centroid_oracle_hz": round(live.centroid(x, FRAME_RATE)),
        "centroid_native_hz": round(live.centroid(y, FRAME_RATE)),
        "tail_oracle_ms": round(live.tail_ms(x, end, FRAME_RATE), 1),
        "tail_native_ms": round(live.tail_ms(y, end, FRAME_RATE), 1),
        "bands_db": " ".join(f"{b:.1f}" for b in band_gains(x, y, gain)),
    }


ROW_REF = 0xC1AC8  # core 1 row 0 of the multi-tap reverb module (docs/research/fx-reverb-leslie.md)
RVBM_PC = 0x120FDB  # wrapper of the module `0xC02BB44C` (same pc in both cores)
RVBM_LINE = (
    0x8004000,
    0x5000,
)  # delay line of the row (c[-15], c[-14]); the image holds data there


def rvbm_words(ty: int, amount: int) -> list[tuple[int, int]]:
    """Instance of the multi-tap reverb: zeroed line, type words (kind synth), amount words
    (`tools/fx_rvbmulti.py` layout on core 1 row 0, whose static row words are in the image)."""
    import fx_rvbmulti as rm

    ctx = ROW_REF + 0x20
    send, dry = rm.amount_words(amount)
    words = [(RVBM_LINE[0] + k, 0) for k in range(RVBM_LINE[1])]
    words += [(a, w) for a, w in sorted(_type_words(ty).items())]
    words += [(ctx - 31, send), (ctx - 30, dry), (ctx - 29, send)]
    return words + live.install(RVBM_PC, ctx)


def _type_words(ty: int) -> dict[int, int]:
    import fx_rvbmulti as rm

    out = {}
    with open(rm.DATA / "fx-type-words.csv", newline="") as f:
        for r in csv.DictReader(f):
            if (r["slot"], r["type"], r["kinds"]) == ("Rvb", str(ty), rm.KIND):
                out[int(r["dm_word"], 16)] = int(r["word"], 16)
    return out


def install_words(args: argparse.Namespace, slot: int, ty: int, rate: int, amount: int):
    out = run([args.install, str(slot), str(ty), str(rate), str(amount)]).splitlines()
    pc = int(out[0].split()[1], 16)
    words = [(int(a, 16), int(w, 16)) for a, w in (ln.split() for ln in out[1:])]
    return words + live.install(pc, live.CHOR_I1)


def one_case(args: argparse.Namespace, case: dict) -> dict:
    fam, slot, ty, rate, amount, stim = (
        case[k] for k in ("family", "slot", "ty", "rate", "amount", "stim")
    )
    out = Path(args.out_dir) / f"{fam}-r{rate}-a{amount}-{stim}"
    out.mkdir(parents=True, exist_ok=True)
    frames = int(args.seconds * FRAME_RATE) // 4 * 4
    lead = args.lead * 4
    sig, end = stimulus(stim, frames)
    sig = np.concatenate([np.zeros((lead, 2), np.float32), sig])
    live.write_wav(out / "in.wav", sig)
    if fam == "identity":
        words, nat = live.install("identity", live.REVERB_I1), ["off"]
    elif fam == "REVERB0":
        text = (live.REPO / "crates/nord-core/src/fx/fw_rvb_words.rs").read_text()
        words, nat = live.reverb0_words(text), ["reverb0"]
    elif fam == "COMP":
        words, nat = install_words(args, 3, 0, rate, amount), ["comp", "--amount", str(amount)]
    elif fam.startswith("RVBM"):
        words, nat = rvbm_words(ty, amount), ["rvbm", "--ty", str(ty), "--amount", str(amount)]
    else:
        words = install_words(args, slot, ty, rate, amount)
        nat = [f"mod{slot}", "--ty", str(ty), "--rate", str(rate), "--amount", str(amount)]
        if slot == 1:
            nat += ["--pass-frames", "4"]  # one live pass = 4 frames = 2 blocks per call
    (out / "words.script").write_text(live.script_text(words))
    run([args.sharc, args.image, "--core", "1", "--passes", str((lead + frames) // 4),
         "--in-wav", str(out / "in.wav"), "--script-flat", str(out / "words.script"),
         "--wav", str(out / "oracle.wav")])  # fmt: skip
    run([args.native, nat[0], str(out / "in.wav"), str(out / "native.wav"), *nat[1:]])
    res = metrics(
        live.read_wav(out / "oracle.wav")[lead:],
        live.read_wav(out / "native.wav")[lead:],
        end,
        CHAIN_GAIN,
    )
    row = {k: case[k] for k in ("family", "slot", "ty", "rate", "amount", "stim")} | res
    row["status"] = (
        "match" if res["null_fit_db"] <= -60 else f"differs (null {res['null_fit_db']} dB)"
    )
    return row


def cases(only: list[str] | None) -> list[dict]:
    fams = [("identity", 0, 0)]
    fams += [(f"M2-{n}", 2, t) for t, n in MOD2.items()] + [
        (f"M1-{n}", 1, t) for t, n in MOD1.items()
    ]
    fams += [("COMP", 3, 0)]
    fams += [("REVERB0", 0, 0)] + [(f"RVBM-{t}", 0, t) for t in (2, 3, 4, 5, 7, 8, 9, 10, 11)]
    out = []
    for fam, slot, ty in fams:
        if only and fam not in only:
            continue
        amt = 127 if fam.startswith("RVBM") else 64
        out += [dict(family=fam, slot=slot, ty=ty, rate=64, amount=amt, stim=s) for s in STIMULI]
        if fam.startswith("RVBM"):  # amount steps (wet level), rate has no meaning
            out += [
                dict(family=fam, slot=slot, ty=ty, rate=64, amount=a, stim="noise")
                for a in (31, 127)
            ]
        elif fam == "COMP":
            out += [
                dict(family=fam, slot=slot, ty=ty, rate=64, amount=a, stim=s)
                for a in (32, 127)
                for s in ("noise", "notes")
            ]
        elif fam not in ("identity", "REVERB0"):
            out += [
                dict(family=fam, slot=slot, ty=ty, rate=r, amount=a, stim="noise") for r, a in STEPS
            ]
    return out


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument(
        "--image", required=True, help="FX core1_sram.bin (tools/ns4_dsp_ldr.py --sram)"
    )
    ap.add_argument("--out", default="fx-vs-oracle.csv")
    ap.add_argument("--out-dir", default=tempfile.mkdtemp(prefix="fx_vs_"))
    ap.add_argument("--only", nargs="*", help="family names, e.g. M2-CHOR0 REVERB0")
    ap.add_argument("--seconds", type=float, default=1.0)
    ap.add_argument("--lead", type=int, default=3000)
    ap.add_argument("--jobs", type=int, default=4)
    rel = live.REPO / "target/release"
    ap.add_argument("--sharc", default=str(rel / "sharc-fxlive"))
    ap.add_argument("--native", default=str(rel / "fx_live_native"))
    ap.add_argument("--install", default=str(rel / "fx_live_install"))
    a = ap.parse_args(argv)
    a.sharc, a.native, a.install = (str(Path(p).resolve()) for p in (a.sharc, a.native, a.install))
    todo = cases(a.only)
    with ThreadPoolExecutor(a.jobs) as ex:
        rows = list(ex.map(lambda c: one_case(a, c), todo))
    cols = list(rows[0])
    with open(a.out, "w", newline="") as f:
        w = csv.DictWriter(f, cols)
        w.writeheader()
        w.writerows(rows)
    for r in rows:
        print(
            r["family"],
            r["rate"],
            r["amount"],
            r["stim"],
            r["lag"],
            r["level_delta_db"],
            r["null_fit_db"],
            r["status"],
        )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
