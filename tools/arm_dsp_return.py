#!/usr/bin/env python3
"""DSP to ARM return channel of the harness (D147): what the ARM reads back from the DSP chips.

The firmware reads DSP memory through SPI: a request of `(chip, DSP address, word count)` that the
DSP answers from its own data memory. Three read routines exist, all listed with their evidence in
`docs/research/dsp-to-arm-return.md`:

* `0xC00119A4(r0 SPI object, r1 chip, r2 address, r3 destination)`: 4 words, the 3 peeks of the
  note-on table refill `0xC01B1840`;
* `0xC00117E8(same)`: 2 words, the status read of the wait `0xC01D6970`;
* `0xC0066E64(r0 comm object, r1 device, r2 address, r3 count, [sp] destination)`: the status read
  of the voice update `0xC01E0728`.

Without this module the harness serves SPI1 as plain RAM (a test pattern) and `arm_dsp_capture`
sets the status bit by hook (`DspAck`). `install` replaces the three routines by answers from a
`ReturnSource`: the SY voice masks, then the live chip (`sharc-live`, `Link.dump`), then the words
the ARM itself sent (the DM shadow), then the initial DSP image, then 0. Opt-in: nothing changes
unless `install` is called.

Pure parts (no firmware, no unicorn) are unit tested in `tools/tests/test_arm_dsp_return.py`.
"""

from __future__ import annotations

import struct
from collections import Counter
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path

# VERIFIED (disassembly, `docs/research/dsp-to-arm-return.md` s2): entry points and argument order.
SPI_READ4, SPI_READ2, SPI_READN = 0xC00119A4, 0xC00117E8, 0xC0066E64
# VERIFIED (`arm_emu.COMM_TABLE`): comm object of chip `c` is the word at `COMM_TABLE + 4 + 4 c`.
COMM_TABLE = 0xC053F8A4
CHIPS = ("FX", "PNO", "ORG", "SY", "SMP")
SY_CHIP = 3
# ASSUMED cap: the decoded sites read 1 to 4 words; larger counts are not answered.
READN_MAX_COUNT = 4
# VERIFIED (oracle `sharc-ref` run of `sy_isr_refill.scn` with `dump`, and the ARM wait at
# `0xC01E0694`): the SY keeps a 64 bit mask of idle voices at DM(0x92196), a mask of voices in the
# release state at DM(0x9219A); voice `k` is bit `k`, the ARM tests bit 0 of word 0 for voice 0.
IDLE_MASK, RELEASE_MASK, VOICES = 0x92196, 0x9219A, 56
# VERIFIED (`sy-smp-audio.md` s3): the ARM sends the Stage A state of voice `k` as path 1 message
# `(0x92414 + 0x38 k, state)`. VERIFIED on the oracle: reset `0x1C0794`, prepare `0x1C07CD` and go
# `0x1C084C`: `sharc-ref target/oracle/sy.bin tools/oracle_scenarios/sy_voice_mask.scn` dumps
# `DM(0x92196..7)` = `ffffffff 00ffffff` (reset), `fffffffe 00ffffff` (prepare), `fffffffe 00ffffff`
# (go), `ffffffff 00ffffff` (reset). The idle state `0x1C07B0` is ASSUMED to set the bit as reset
# does (not dumped separately; same ARM record initialiser).
# VERIFIED (listings `0xC01E0694..0xC01E073C`, `0xC01E26A8..0xC01E270C`): two voice update waits
# test
# `cache[[d + 4]] & [d + 8]` when the status read returns to the address below, with the descriptor
# `d` in the register given; the destination of the read is the cache base.
WAIT_RETURN = 0xC01E072C  # first wait (idle mask, descriptor in r5)
WAIT_SITES = {WAIT_RETURN: 5, 0xC01E26FC: 4}  # second wait: the ready mask, descriptor in r4
STATE_BASE, STATE_STRIDE = 0x92414, 0x38
STATE_IDLE, STATE_RESET, STATE_PREPARE, STATE_GO = 0x1C07B0, 0x1C0794, 0x1C07CD, 0x1C084C
IDLE_STATES, BUSY_STATES = (STATE_IDLE, STATE_RESET), (STATE_PREPARE, STATE_GO)


def resolve(chip: int, address: int) -> tuple[int, int]:
    """`(core, DM word)` of a DSP address as the ARM sends it.

    PNO and ORG are addressed through the `0x28000000` alias: byte offset `4 w`, bit 23 selects
    core 2 (VERIFIED: `0x28B07770` and `0x28307770` are the same word `0xC1DDC` of the two cores,
    `docs/research/dsp-rx-protocol.md` s2). SY and SMP use plain DM words (core 0).
    """
    if 0x28000000 <= address < 0x2C000000 and chip in (1, 2):
        return (2 if address & 0x800000 else 1), (address & 0x7FFFFF) >> 2
    return 0, address


