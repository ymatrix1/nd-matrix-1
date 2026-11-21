#!/usr/bin/env python3
"""ARM words of the synth LFO destination and depth (`kLFODst`, `kLFOAmt`) (D207).

`arm_edit_knobs.py` applies every edit id from the settled state, so `kLFOAmt` never saw the
destination on (`synth-super-lp24-clips.md` s8: common words 2, 3, 8 stay 0). This tool applies
`kLFODst` first and `kLFOAmt` after it (no restore in between) and records the last word each DSP
address received, for the layer A client (index 4).

    uv run --with unicorn --with capstone python tools/synth_lfo_words.py \
        target/fw_unpacked/bank0_CODE.bin target/fw_unpacked/bank0_SRAM.bin \
        --cache target/settled.snap --out docs/research/data/synth-lfo-arm-words.csv

Result: destination 3 (cutoff) writes common word 3 (`DM(0xE02EB)`) = `amt / 127 * 0x40000000`;
destination 1 (OSC CTRL) writes common word 8 (float `amt / 254`), destination 2 (pitch) common
word 2 (`DM(0xE02EA)`, table), the gate `DM(0xC13A9)` is 1.0 for every destination but 2.
"""

from __future__ import annotations

import argparse
import csv
import sys
from pathlib import Path

#: DSP words the destination and amount ids speak to (common 0 words 2, 3, 8 and the LFO gate).
WATCH = (0xE02EA, 0xE02EB, 0xE02F0, 0xC13A9)
AMOUNTS = (0, 32, 64, 127)
LAYER_A_CLIENT = 4
FIELDS = ("dst", "amt", "dm_word", "word")


def last_words(messages: list[tuple[int, int]], watch: tuple[int, ...] = WATCH) -> dict[int, int]:
    """Last word per watched DM address of `(dm_word, word)` pairs in message order."""
    out: dict[int, int] = {}
    for dm, word in messages:
        if dm in watch:
            out[dm] = word
    return out


def cutoff_depth_word(amt: int) -> int:
    """Common word 3 the ARM writes for `kLFOAmt = amt` with the destination on the cutoff."""
    return round(min(max(amt, 0), 127) / 127.0 * 0x40000000)


def rows_csv(rows: list[dict]) -> str:
    import io

    buf = io.StringIO()
    w = csv.DictWriter(buf, fieldnames=FIELDS, lineterminator="\n")
    w.writeheader()
    w.writerows(rows)
    return buf.getvalue()


def capture(code: Path, sram: Path, cache: Path | None) -> list[dict]:  # pragma: no cover
    import arm_edit_capture as cap
    import arm_emu as ae
    import arm_steady as ast

    emu, blob = cap.settled_emulator(code, sram, cache)
    if not any(b <= cap.SCRATCH <= e for b, e, _ in emu.uc.mem_regions()):
        emu.uc.mem_map(cap.SCRATCH, ae.PAGE)
    stores = cap.first_store_per_layer(cap.find_stores(emu))
    store = next(s for s in stores if s.layer == LAYER_A_CLIENT)
    params = {p.name: p for p in cap.load_params(Path("docs/research/arm-edit-params.csv"), "kLFO")}

    def apply(name: str, v: int) -> dict[int, int]:
        p = params[name]
        during, later, _ = cap.apply_value(
            emu, store.address, p.edit_id, cap.cell_word(p, v, "q16"), 20_000
        )
        return last_words([(m.dm_word, m.word2) for m in during + later])

    rows = []
    for dst in range(4):
        ast.restore(emu, blob)
        emu.install_dsp_ports()
        apply("Edt_Syn_kLFODst", dst)
        for amt in AMOUNTS:
            for dm, word in sorted(apply("Edt_Syn_kLFOAmt", amt).items()):
                rows.append(
                    {"dst": dst, "amt": amt, "dm_word": f"{dm:#x}", "word": f"{word:#010x}"}
                )
    return rows


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local firmware
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("image", type=Path)
    ap.add_argument("sram", type=Path)
    ap.add_argument("--cache", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    ns = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    ns.out.write_text(rows_csv(capture(ns.image, ns.sram, ns.cache)), encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
