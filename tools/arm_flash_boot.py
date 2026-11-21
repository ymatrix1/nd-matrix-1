#!/usr/bin/env python3
"""Boot the ARM firmware with the flash model of `arm_flash_model` answering SY / SMP flash reads
(D176, docs/research/sy-flash-bus-hunt.md).

The harness stub of `0xC0066E64` returns 0 for every read. Here a read of the AMI window
(`0x0400_0000..0x0800_0000`) of the SY (comm 3) or SMP (comm 4) first feeds the new port messages
of that chip to its `FlashChip`, then writes the model's words to the destination buffer.

    uv run --with unicorn --with capstone python tools/arm_flash_boot.py \
        target/fw_unpacked/bank0_CODE.bin target/fw_unpacked/bank0_SRAM.bin
"""

from __future__ import annotations

import argparse
import sys
from collections import Counter
from collections.abc import Iterable
from pathlib import Path

import arm_flash_model as fm

SPI_READN = 0xC0066E64
CHIP_COMMS = {3: "SY", 4: "SMP"}


def in_window(address: int) -> bool:
    """True for a DSP address in the AMI bank 1 window the flash answers."""
    return fm.BANK1 <= address < fm.BANK1 + fm.BANK_SIZE


def feed(chips: dict[int, fm.FlashChip], messages: Iterable, start: int) -> int:
    """Give `messages[start:]` to the chip each is addressed to; returns the new `start`."""
    msgs = list(messages)
    for m in msgs[start:]:
        if m.comm in chips:
            chips[m.comm].message(m.word1, m.word2)
    return len(msgs)


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the firmware
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("code", type=Path)
    ap.add_argument("sram", type=Path)
    a = ap.parse_args(argv)
    import arm_boot_full as bf
    import arm_bulk_tables as bt
    import arm_dsp_return as rt
    import arm_steady as ast

    emu = bf.prepare(a.code, a.sram)
    bt.real_port_capture(emu)  # the boot stores go to the real EMIFA ports, log them as messages
    chips = {c: fm.FlashChip(n) for c, n in CHIP_COMMS.items()}
    seen = [0]
    reads: Counter = Counter()
    inner = emu.stubs[SPI_READN]

    def spi(e) -> int | None:  # noqa: ANN001
        chip = rt.chip_of_comm(e.read32, e.reg(0))
        address, count = e.reg(2), e.reg(3)
        if chip not in chips or not in_window(address) or not 0 < count <= 64:
            return inner(e)
        seen[0] = feed(chips, e.messages, seen[0])
        dst = e.read32(e.reg(13))
        for k in range(count):
            e.write32(dst + 4 * k, chips[chip].read(address + k))
        reads[(chips[chip].name, hex(address), count, chips[chip].mode)] += 1
        return 0

    emu.stubs[SPI_READN] = spi
    pc, n, err = ast.boot_to_main_loop(emu)
    print(f"boot stopped at 0x{pc:08X} after {n} instructions {err or ''}")
    for k, v in sorted(reads.items()):
        print("flash read", k, v)
    for c in chips.values():
        print(c.name, "mode", c.mode, "commands", len(c.log), "programmed", len(c.array))
    return 0


if __name__ == "__main__":
    sys.exit(main())