def chip_of_comm(read32: Callable[[int], int], comm_object: int) -> int:
    """Chip index whose comm object is `comm_object` (`-1` when it is none of the five)."""
    for chip in range(len(CHIPS)):
        if read32(COMM_TABLE + 4 + 4 * chip) == comm_object:
            return chip
    return -1


def voice_of(word1: int) -> int | None:
    """Voice index of an SY state message address, else `None`."""
    offset = word1 - STATE_BASE
    if offset < 0 or offset % STATE_STRIDE or offset // STATE_STRIDE >= VOICES:
        return None
    return offset // STATE_STRIDE


@dataclass
class SyVoiceMask:
    """The SY idle voice mask, driven by the state messages the ARM sends.

    ASSUMED initial value: all 56 voices idle (the ARM record initialiser `0xC04FECFC` sends the
    idle state `0x1C07B0` to each voice, 56 literals; the oracle idle state ORs the voice bit into
    the mask, VERIFIED). The `DM(0x9219A)` release mask is not modelled (the transition to the
    release state is a DSP internal sample position test, not an ARM message): it reads 0.
    """

    idle: int = (1 << VOICES) - 1

    def observe(self, word1: int, value: int) -> None:
        k = voice_of(word1)
        if k is None:
            return
        if value in IDLE_STATES:
            self.idle |= 1 << k
        elif value in BUSY_STATES:
            self.idle &= ~(1 << k)

    def word(self, dm: int) -> int | None:
        """The 32 bit word of a mask address, `None` when `dm` is no mask word."""
        if IDLE_MASK <= dm < IDLE_MASK + 2:
            return (self.idle >> (32 * (dm - IDLE_MASK))) & 0xFFFFFFFF
        if RELEASE_MASK <= dm < RELEASE_MASK + 2:
            return 0
        return None


def image_word(image: bytes | None, dm: int) -> int:
    """Word `dm` of a flat SRAM image (`ns4_dsp_ldr.py --sram`: little endian, `4 dm` bytes)."""
    if image is None or not 0 <= 4 * dm <= len(image) - 4:
        return 0
    return struct.unpack_from("<I", image, 4 * dm)[0]


def load_images(directory: Path | None, chips: dict[int, str]) -> dict[tuple[int, int], bytes]:
    """`{(chip, core): flat image}` from `directory/<name>/core<N>_sram.bin` (missing: skipped)."""
    out: dict[tuple[int, int], bytes] = {}
    for chip, name in chips.items():
        for core in (1, 2):
            path = directory / name / f"core{core}_sram.bin" if directory else None
            if path and path.is_file():
                out[(chip, core)] = path.read_bytes()
    return out


@dataclass
class ReturnSource:
    """Layered DSP memory view for the reads of the ARM."""

    images: dict[tuple[int, int], bytes] = field(default_factory=dict)
    live: dict[tuple[int, int], Callable[[int, int], list[int]]] = field(default_factory=dict)
    sy: SyVoiceMask = field(default_factory=SyVoiceMask)
    shadow: dict[tuple[int, int, int], int] = field(default_factory=dict)
    reads: Counter = field(default_factory=Counter)  # (chip, core, dm) -> reads
    layer: Counter = field(default_factory=Counter)  # answer source -> words

    def observe(self, chip: int, core: int, path: int, word1: int, value: int) -> None:
        """One ARM to DSP message (`arm_emu.Message` fields): updates the shadow and the SY mask."""
        if chip == SY_CHIP and path == 1:
            self.sy.observe(word1, value)
        if path == 2:
            # keyed by the core of the address, as `read` resolves it (not by the message core)
            addr_core, dm = resolve(chip, word1)
            self.shadow[(chip, addr_core, dm)] = value & 0xFFFFFFFF
        elif path == 1:
            self.shadow[(chip, 0, word1)] = value & 0xFFFFFFFF

    def word(self, chip: int, core: int, dm: int) -> int:
        """One DM word, first layer that knows it."""
        self.reads[(chip, core, dm)] += 1
        if (chip, core) in self.live:  # a live chip (SY included, D153) answers before any model
            return self._hit("live", self.live[(chip, core)](dm, 1)[0])
        if chip == SY_CHIP and (w := self.sy.word(dm)) is not None:
            return self._hit("sy_mask", w)
        if (chip, core, dm) in self.shadow:
            return self._hit("shadow", self.shadow[(chip, core, dm)])
        if image_word(self.images.get((chip, core)), dm):
            return self._hit("image", image_word(self.images[(chip, core)], dm))
        return self._hit("zero", 0)

    def knows(self, chip: int, address: int) -> bool:
        """True when a layer other than the image or 0 answers `address` (SY mask, live, shadow)."""
        core, dm = resolve(chip, address)
        return (
            (chip == SY_CHIP and self.sy.word(dm) is not None)
            or (chip, core) in self.live
            or (chip, core, dm) in self.shadow
        )

    def _hit(self, name: str, value: int) -> int:
        self.layer[name] += 1
        return value

    def read(self, chip: int, address: int, count: int) -> list[int]:
        core, dm = resolve(chip, address)
        return [self.word(chip, core, dm + k) for k in range(count)]


