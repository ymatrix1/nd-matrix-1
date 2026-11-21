#!/usr/bin/env python3
"""Sample flash model behind the SY and SMP external bus (D176, docs/research/sy-flash-bus-hunt.md).

The ARM reaches the flash through the DSP: writes are port messages (bank 2 latch at
`0x0800_0000 + a`, bank 1 command or data at `0x0400_0000 + a`) and reads are SPI reads of the DSP
address space (`0xC0066E64`, address `0x0400_0000 + a`). The harness stub answers 0 and writes
nothing. This model answers the reads:

* command `0x98` enters CFI query mode (`Q R Y`, command set, size), `0xF0` / `0xFF` returns to
  array mode, `0x90` enters autoselect (manufacturer / device id), `0xA0` programs the next write;
* array reads return the programmed words or `0xFFFF` (erased).

Everything about the device is ASSUMED (standard JEDEC CFI layout, size, ids): the real chips and
their content are not available locally.

Boot run (`tools/arm_flash_boot.py`, EMPIRICAL): per chip the ARM sends `0xF0`, `0x90`, reads the
autoselect offsets 0, 1, 0xE and 0xF, sends `0x98` and reads five words at `0x4000040..44` (SY) or
`0x4000080..84` (SMP), i.e. CFI words `0x10..0x14` (`Q R Y` and the command set). The boot reaches
the main loop (`0xC00A0248`) with the model and with the stub that returns 0, so these answers
decide nothing in the boot itself; no array read and no program command follows.
"""

from __future__ import annotations

from dataclasses import dataclass, field

BANK1, BANK2 = 0x0400_0000, 0x0800_0000
BANK_SIZE = 0x0400_0000
ERASED = 0xFFFF
CMD_RESET, CMD_RESET2, CMD_QUERY, CMD_AUTOSELECT, CMD_PROGRAM = 0xF0, 0xFF, 0x98, 0x90, 0xA0
# EMPIRICAL (boot run): after `0x98` the ARM reads five words here, CFI words 0x10..0x14.
QUERY_BASE = {"SY": 0x40, "SMP": 0x80}
CFI_FIRST_WORD = 0x10
# ASSUMED: manufacturer / device id words of the autoselect read (no data sheet locally).
AUTOSELECT = {0: 0x0001, 1: 0x227E}


def cfi_word(index: int, size_log2: int) -> int:
    """Word `index` of a JEDEC CFI query table for an x16 AMD command set device.

    ASSUMED: layout from the JEDEC CFI specification, values typical for a 64 MiB part; nothing
    local confirms them."""
    table = {
        0x10: 0x51,  # 'Q'
        0x11: 0x52,  # 'R'
        0x12: 0x59,  # 'Y'
        0x13: 0x02,  # primary command set: AMD / Fujitsu
        0x15: 0x40,  # extended table address
        0x1B: 0x27,  # Vcc min 2.7 V
        0x1C: 0x36,  # Vcc max 3.6 V
        0x1F: 0x07,  # typical word write 2^7 us
        0x20: 0x07,
        0x21: 0x0A,  # typical block erase 2^10 ms
        0x27: size_log2,  # device size 2^n bytes
        0x28: 0x02,  # interface x8 / x16
        0x2A: 0x05,  # write buffer 2^5 bytes
        0x2C: 0x01,  # one erase region
        0x2D: 0xFF,  # blocks - 1 (low)
        0x2F: 0x00,  # block size / 256 (low)
        0x30: 0x01,
    }
    return table.get(index, 0)


@dataclass
class FlashChip:
    """One flash behind one DSP: command state, address latch and programmed words."""

    name: str
    size_log2: int = 26
    mode: str = "array"
    latch: int = 0
    array: dict[int, int] = field(default_factory=dict)
    log: list[tuple[str, int, int]] = field(default_factory=list)  # (kind, address, value)
    _program_next: bool = False

    def message(self, word1: int, value: int) -> None:
        """One port message to this chip (opcode 0 store, `word1` low 28 bits = address)."""
        a = word1 & 0x0FFF_FFFF
        if BANK2 <= a < BANK2 + BANK_SIZE:
            self.latch = value & 0xFFFF
            self.log.append(("latch", a - BANK2, value))
        elif BANK1 <= a < BANK1 + BANK_SIZE:
            self._command(a - BANK1, value & 0xFFFF)

    def _command(self, addr: int, value: int) -> None:
        self.log.append(("cmd", addr, value))
        if self._program_next:
            self.array[(self.latch << 24) | addr] = value
            self._program_next = False
        elif value in (CMD_RESET, CMD_RESET2):
            self.mode = "array"
        elif value == CMD_QUERY:
            self.mode = "query"
        elif value == CMD_AUTOSELECT:
            self.mode = "autoselect"
        elif value == CMD_PROGRAM:
            self._program_next = True

    def read(self, address: int) -> int:
        """Word read at DSP address `0x0400_0000 + a` (or the offset `a` alone)."""
        a = address & 0x00FF_FFFF
        if self.mode == "query":
            base = QUERY_BASE.get(self.name, 0)
            return cfi_word(CFI_FIRST_WORD + (a - base), self.size_log2)
        if self.mode == "autoselect":
            return AUTOSELECT.get(a & 0xFF, 0)
        return self.array.get((self.latch << 24) | a, ERASED)
