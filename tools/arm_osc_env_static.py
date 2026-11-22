#!/usr/bin/env python3
"""Static Osc Env Amount sweep of the booted firmware, with the Env To Pitch bit forced (OQ43).

Recalls a `.ns4p` (optionally with the `Edt_Syn_kOscEnvDst` bit of synth layer A patched), then
dispatches `Edt_Syn_kOscEnvAmt` values through the layer client dispatch `0xC00CC4F0` (store, 0, id,
&value) and logs the last float32 the ARM sends to ORG core 2 common block word 12
(`DM(0xE02E8 + 32 L + 12)`), in amount steps (`word * 60`).

    uv run --with unicorn python tools/arm_osc_env_static.py target/fw_unpacked/bank0_CODE.bin \
        target/fw_unpacked/bank0_SRAM.bin --eeprom target/oled/eeprom.bin \
        --ns4b ".reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b" \
        --program "Program/Bank F/Time Warp Saw.ns4p" --dst 0 --csv out.csv

Findings: `docs/research/morph-fw-osc-env-dst.md`. Only derived numbers are printed.
"""

from __future__ import annotations

import argparse
import csv
import io
import struct
import sys
from pathlib import Path

# VERIFIED (run, `docs/research/morph-fw-osc-env-dst.md` s2): bit offset in the `.ns4p` file of the
# `osc_env_to_pitch` flag (`kOscEnvDst`) of synth layer A: `sound_base + 123` of
# `cbin/synth_preset.rs` with the decoder's `sound_base + 71` at file bit 4543 (Atk Dcy Rls
# `0, 126, 127` of Time Warp Saw, Cassette Pad AT and Favorite Video; none other has them).
DST_BIT = 4595
# VERIFIED (`arm_program_morph.WATCH_WORDS`, FM Harpies A and C): common block word 12 of layer
# `L` (0 = A, 2 = C); layer B writes `0xE01FE` (`morph_osc_env_report.DSP_WORDS`), unused here.
WORD_BASE, WORD_STRIDE, WORD_INDEX = 0xE02E8, 32, 12
FIELDS = ("program", "dst", "x", "dsp_word", "dsp_steps")
Q16 = 16


def patch_dst(program: bytes, dst: int) -> bytes:
    """Copy of the program file with the layer A `kOscEnvDst` bit set to `dst` (0 or 1)."""
    if dst not in (0, 1):
        raise ValueError(f"dst {dst} is not 0 or 1")
    out = bytearray(program)
    mask = 1 << (7 - DST_BIT % 8)
    out[DST_BIT // 8] = (out[DST_BIT // 8] & ~mask) | (mask if dst else 0)
    return bytes(out)


def dst_of(program: bytes) -> int:
    """The layer A `kOscEnvDst` bit of a program file."""
    return (program[DST_BIT // 8] >> (7 - DST_BIT % 8)) & 1


def dsp_address(layer: int) -> int:
    """ORG core 2 DM address of the osc env amount word of synth layer `layer` (0 = A)."""
    return WORD_BASE + WORD_STRIDE * layer + WORD_INDEX


def word_steps(word: int) -> float:
    """Amount steps (`float32 * 60`) of a 32-bit DSP word."""
    return struct.unpack("<f", struct.pack("<I", word))[0] * 60.0


def q16_word(steps: int) -> int:
    """Q16.16 cell word of an integer amount value."""
    return (steps << Q16) & 0xFFFFFFFF


def rows_csv(rows: list[tuple]) -> str:
    """CSV text of `FIELDS` rows."""
    buf = io.StringIO()
    writer = csv.writer(buf, lineterminator="\n")
    writer.writerow(FIELDS)
    writer.writerows(rows)
    return buf.getvalue()


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local firmware
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("image", type=Path)
    ap.add_argument("sram", type=Path)
    ap.add_argument("--eeprom", type=Path, required=True)
    ap.add_argument("--ns4b", type=Path, required=True)
    ap.add_argument("--program", required=True, help="archive member of the .ns4p")
    ap.add_argument("--dst", type=int, choices=(0, 1), help="force the Env To Pitch bit")
    ap.add_argument("--values", default="-60:60", help="LO:HI integer amount range")
    ap.add_argument("--settle", type=int, default=200, help="ticks after each dispatch")
    ap.add_argument("--csv", type=Path)
    ns = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import arm_edit_capture as aec
    import arm_keydown as kd
    import arm_program_load as pg
    import arm_program_morph as apm
    import arm_screens as sc
    import arm_steady as ast

    program = apm.read_member(ns.ns4b, ns.program)
    if ns.dst is not None:
        program = patch_dst(program, ns.dst)
    params = Path(__file__).resolve().parent.parent / "docs/research/arm-edit-params.csv"
    amt = {n: i for i, n in apm.id_names(params).items()}["Edt_Syn_kOscEnvAmt"]
    session = sc.Session(ns.image, ns.sram)
    emu = session.emu
    kd.install_board_present(emu)
    session.load_eeproms(
        pg.build_image(ns.eeprom.read_bytes(), program, Path(ns.program).stem[:23])
    )
    session.boot()
    session.run(300)
    emu.install_dsp_ports()
    kd.Rig(emu, kd.Uart2().install(emu)).tick(20)
    blob = ast.snapshot(emu)
    store = next(s for s in aec.find_stores(emu) if s.kind == "synth" and s.layer == 4)
    if not any(b <= aec.SCRATCH <= e for b, e, _ in emu.uc.mem_regions()):
        emu.uc.mem_map(aec.SCRATCH, 0x1000)
    lo, hi = (int(v) for v in ns.values.split(":"))
    target = dsp_address(0)
    rows: list[tuple] = []
    for x in range(lo, hi + 1):
        ast.restore(emu, blob)
        emu.install_dsp_ports()
        emu.clear_messages()
        cell = aec.value_cell(emu, store.address, amt)
        emu.write32(cell, q16_word(x))
        emu.write32(aec.SCRATCH, q16_word(x))
        ast.call_preserving(emu, aec.DISPATCH, store.address, 0, amt, aec.SCRATCH)
        ast.run_ticking(emu, ns.settle)
        words = [
            m.word2 for m in emu.messages if m.chip == "ORG" and m.core == 2 and m.dm_word == target
        ]
        if words:
            rows.append(
                (
                    Path(ns.program).stem,
                    dst_of(program),
                    x,
                    f"{words[-1]:#010x}",
                    f"{word_steps(words[-1]):.4f}",
                )
            )
    text = rows_csv(rows)
    if ns.csv:
        ns.csv.write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