def poll_advance(dump: Callable, step: Callable[[], object], every: int) -> Callable:
    """`dump` that also runs one pass (`step`) per `every` reads of the same address: the DSP runs
    while the ARM polls a status word (ASSUMED: the real ARM polls against a running chip; the
    harness otherwise freezes the chip in the poll loop, D153). `every` 0 leaves `dump` as is."""
    if every <= 0:
        return dump
    count: dict[int, int] = {}

    def advance(address: int, n: int) -> list[int]:
        count[address] = count.get(address, 0) + 1
        if count[address] % every == 0:
            step()
        return dump(address, n)

    return advance


@dataclass
class ReadLog:
    """Reads the ARM issued through the three routines (numbers only)."""

    calls: list[tuple[int, int, int, int, int]] = field(
        default_factory=list
    )  # pc, chip, addr, n, tick
    polls: Counter = field(default_factory=Counter)  # (chip, address) -> reads
    tick: int = 0
    fallbacks: int = 0  # `0xC0066E64` calls left to the old stub
    # the voice update wait (D153): (cache word index, mask, address) -> [tests, tests that passed]
    waits: dict[tuple[int, int, int], list[int]] = field(default_factory=dict)
    wait_ticks: dict[int, list[int]] = field(default_factory=dict)  # address -> [first, last] tick
    callers: Counter = field(default_factory=Counter)  # (routine, return address, address) -> calls


def make_stub(source: ReturnSource, log: ReadLog, kind: int) -> Callable:  # noqa: ANN201
    """Stub for one routine; `kind` is `SPI_READ4`, `SPI_READ2` or `SPI_READN`."""

    def stub(emu) -> int:  # noqa: ANN001
        if kind == SPI_READN:
            chip = chip_of_comm(emu.read32, emu.reg(0))
            address, count, dst = emu.reg(2), emu.reg(3), emu.read32(emu.reg(13))
            # only the voice update site is decoded: any other chip, address or count keeps the
            # earlier harness stub (return 0, no write)
            if chip < 0 or not 0 < count <= READN_MAX_COUNT or not source.knows(chip, address):
                log.fallbacks += 1
                return 0
        else:
            chip, address = emu.reg(1), emu.reg(2)
            count, dst = (4 if kind == SPI_READ4 else 2), emu.reg(3)
        key = (chip, address)
        log.polls[key] += 1
        words = source.read(chip, address, count)
        log.calls.append((kind, chip, address, count, log.tick))
        log.callers[(kind, emu.reg(14), address)] += 1
        for k, w in enumerate(words):
            emu.write32(dst + 4 * k, w)
        if kind == SPI_READN and emu.reg(14) in WAIT_SITES:
            desc = emu.reg(WAIT_SITES[emu.reg(14)])
            index, mask = emu.read32(desc + 4), emu.read32(desc + 8)
            row = log.waits.setdefault((index, mask, address), [0, 0])
            row[0] += 1
            row[1] += bool(emu.read32(dst + 4 * index) & mask)
            span = log.wait_ticks.setdefault(address, [log.tick, log.tick])
            span[1] = log.tick
        return 0

    return stub


def install(emu, source: ReturnSource, log: ReadLog) -> list[tuple]:  # noqa: ANN001  # pragma: no cover
    """Replace the three SPI read routines of the booted firmware; returns the undo records.

    The harness boot already stubs `0xC0066E64` (returns 0, `arm_bulk_tables`); that stub entry is
    swapped for the return stub and put back by `uninstall`. The other two get a code hook that
    returns to the caller (`uninstall` deletes it, so the original routine runs again).
    """
    undo: list[tuple] = []
    for pc in (SPI_READ4, SPI_READ2, SPI_READN):
        fn = make_stub(source, log, pc)
        if pc in emu.stubs:
            undo.append((pc, emu.stubs[pc], None))
            emu.stubs[pc] = fn
            continue

        def hook(_uc, _address, _size, _user, fn=fn) -> None:  # noqa: ANN001
            emu.set_reg(0, fn(emu))
            emu.set_reg(15, emu.reg(14) & ~1)

        undo.append((pc, None, emu.uc.hook_add(emu._u.UC_HOOK_CODE, hook, begin=pc, end=pc)))
    return undo


def uninstall(emu, undo: list[tuple]) -> None:  # noqa: ANN001  # pragma: no cover
    """Undo `install`: restore the earlier stub or delete the added hook."""
    for pc, old, handle in undo:
        if old is not None:
            emu.stubs[pc] = old
        else:
            emu.uc.hook_del(handle)
