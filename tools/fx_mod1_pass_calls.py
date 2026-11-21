#!/usr/bin/env python3
"""Mod 1 calls per DSP pass in the FX chip (OQ38, D186, `docs/research/fx-mod1-pass-calls.md`).

    python tools/fx_mod1_pass_calls.py stage-writes target/bulk/fx_core1.script CAPTURE.txt.gz
    python tools/fx_mod1_pass_calls.py dispatch FX/core1_sram.bin target/bulk/fx_core1.script \\
        --capture CAPTURE.txt.gz
    python tools/fx_mod1_pass_calls.py install FX/core1_sram.bin 4 -16

`stage-writes` counts the ARM -> FX core 1 messages (boot script plus an `arm_dsp_capture.py`
file of one program) that land in the stage records of the dispatcher (head record, 10 group
headers, 35 rows: DM `0xB62C0 .. 0xB6520`). `dispatch` runs the FX core 1 pass (`sharc-fxlive`) on
the full ARM state of that program and counts, with `--calls-at`, how often one pass reaches the
null stub and each Mod 1 routine. `install` puts one Mod 1 routine in stage 0 (the
`fx_live_install` words, the harness of `fx_mod1_call_count.py probe`) with block count 2 and 1 and
prints calls per pass and the frame positions of the 4-frame pass that the call changes. Needs the
built `sharc-fxlive` (and `fx_live_install` for `install`) and the local FX image; only numbers
are printed.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fx_mod1_call_count as cc  # noqa: E402
import fx_oracle_live as live  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
#: VERIFIED: head record `0xB62CE..` (`fx_oracle_live.REC`), 10 group headers `0xB63EC..0xB6506`,
#: 35 rows `0xB63EE + 8 k`: the DM window of the dispatcher's stage records (fx-chain-install s3).
STAGE_LO, STAGE_HI = 0xB62C0, 0xB6520
#: VERIFIED: null stub of FX core 1 (every stage of the image).
NULL_STUB = 0x125346
#: VERIFIED: core 1 pc = core 2 pc + 0x974 for the library routines (fx-chain-install s2, runs 1,
#: 11, 22, 23, 32, 37); the core 2 pcs are the ones of fx-mod1-call-count s5b.
MOD1_PCS = {
    "A-PAN": 0x123707,
    "TREM": 0x1235AD,
    "RM": 0x124344,
    "A-WAH": 0x12480F,
    "WAH": 0x124746,
    "PUMP": 0x1236A0,
}
CALLS_RE = re.compile(r"calls of (0x[0-9a-fA-F]+) in one pass: (\d+)")


def parse_calls(text: str) -> int:
    """The count of the `--calls-at` line of `sharc-fxlive`."""
    m = CALLS_RE.search(text)
    if not m:
        raise ValueError(f"no 'calls of PC in one pass' line in {text!r}")
    return int(m.group(2))


def in_stage_window(dm: int) -> bool:
    return STAGE_LO <= dm < STAGE_HI


def script_hits(lines: list[str]) -> int:
    """Lines `PASS ADDR VALUE` of a boot script whose address is in the stage window."""
    return sum(
        1 for ln in lines if len(ln.split()) >= 3 and in_stage_window(int(ln.split()[1], 16))
    )


def capture_hits(rows: list[dict[str, object]], core: int = 1) -> int:
    """Messages of a capture (`arm_dsp_capture.parse_row`) to FX `core` inside the stage window."""
    return sum(
        1
        for r in rows
        if r["chip"] == "FX" and r["core"] == core and in_stage_window(int(r["dm"]))  # type: ignore[call-overload]
    )


def merged_script(boot_lines: list[str], rows: list[dict[str, object]], core: int = 1) -> str:
    """Boot script followed by the FX core `core` messages of a capture, all before pass 0."""
    extra = [f"0 {int(r['dm']):#x} {int(r['w2']):#010x}" for r in rows  # type: ignore[call-overload]
             if r["chip"] == "FX" and r["core"] == core]  # fmt: skip
    return "\n".join([*boot_lines, *extra]) + "\n"


def frames_processed(deviation: list[float]) -> int:
    """Frame positions of the pass whose output deviates from the pass-through."""
    return sum(1 for d in deviation if d > 1e-3)


def fxlive(binary: str, image: str, script: Path, *extra: str, passes: int = 3200) -> str:
    cmd = [binary, image, "--core", "1", "--passes", str(passes), "--lead", "3000"]
    cmd += ["--script-flat", str(script), *extra]
    return subprocess.run(cmd, capture_output=True, text=True, check=True).stdout


def cmd_stage_writes(args: argparse.Namespace) -> int:
    import arm_dsp_capture as cap

    boot = Path(args.boot).read_text().splitlines()
    rows = list(cap.read_rows(Path(args.capture)))
    print(f"boot script lines {len(boot)}, stage-window writes {script_hits(boot)}")
    fx1 = sum(1 for r in rows if r["chip"] == "FX" and r["core"] == 1)
    print(f"capture messages to FX core 1 {fx1}, stage-window writes {capture_hits(rows)}")
    return 0


def cmd_dispatch(args: argparse.Namespace) -> int:
    import arm_dsp_capture as cap

    boot = Path(args.boot).read_text().splitlines()
    rows = list(cap.read_rows(Path(args.capture))) if args.capture else []
    work = Path(tempfile.mkdtemp(prefix="m1pc_"))
    script = work / "state.script"
    script.write_text(merged_script(boot, rows))
    binary = str(Path(args.bin_dir) / "sharc-fxlive")
    print("pc        routine   calls in one pass")
    for name, pc in {"null stub": NULL_STUB, **MOD1_PCS}.items():
        n = parse_calls(fxlive(binary, args.image, script, "--calls-at", f"{pc:#x}"))
        print(f"{pc:#08x}  {name:9} {n}")
    return 0


def cmd_install(args: argparse.Namespace) -> int:
    ty, off = int(args.type), int(args.offset)
    rel = Path(args.bin_dir)
    lines = subprocess.run([str(rel / "fx_live_install"), "1", str(ty), "64", "64"],
                           capture_output=True, text=True,
                           check=True).stdout.splitlines()  # fmt: skip
    pc = int(lines[0].split()[1], 16)
    words = [(int(a, 16), int(w, 16)) for a, w in (ln.split() for ln in lines[1:])]
    words += live.install(pc, live.CHOR_I1)
    work = Path(tempfile.mkdtemp(prefix="m1pc_"))
    lead, frames = 4 * 3000, 4 * 6000
    x = (0.2 * np.random.default_rng(3).standard_normal((lead + frames, 2))).astype(np.float32)
    x[:lead] = 0
    live.write_wav(work / "in.wav", x)
    binary = str(rel / "sharc-fxlive")
    print("block count  calls of the routine per pass  frame positions processed (of 4)")
    for count in (2, 1):
        w = [(a, count if a == live.CHOR_I1 + off else v) for a, v in words]
        script = work / "w.script"
        script.write_text(live.script_text(w))
        base = [binary, args.image, "--core", "1", "--passes", str((lead + frames) // 4)]
        out = subprocess.run([*base, "--in-wav", str(work / "in.wav"), "--script-flat", str(script),
                              "--wav", str(work / "o.wav"), "--calls-at", f"{pc:#x}"],
                             capture_output=True, text=True, check=True).stdout  # fmt: skip
        y = live.read_wav(work / "o.wav")[lead:, 0]
        dev = cc.deviation_by_position(y, x[lead:, 0])
        print(f"{count:11}  {parse_calls(out):30}  {frames_processed(dev)}")
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("stage-writes")
    s.add_argument("boot")
    s.add_argument("capture")
    s.set_defaults(fn=cmd_stage_writes)
    for name, fn in (("dispatch", cmd_dispatch), ("install", cmd_install)):
        p = sub.add_parser(name)
        p.add_argument("image")
        if name == "dispatch":
            p.add_argument("boot")
            p.add_argument("--capture")
        else:
            p.add_argument("type", help="kFx1Type 0..11")
            p.add_argument("offset", help="block-count word offset from I1 (RM -16, ...)")
        p.add_argument("--bin-dir", default=str(REPO / "target" / "release"))
        p.set_defaults(fn=fn)
    args = ap.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
