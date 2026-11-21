#!/usr/bin/env python3
"""The firmware compressor row (SLOPE, REDUCTION_SCALE, TABLE1, LOG_GAIN) against the port and the
SHARC oracle (D187, `docs/research/fx-compressor-row.md`).

    python tools/fx_compressor_row.py row   FX/core1_sram.bin
    python tools/fx_compressor_row.py scan  target/bulk/boot_log.jsonl
    python tools/fx_compressor_row.py chain FX/core1_sram.bin target/bulk/fx_core1.script

`row`   prints the gain-law words of the five compressor rows of the FX image (core 1).
`scan`  lists every ARM write (all phases, both FX cores) into the compressor instances and says
        whether one of them touches the four gain-law words (it does not).
`chain` runs the oracle chain (`sharc-fxlive`, FX core 1, real pass) with the compressor stage on
        the image's own row I1 = 0xB0168 (image words, the ARM boot words of that row, the `kCmpAmt`
        words of the port) and `fx_live_native comp` with the ASSUMED and the firmware row on the
        same noise; prints lag, fit gain and null depth for each. Needs the built `sharc-fxlive` and
        `fx_live_native` (`cargo build --release -p nord-sharc -p nord-tools`) and the local files;
        only derived numbers are printed.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import tempfile
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fx_oracle_live as live  # noqa: E402
import fx_vs_oracle as vs  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
ROWS = (0xB0168, 0xB0180, 0xB0198, 0xB01B0, 0xB01C8)  # compressor I1 of FX core 1 / core 2
PC = 0x120630  # same pc on both cores
#: gain-law words: name -> (offset from I1, is_pair)
LAW = {"SLOPE": (-0x17, True), "REDUCTION_SCALE": (-0x15, False), "TABLE1": (-0x14, False),
       "LOG_GAIN": (-0x0B, False), "SMOOTH": (-0x09, False)}  # fmt: skip
#: what `FwCompressor::new` / `compressor_for_amount` assume (D187 replaces them behind a switch)
ASSUMED = {"SLOPE": (0.05, 0.05), "REDUCTION_SCALE": (0.5,), "TABLE1": (0.5,),
           "LOG_GAIN": (1.0,), "SMOOTH": (0.9908,)}  # fmt: skip


def f32(w: int) -> float:
    return struct.unpack("<f", struct.pack("<I", w))[0]


def image_word(image: bytes, addr: int) -> int:
    return struct.unpack("<I", image[4 * addr : 4 * addr + 4])[0]


def cmd_row(args: argparse.Namespace) -> int:
    img = Path(args.image).read_bytes()
    print("row I1     word        image value        assumed (port)")
    for i1 in ROWS:
        for name, (off, pair) in LAW.items():
            vals = [f32(image_word(img, i1 + off + k)) for k in range(2 if pair else 1)]
            print(f"{i1:#x} {name:16} {off:#5x} {vals}  {ASSUMED[name]}")
    same = len({tuple(image_word(img, i1 + o) for o in range(-0x20, -6)) for i1 in ROWS[:4]})
    print("rows 0..3 identical:", same == 1)
    return 0


def cmd_scan(args: argparse.Namespace) -> int:
    rows = [json.loads(ln) for ln in Path(args.log).read_text().splitlines()]
    fx = [r for r in rows if r["chip"] == "FX"]
    inst = {(r["core"], int(r["dm"], 16) + 0x20) for r in fx if r["w2"] == "0x3dd10ac8"}
    law = {off + k for off, pair in LAW.values() for k in range(2 if pair else 1)}
    seen, hits = {}, []
    for r in fx:
        a = int(r["dm"], 16)
        for core, i1 in inst:
            if r["core"] == core and i1 - 0x20 <= a <= i1 - 7:
                seen.setdefault((core, i1), set()).add(a - i1)
                if a - i1 in law:
                    hits.append((r["phase"], core, hex(i1), a - i1, r["w2"]))
    print(f"{len(inst)} compressor instances written by the ARM; offsets written per instance:")
    for k in sorted(seen):
        print(f"  core {k[0]} I1 {k[1]:#x}: {sorted(hex(o) for o in seen[k])}")
    print("ARM writes at the gain-law offsets:", hits if hits else "none")
    return 0 if not hits else 1


def amount_words(amount: int) -> dict[int, int]:
    """The port's kCmpAmt words (`fx_live_install 3 0`), as offsets from I1."""
    out = vs.run([str(args_install()), "3", "0", "64", str(amount)]).splitlines()
    i1 = int(out[0].split()[3], 16)
    return {int(a, 16) - i1: int(w, 16) for a, w in (ln.split() for ln in out[1:])}


