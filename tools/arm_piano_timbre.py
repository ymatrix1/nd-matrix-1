#!/usr/bin/env python3
"""ARM oracle for the piano timbre gain: play notes in the real firmware with a timbre mode forced.

The `kEq` handler stores the mode at `[[core + 0x390] + 0x5C]` and sends a sample reload trigger;
the
voice-frame builder `0xC00AA520` reads the mode of a **new** voice from `[[core + 0x390] + 0x58]`
(`docs/research/piano-timbre-gain.md` s3). The harness has no completed reload, so it writes `+0x58`
directly, sends a MIDI note-on over UART2 and records the gain word the ARM sends for the voice: the
first message to DM word `0x9252D` (float `amp / 2^23`; `0.25` = `0x3E800000` with no timbre).

    uv run --with unicorn --with capstone python tools/arm_piano_timbre.py capture \
        target/fw_unpacked/bank0_CODE.bin target/fw_unpacked/bank0_SRAM.bin --cache
        target/keydown.snap \
        --ns4b "<backup>.ns4b" --npno "Piano/Clavinet/Clavinet D6  6.1.npno" --out cases.json

Each case holds `mode`, `key`, `velocity`, `r8` (the internal velocity, argument 3 of
`0xC00AA520`) and
the `word`. `tools/piano_timbre_gain.py check` compares the model with them.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

GAIN_WORD1 = 0x9252D
FRAME_BUILDER = 0xC00AA520
KEQ_RETARGET = 0xC00C07D8
EDIT_KEQ = 0x15C
MODES = tuple(range(0x10, 0x1B))
KEYS = (24, 60, 96)
VELOCITIES = (30, 80, 127)
# ARM register ids of unicorn (`UC_ARM_REG_R0`, `R2`, `R3`, `SP`).
REG_R0, REG_R2, REG_R3, REG_SP = 66, 68, 69, 12


def gain_word(words: list[tuple[int, int]]) -> int | None:
    """Value of the first message to the voice gain word, or None when no voice started."""
    for word1, word2 in words:
        if word1 == GAIN_WORD1:
            return word2
    return None


def case_rows(mode: int, key: int, velocity: int, r8: int, word: int) -> dict:
    """One JSON case."""
    return {"mode": mode, "key": key, "velocity": velocity, "r8": r8, "word": word}


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - drives the firmware
    import arm_edit_capture as ec
    import arm_keydown as kd
    import arm_piano_load as pl
    import arm_steady as ast
    from unicorn import UC_HOOK_CODE

    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("command", choices=["capture"])
    ap.add_argument("image", type=Path)
    ap.add_argument("sram", type=Path)
    ap.add_argument("--cache", type=Path)
    ap.add_argument("--ns4b", type=Path, required=True)
    ap.add_argument("--npno", required=True)
    ap.add_argument("--out", type=Path, required=True)
    ns = ap.parse_args(argv)
    rig = kd.build_rig(ns.image, ns.sram, ns.cache)
    emu = rig.emu
    stub = pl.LibraryStub(
        {1: pl.device_image(pl.read_npno(ns.ns4b, ns.npno), "Library.npno")}, pl.KIND_FULL
    )
    stub.install(emu)
    pl.load_sound(emu, pl.managers(emu)[0], 1)
    pl.select_sound(emu, 1)
    pl.install_dsp_ready(emu)
    objs: set[int] = set()

    def on_retarget(uc, _a, _s, _u) -> None:  # noqa: ANN001
        objs.add(emu.read32(emu.read32(uc.reg_read(REG_R0) + 0xC) + 0x390))

    handle = emu.uc.hook_add(UC_HOOK_CODE, on_retarget, begin=KEQ_RETARGET, end=KEQ_RETARGET + 4)
    for store in (s for s in ec.find_stores(emu) if s.kind == "piano"):
        ec.apply_value(emu, store.address, EDIT_KEQ, 1, window=1000)
    emu.uc.hook_del(handle)
    rig.tick(2)
    blob = ast.snapshot(emu)
    frames: list[tuple[int, int]] = []

    def on_frame(uc, _a, _s, _u) -> None:  # noqa: ANN001
        if not frames:
            frames.append((uc.reg_read(REG_R2), uc.reg_read(REG_R3)))

    emu.uc.hook_add(UC_HOOK_CODE, on_frame, begin=FRAME_BUILDER, end=FRAME_BUILDER + 4)
    cases = []
    for mode in (0, *MODES):
        for key in KEYS:
            for velocity in VELOCITIES:
                ast.restore(emu, blob)
                frames.clear()
                for obj in objs:
                    emu.write32(obj + 0x58, mode)
                rig.send(kd.note_on(key, velocity))
                words = [(m.word1, m.word2) for _, m in rig.tick(30)]
                word = gain_word(words)
                if word is None or not frames:
                    continue
                cases.append(case_rows(mode, key, velocity, frames[0][1], word))
                print(cases[-1], flush=True)
    ns.out.write_text(json.dumps({"cases": cases}, indent=1) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
