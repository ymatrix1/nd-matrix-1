#!/usr/bin/env python3
"""ARM capture of every DSP word a reverb edit writes, per effect unit, with the writer pc.

For each unit the harness restores the settled snapshot, then applies a script of edits
(`kRvbEnable`, `kRvbType`, `kRvbAmt`, optional `kRvbColor`) and records every DSP message of each
step. `fx_rvb_stage.py` reduces the rows to stage-record words (gain, level, step, mix).

    uv run --with unicorn python tools/fx_rvb_stage_capture.py CODE.bin SRAM.bin \
        --cache target/settled.snap --units 4,10,6 --types 8,10 --amts 0,38,127 --out cap.csv

Evidence and limits: `docs/research/reverb-stage-gain.md`.
"""

from __future__ import annotations

import argparse
import csv
import sys
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent))

FIELDS = "unit,step,type,amt,core,dm_word,word,pc,row,ctx".split(",")
#: ASSUMED: Q16.16 scale for continuous ids (range >= 12), raw for enums, as `fx_clients.cell_word`.
Q16_SHIFT = 16


def parse_ints(text: str) -> list[int]:
    """`"8,10,0x26"` to `[8, 10, 38]`."""
    return [int(x, 0) for x in text.split(",") if x.strip()]


def script(rvb_type: int, amts: list[int]) -> list[tuple[str, str, int]]:
    """Edit script: enable the reverb, set `rvb_type`, then each amount."""
    steps: list[tuple[str, str, int]] = [("enable", "Edt_Effects_kRvbEnable", 1)]
    steps += [(f"type{rvb_type}", "Edt_Effects_kRvbType", rvb_type)]
    steps += [(f"amt{a}", "Edt_Effects_kRvbAmt", a) for a in amts]
    return steps


def row(  # noqa: PLR0913
    unit: int,
    step: str,
    rtype: int,
    amt: int,
    m,
    where: tuple[int, int] | None = None,  # noqa: ANN001
) -> dict[str, object]:
    """One capture row from a DSP message `m` (fields `core`, `dm_word`, `word2`, `pc`);
    `where` is `(row word0, context offset)` of the state row that owns the word, if any."""
    base, ctx = where if where else (-1, 0)
    return {
        "unit": unit,
        "step": step,
        "type": rtype,
        "amt": amt,
        "core": m.core,
        "dm_word": f"{m.dm_word:#x}",
        "word": f"{m.word2:#010x}",
        "pc": f"{m.pc:#x}",
        "row": f"{base:#x}" if base >= 0 else "",
        "ctx": ctx if base >= 0 else "",
    }


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the firmware
    import arm_edit_capture as cap
    import arm_steady as ast
    import fx_clients as fc
    import fx_contexts as fxc

    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("image", type=Path)
    ap.add_argument("sram", type=Path)
    ap.add_argument("--cache", type=Path)
    ap.add_argument("--params", type=Path, default=Path("docs/research/arm-edit-params.csv"))
    ap.add_argument("--units", default="4")
    ap.add_argument("--types", default="8")
    ap.add_argument("--amts", default="38")
    ap.add_argument("--window", type=int, default=20_000_000)
    ap.add_argument("--out", type=Path, required=True)
    ns = ap.parse_args(argv)
    emu, blob = fc.fx_snapshot(ns.image, ns.sram, ns.cache)
    allp = cap.load_params(ns.params, ".")
    markers = {p.name: p.edit_id for p in allp if p.name in fc.KIND_MARKERS}
    units = {u.index: u for u in fc.find_units(emu, markers)}
    by = {p.name: p for p in cap.load_params(ns.params, fc.DEFAULT_FILTER)}
    table = fxc.blocks(emu.read32)
    rows: list[dict[str, object]] = []
    for ui in parse_ints(ns.units):
        for rtype in parse_ints(ns.types):
            ast.restore(emu, blob)
            emu.install_dsp_ports()
            cur_a = -1
            for step, name, value in script(rtype, parse_ints(ns.amts)):
                p = by[name]
                if name.endswith("kRvbAmt"):
                    cur_a = value
                cell = fc.cell_word(p, value)
                during, later, err = cap.apply_value(
                    emu, units[ui].address, p.edit_id, cell, ns.window
                )
                for m in during + later:
                    b = fxc.locate(table, m.core, m.dm_word)
                    where = (b.word0, m.dm_word - (b.word0 + 0x20)) if b else None
                    rows.append(row(ui, step, rtype, cur_a, m, where))
                print(ui, rtype, step, len(during + later), err, flush=True)
    with ns.out.open("w", encoding="utf-8", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=FIELDS)
        w.writeheader()
        w.writerows(rows)
    return 0


if __name__ == "__main__":
    sys.exit(main())