def args_install() -> Path:
    return REPO / "target" / "release" / "fx_live_install"


def oracle_words(boot: Path, amount: int) -> list[tuple[int, int]]:
    i1 = ROWS[0]
    words: dict[int, int] = {}
    for ln in boot.read_text().splitlines():  # `pass addr value` (last value per word)
        p = ln.split()
        a = int(p[1], 16)
        if i1 - 0x20 <= a <= i1 - 7:
            words[a - i1] = int(p[2], 16)
    am = amount_words(amount)
    # the kCmpAmt words the ARM writes on a knob move: targets -0x1C, -0x1B and the settled state
    for off in (-0x1C, -0x1B, -0x1A, -0x19, -0x18):
        words[off] = am[off]
    return [(i1 + o, w) for o, w in sorted(words.items())] + live.install(PC, i1)


def cmd_chain(args: argparse.Namespace) -> int:
    rel = REPO / "target" / "release"
    work = Path(tempfile.mkdtemp(prefix="cmprow_"))
    frames = int(args.seconds * live.FRAME_RATE) // 4 * 4
    lead = 3000 * 4
    rng = np.random.default_rng(7)
    env = np.repeat(0.05 + 0.5 * (np.sin(np.arange(frames // 4000 + 1) * 0.9) ** 2), 4000)[:frames]
    sig = (env[:, None] * rng.standard_normal((frames, 2)) * 0.5).astype(np.float32)
    sig = np.concatenate([np.zeros((lead, 2), np.float32), np.clip(sig, -1, 1)])
    live.write_wav(work / "in.wav", sig)
    print(f"{'amount':>6} {'native row':>10} {'lag':>4} {'level dB':>9} {'null dB':>8}")
    for amount in args.amounts:
        (work / "w.script").write_text(live.script_text(oracle_words(Path(args.boot), amount)))
        vs.run([str(rel / "sharc-fxlive"), args.image, "--core", "1", "--passes",
                str((lead + frames) // 4), "--in-wav", str(work / "in.wav"),
                "--script-flat", str(work / "w.script"),
                "--wav", str(work / "oracle.wav")])  # fmt: skip
        ora = live.read_wav(work / "oracle.wav")[lead:]
        for row in ("assumed", "firmware"):
            vs.run([str(rel / "fx_live_native"), "comp", str(work / "in.wav"),
                    str(work / f"{row}.wav"), "--amount", str(amount),
                    "--comp-row", row])  # fmt: skip
            nat = live.read_wav(work / f"{row}.wav")[lead:]
            m = vs.metrics(ora, nat, frames, vs.CHAIN_GAIN)
            print(
                f"{amount:>6} {row:>10} {m['lag']:>4} "
                f"{m['level_delta_db']:>9} {m['null_fit_db']:>8}"
            )
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("row")
    r.add_argument("image")
    r.set_defaults(fn=cmd_row)
    s = sub.add_parser("scan")
    s.add_argument("log")
    s.set_defaults(fn=cmd_scan)
    c = sub.add_parser("chain")
    c.add_argument("image")
    c.add_argument("boot")
    c.add_argument("--seconds", type=float, default=1.0)
    c.add_argument("--amounts", type=int, nargs="*", default=[32, 64, 127])
    c.set_defaults(fn=cmd_chain)
    args = ap.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
