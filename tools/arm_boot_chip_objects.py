#!/usr/bin/env python3
"""Report the per-chip record list counts of the OS boot function `0xC0247B64` (D171).

Hooks `0xC0248744` (after the 7 byte record loop) in the booted ARM harness and prints, per chip
object, the chip id, the descriptor pointer and the counts of the 7 byte and the 9 byte lists.
VERIFIED run result (docs/research/sy-flash-bus-hunt.md s4): both counts are 0 for SY (chip 3) and
SMP (chip 4) in the harness boot.

    uv run --with unicorn --with capstone python tools/arm_boot_chip_objects.py \
        target/fw_unpacked/bank0_CODE.bin target/fw_unpacked/bank0_SRAM.bin
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

# VERIFIED (listing): `0xC0248744` follows the 7 byte loop; r5 = chip object, r11 = 7 byte list,
# `[sp + 0x20]` = 9 byte list, count at `+8` in both.
HOOK_PC = 0xC024_8744
CHIPS = {3: "SY", 4: "SMP"}


def format_row(chip: int, descriptor: int, count7: int, count9: int) -> str:
    """One report line for a chip object."""
    name = CHIPS.get(chip, f"chip{chip}")
    return (
        f"{name}: descriptor 0x{descriptor:08X}, 7 byte records {count7}, 9 byte records {count9}"
    )


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the firmware
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("code", type=Path)
    ap.add_argument("sram", type=Path)
    a = ap.parse_args(argv)
    import arm_boot_full as bf
    import arm_steady as ast

    emu = bf.prepare(a.code, a.sram)

    def rd(addr: int) -> int:
        return emu.read32(addr) if addr else 0

    def on_hook(_uc, _addr, _size, _user) -> None:  # noqa: ANN001
        r5, fp, sp = emu.reg(5), emu.reg(11), emu.reg(13)
        print(format_row(rd(r5), rd(r5 + 4), rd(fp + 8), rd(rd(sp + 0x20) + 8)))

    emu.uc.hook_add(emu._u.UC_HOOK_CODE, on_hook, begin=HOOK_PC, end=HOOK_PC)
    pc, _n, err = ast.boot_to_main_loop(emu)
    print(f"boot stopped at 0x{pc:08X} {err or ''}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
