#!/usr/bin/env python3
"""Farfisa processor `0x122B85` as a decoded firmware routine: oracle replay and Rust tables.

`docs/research/organ-farfisa-bass.md` s16. The processor listing (branches included: `if ne jump`,
`jump`, AZ / AN flags of the ALU) runs in `tools/sharc_listing_ext.py` and is compared with
`sharc-chain` (model 2 install, key-down of a chord), call after call. The generator `0x122A71`
is **not** run here: its ring (frame 0 at `DM 0xC2B52`, frame 1 at `0xC2BB4`) is the input, taken
from the oracle per call (`golden`) or produced by `FarfisaWheels` at the DSP rate in the stage.

    python tools/organ_farfisa_fw.py check  IMAGE SCRIPT --calls 48 --exe target/release/sharc-chain
    python tools/organ_farfisa_fw.py image  IMAGE SCRIPT OUT.csv      --exe ...   # install delta
    python tools/organ_farfisa_fw.py golden IMAGE SCRIPT OUT.csv --calls 24 --exe ...
    python tools/organ_farfisa_fw.py chords IMAGE --image-out OUT.csv --chord NAME:KEYS:SCRIPT ...

The DM image and the instruction rows are no longer tracked (D128): the Rust loader
`crates/nord-core/src/organ/farfisa_fw/` builds them at run time from the user's OS update file
(PNO core 1 loader data and the listing of `0x122B85`). `image` writes only what the loader cannot
read from the image: `docs/research/data/farfisa-fw-install-delta.csv` (the DM words the ARM
install changes against the loader data, and the start registers).
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np
import organ_bass_fw_gen as gen
import organ_listing_check as olc
import organ_replay as rp
import sharc_listing_ext as ext

GEN_SPEC = "0x122A71:0x122B84:I0=0xc00ea,I1=0xb52c7,M0=0,M1=1,M2=2,M7=0xffffffff"
PROC_SPEC = "0x122B85:0x122EF5:I0=0xc4000,I1=0xb474c,M0=0,M1=1,M2=2,M7=0xffffffff"
# VERIFIED (probe of the processor, s16): the DM ranges the routine reads or writes.
KEEP = ((0x9_5FFC, 0x178), (0x9_65DC, 0x10), (0xB_472C, 8), (0xC_27E0, 0x640), (0xC_4000, 8))
RATE = 43_402.6  # ASSUMED anchor of the DSP frame rate (s3)
SKIP = 1200
RING = (0xC_2B52, 0xC4)  # frame 0 at +0, frame 1 at +0x62
OUT = (0xC_4000, 4)  # PEx / PEy words of frame 0, then frame 1
DUMP = ("0x95000:0x2000", "0xB3000:0x2000", "0xC0000:0x500", "0xC2000:0x1000", "0xC4000:0x100")


def setup(a: argparse.Namespace) -> None:
    """Oracle arguments of the Farfisa chain (generator + processor, the ranges above)."""
    a.gen, a.proc, a.dump, a.window = GEN_SPEC, PROC_SPEC, list(DUMP), [f"{OUT[0]:#x}:4"]
    a.window.append(f"{RING[0]:#x}:{RING[1]:#x}")
    rp.set_ranges(a)


def kept(mem: dict[int, int]) -> list[tuple[int, int]]:
    """Non zero words of the kept ranges, sorted."""
    return sorted((k, v) for k, v in mem.items() if v and any(b <= k < b + n for b, n in KEEP))


def static_words(sram: bytes) -> dict[int, int]:
    """Non zero DM words of the kept ranges in a flat core 1 SRAM image (`4 * address`)."""
    out = {}
    for base, n in KEEP:
        for a in range(base, base + n):
            v = int.from_bytes(sram[4 * a : 4 * a + 4], "little")
            if v:
                out[a] = v
    return out


def install_delta(mem: dict[int, int], static: dict[int, int]) -> list[tuple[int, int]]:
    """`(address, value)` of the kept words whose oracle value differs from the loader data.

    A word the install clears appears with value 0.
    """
    now = dict(kept(mem))
    return sorted(
        (a, now.get(a, 0)) for a in set(now) | set(static) if now.get(a, 0) != static.get(a, 0)
    )


def start_machine(a: argparse.Namespace, mem: dict[int, int]) -> ext.GenVm:
    row0 = rp.trace_rows(a, *rp.parse_step(a.gen)[:2], (a.gen,))[0]
    machine = rp.start_vm(mem, row0)
    return machine


def cmd_check(a: argparse.Namespace) -> int:  # pragma: no cover - runs the oracle
    setup(a)
    p_entry, p_end, p_regs = rp.parse_step(a.proc)
    proc = olc.disassemble(a.image, p_entry, p_end)
    mem = rp.memory(a, ())
    machine = start_machine(a, mem)
    want, _ = rp.oracle_calls(a, a.calls)
    bad = 0
    for call in range(a.calls):
        for k in range(RING[1]):
            machine.wr(RING[0] + k, int(want[call][4 + k]))
        rp.run_step(machine, proc, p_regs, p_entry)
        got = [machine.rd(OUT[0] + k) for k in range(4)]
        if got != want[call][:4].tolist():
            bad += 1
            if bad < 4:
                print(f"call {call}: {[hex(g) for g in got]} != {[hex(w) for w in want[call][:4]]}")
    print(f"{a.calls} calls, {bad} calls with a differing output word")
    return 1 if bad else 0


def cmd_image(a: argparse.Namespace) -> int:  # pragma: no cover - runs the oracle
    setup(a)
    mem = rp.memory(a, ())
    regs = rp.vm.parse_trace_line(rp.trace_rows(a, *rp.parse_step(a.gen)[:2], (a.gen,))[0])[1]
    delta = install_delta(mem, static_words(Path(a.image).read_bytes()))
    lines = ["# Farfisa model 2 install words and start registers (D128), research s16"]
    for file in "rsim":
        lines += [f"R,{file}{k},{v:x}" for k, v in enumerate(regs[file]) if v]
    lines += gen.mem_runs(delta)
    Path(a.out).write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"{len(delta)} words, {len(lines)} lines -> {a.out}")
    return 0


def cmd_golden(a: argparse.Namespace) -> int:  # pragma: no cover - runs the oracle
    setup(a)
    want, _ = rp.oracle_calls(a, a.calls)
    lines = ["# call,out0,out1,ring words 0..195 (hex; oracle, ring = input of the processor)"]
    for c, row in enumerate(want.tolist()):
        lines.append(f"{c},{row[0]:x},{row[2]:x}," + " ".join(f"{w:x}" for w in row[4:]))
    Path(a.out).write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    return 0


def hann_lines(x: np.ndarray, count: int, skip: int, n: int = 8192) -> list[tuple[float, float]]:
    """`(Hz, amplitude)` of the `count` strongest peak bins of an `n` point Hann DFT of `x[skip:]`.

    The amplitude is `2 |X| / sum(w)` of the peak bin (the method of `bass-chord-goldens.csv`).
    """
    seg = x[skip : skip + n].astype(np.float64)
    win = np.hanning(n + 1)[:-1]  # periodic Hann, as the Rust `hann_line`
    spec = 2 * np.abs(np.fft.rfft(seg * win)) / win.sum()
    peaks = [k for k in range(2, len(spec) - 1) if spec[k] > spec[k - 1] and spec[k] >= spec[k + 1]]
    peaks.sort(key=lambda k: -spec[k])
    return [(k * RATE / n, float(spec[k])) for k in sorted(peaks[:count])]


def cmd_chords(a: argparse.Namespace) -> int:  # pragma: no cover - runs the oracle
    """Line goldens of the oracle chain (generator + processor) for `NAME:KEYS:SCRIPT` chords."""
    setup(a)
    a.window = [f"{OUT[0]:#x}:3"]
    rp.set_ranges(a)
    lines = ["chord,keys,hz,out_amp"]
    for spec in a.chord:
        name, keys, script = spec.split(":", 2)
        a.script = script
        calls = (SKIP + 8192) // 2 + 1
        want, _ = rp.oracle_calls(a, calls)
        f = np.ascontiguousarray(want, dtype="<u4").view(np.float32)
        out = np.stack([f[:, 0], f[:, 2]], 1).reshape(-1)
        lines += [
            f"{name},{keys.replace(',', ' ')},{hz:.2f},{amp:.6f}"
            for hz, amp in hann_lines(out, a.lines, SKIP)
        ]
    Path(a.image_out).write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("cmd", choices=("check", "image", "golden", "chords"))
    ap.add_argument("image")
    ap.add_argument("script", nargs="?")
    ap.add_argument("out", nargs="?")
    ap.add_argument("--chord", action="append", default=[], help="NAME:KEYS:SCRIPT (chords)")
    ap.add_argument("--lines", type=int, default=12)
    ap.add_argument("--image-out", dest="image_out")
    ap.add_argument("--calls", type=int, default=24)
    ap.add_argument("--exe", default="sharc-chain")
    a = ap.parse_args(argv)
    a.base = None
    cmds = {"check": cmd_check, "image": cmd_image, "golden": cmd_golden}
    return {**cmds, "chords": cmd_chords}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
