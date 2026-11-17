#!/usr/bin/env python3
"""What the ARM writes to a PUMP instance under MST CLK (`docs/research/fx-mod-types.md` s7.2).

Applies `kFx1Type` 10, then `kFx1ClkEnable` 0 or 1 and `kFx1RateClkDiv` to unit 0 of the settled
firmware and reports the pointer word `c[-29]` and the multiplier word `c[-28]` of the PUMP row
(routine `0x122D2C`: `g = f(imul([c[-29]], c[-28]) + c[-18])`). Free run: the pointer is the
address of the phase word `c[-31]` and `N` is 1. MST CLK: the pointer is `0xC0C70` and `N` is the
rate step multiplier (`fx-pump-sync-n.csv`).

    python tools/fx_pump_clock_probe.py CODE SRAM --cache SNAP   # CODE, SRAM: bank0_*.bin

Needs unicorn and the firmware (not committed); prints addresses and numbers only.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

PHASE_TO_POINTER = 2  # `c[-29]` is two words above `c[-31]`


def pointer_and_n(free: dict[int, int], sync: dict[int, int]) -> dict[str, int] | None:
    """The PUMP pointer word and multiplier from the DM words of a free run and a MST CLK run.

    The pointer is the word of the free run whose value is the address of the phase word two
    below it (`value == dm - 2`) and that changes under MST CLK; the multiplier is the next word.
    """
    for dm, word in sorted(free.items()):
        if word != dm - PHASE_TO_POINTER or dm not in sync or sync[dm] == word:
            continue
        n_dm = dm + 1
        if n_dm not in sync:
            continue
        return {
            "pointer_dm": dm,
            "free_target": word,
            "sync_target": sync[dm],
            "n_dm": n_dm,
            "n": sync[n_dm],
            "free_n": free.get(n_dm, 1),
        }
    return None


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs unicorn and firmware
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import arm_edit_capture as cap
    import arm_steady as ast
    import fx_clients as fc

    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("image", type=Path)
    ap.add_argument("sram", type=Path)
    ap.add_argument("--cache", type=Path)
    ap.add_argument("--params", type=Path, default=Path("docs/research/arm-edit-params.csv"))
    ap.add_argument("--rate", type=int, default=73)
    ns = ap.parse_args(argv)
    emu, blob = fc.fx_snapshot(ns.image, ns.sram, ns.cache)
    params = {p.name: p for p in cap.load_params(ns.params, fc.DEFAULT_FILTER)}
    markers = {p.name: p.edit_id for p in params.values() if p.name in fc.KIND_MARKERS}
    unit = fc.find_units(emu, markers)[0]
    noise = cap.baseline_keys(emu)

    def apply(name: str, value: int) -> dict[int, int]:
        p = params[name]
        word = fc.cell_word(p, value)
        during, later, _ = cap.apply_value(emu, unit.address, p.edit_id, word, 400_000)
        return {
            m.dm_word: m.word2
            for m in during + later
            if m.core == 1 and (m.chip, m.core, m.path, m.dm_word) not in noise
        }

    runs = []
    for clk in (0, 1):
        ast.restore(emu, blob)
        emu.install_dsp_ports()
        words = apply("Edt_Effects_kFx1Type", 10)
        words |= apply("Edt_Effects_kFx1ClkEnable", clk)
        words |= apply("Edt_Effects_kFx1RateClkDiv", ns.rate)
        runs.append(words)
    found = pointer_and_n(*runs)
    if found is None:
        print("no PUMP pointer word found")
        return 1
    for key, value in found.items():
        print(f"{key} {value:#x}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
