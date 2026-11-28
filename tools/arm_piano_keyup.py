#!/usr/bin/env python3
"""Key-up and release placement of the firmware under the sustain pedal (ARM harness).

Plays one note through the real firmware (`arm_piano_load`: stubbed library, real loader, binder
and key path) and records, per phase, which SMP voice records the ARM writes: release writes
(`F16` decay word, hold = 1.0), voice starts (stroke fields, level `F15`, rate `F17`, initial
ramp from the voice block) and ramp writes. Times are in timer ticks of 41.67 us (24 kHz).

    uv run --with unicorn python tools/arm_piano_keyup.py target/fw_unpacked/bank0_CODE.bin \
        target/fw_unpacked/bank0_SRAM.bin --cache target/keydown.snap \
        --ns4b ".reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b" \
        --npno "Piano/Grand/Velvet Grand Med 6.1.npno" --scenario plain --scenario a

Scenarios (`SCENARIOS`): plain, a (key-up under the pedal, then lift), a2 (pedal pressed after
the strike), c (lift first, key-up after), d (re-strike under the pedal), half (CC64 = 64; `--cc`
replaces the value), again / again_pedal (a second note after a key-up: the harness limit).
Findings: `docs/research/piano-pedal-keyup.md`. Only derived numbers are printed (no file bytes).
"""

from __future__ import annotations

import argparse
import math
import struct
import sys
from collections.abc import Iterable, Sequence
from dataclasses import dataclass
from pathlib import Path

import arm_smp_slew_init as si

# VERIFIED (`tools/arm_steady.py` `TIMER_IRQ` note, D130): one timer interrupt is 24 kHz.
TICK_US = 1e6 / 24000.0
# VERIFIED (`docs/research/smp-slew.md` s6): one slew update per second interrupt at 43,402.34 Hz
# (the interrupt rate itself ASSUMED one per output frame).
UPDATE_HZ = 43402.34 / 2.0
# VERIFIED (run, `docs/research/smp-slew.md` s6, D68): the release write is `F14 = 1.0`,
# `F12 = F11 = 0` and the decay factor `F16` per update; `F16 = 1.0` leaves only the hold decay.
F_RELEASE = (14, 12, 11, 16)
# VERIFIED (run): a voice start writes the stroke descriptor words `F-11 .. F-4` and `F-18`.
F_START = -18
# VERIFIED (run, smp-slew s6): voice blocks `0x9251C` and `0x92528`; level `F15` at `+2`, rate
# `F17` at `+4`, initial ramp at `+5`; the pointer to the record is the word at `block - 2`.
BLOCKS = (0x9251C, 0x92528)
LEVEL_AT, RATE_AT, RAMP_AT, POINTER_BACK = 2, 4, 5, 2
SUSTAIN = 0xB0, 64


@dataclass(frozen=True)
class Write:
    """One decoded path 1 write: `field` of `record` (or a raw DM word with `record = -1`)."""

    tick: int
    record: int
    field: int
    word: int


@dataclass(frozen=True)
class Event:
    """One derived event of a phase. `kind`: `hold`, `release`, `start` or `ramp`."""

    tick: int
    kind: str
    record: int
    f16: float | None = None
    tau_ms: float | None = None
    level: float | None = None
    rate_d: float | None = None
    ramp: float | None = None


def as_float(word: int) -> float:
    return struct.unpack("<f", struct.pack("<I", word & 0xFFFFFFFF))[0]


def decode_writes(msgs: Iterable[tuple[int, object]]) -> list[Write]:
    """Path 1 messages `(tick, msg)` (`msg.path`, `msg.word1`, `msg.word2`) as `Write`s; the batch
    framing words (`op` 9 / 10) are dropped. DM words outside the record array keep `record -1`
    and the raw DM address in `field`."""
    out: list[Write] = []
    for tick, msg in msgs:
        if getattr(msg, "path", 0) != 1 or (msg.word1 >> 28) != 0:
            continue
        dm = msg.word1 & 0xFFFFFFF
        hit = si.decode(dm)
        out.append(Write(tick, *(hit if hit else (-1, dm)), msg.word2))
    return out


def decay_tau_ms(f16: float) -> float | None:
    """Time constant of the per-update decay factor `F16` (None for 1.0 or above)."""
    if not 0.0 < f16 < 1.0:
        return None
    return 1000.0 / (-math.log(f16) * UPDATE_HZ)


def rate_decay_count(word: int) -> float:
    """The hold decay `d` of `1 - d / 2^24` per update for a rate word `F17`."""
    return round((1.0 - as_float(word)) * float(1 << 24), 1)


def block_words(writes: Sequence[Write]) -> dict[int, int]:
    """Last value written to each raw DM word of the voice blocks (`record -1`)."""
    return {w.field: w.word for w in writes if w.record < 0}


def block_for_record(words: dict[int, int], record: int) -> int | None:
    """Voice block whose pointer word (`block - 2`) addresses `record`'s field 0."""
    for block in BLOCKS:
        pointer = words.get(block - POINTER_BACK)
        if pointer is not None and si.decode(pointer) == (record, 0):
            return block
    return None


def events(writes: Sequence[Write]) -> list[Event]:
    """Release, hold, start and ramp events of `writes`, in message order."""
    words = block_words(writes)
    per: dict[tuple[int, int], dict[int, int]] = {}
    for w in writes:
        if w.record >= 0:
            per.setdefault((w.tick, w.record), {})[w.field] = w.word
    out: list[Event] = []
    for (tick, record), fields in per.items():
        if F_START in fields:
            block = block_for_record(words, record)
            out.append(
                Event(
                    tick,
                    "start",
                    record,
                    level=as_float(words[block + LEVEL_AT]) if block else None,
                    rate_d=rate_decay_count(words[block + RATE_AT]) if block else None,
                    ramp=as_float(words[block + RAMP_AT]) if block else None,
                )
            )
        elif all(f in fields for f in F_RELEASE):
            f16 = as_float(fields[16])
            kind = "hold" if f16 >= 1.0 else "release"
            out.append(Event(tick, kind, record, f16=f16, tau_ms=decay_tau_ms(f16)))
        elif 3 in fields:
            out.append(Event(tick, "ramp", record, ramp=as_float(fields[3])))
    return out


# A scenario step: (kind, value, ticks to run afterwards). Kinds: `cc` (CC64 value), `on`
# (note-on with velocity `value`), `off`, `wait`. EMPIRICAL: the waits are long enough for every
# message of the phase to appear (a note-off writes at +3 ticks and at +40 ticks).
Step = tuple[str, int, int]
TAIL = 2000
SCENARIOS: dict[str, list[Step]] = {
    "plain": [("on", 90, 200), ("off", 0, TAIL)],
    "a": [("cc", 127, 12), ("on", 90, 200), ("off", 0, 400), ("cc", 0, TAIL)],
    "a2": [("on", 90, 200), ("cc", 127, 100), ("off", 0, 400), ("cc", 0, TAIL)],
    "c": [("on", 90, 200), ("cc", 127, 100), ("cc", 0, 200), ("off", 0, TAIL)],
    "d": [
        ("cc", 127, 12),
        ("on", 90, 200),
        ("off", 0, 200),
        ("on", 70, 400),
        ("off", 0, 400),
        ("cc", 0, TAIL),
    ],
    "half": [("cc", 64, 12), ("on", 90, 200), ("off", 0, 800), ("cc", 0, TAIL)],
    # Harness limit (docs/research/piano-pedal-keyup.md s4): a second note-on after a key-up
    # starts no voice, with and without the pedal.
    "again": [("on", 90, 200), ("off", 0, 2000), ("on", 70, 1500)],
    "again_pedal": [
        ("cc", 127, 12),
        ("on", 90, 200),
        ("off", 0, 400),
        ("cc", 0, 2000),
        ("on", 70, 1500),
    ],
}


def with_tail(steps: Sequence[Step], tail: int | None) -> list[Step]:
    """`steps` with the last step's run length replaced by `tail` (ticks) when given."""
    out = list(steps)
    if tail is not None:
        kind, value, _ticks = out[-1]
        out[-1] = (kind, value, tail)
    return out


def with_cc(steps: Sequence[Step], value: int | None) -> list[Step]:
    """`steps` with every non-zero CC64 value replaced by `value` when given (pedal sweep)."""
    if value is None:
        return list(steps)
    return [(k, value if k == "cc" and v else v, t) for k, v, t in steps]


def midi_bytes(kind: str, value: int, note: int) -> bytes:
    """MIDI bytes of a scenario step (`wait` sends none)."""
    if kind == "cc":
        return bytes([SUSTAIN[0], SUSTAIN[1], value & 0x7F])
    if kind == "on":
        return bytes([0x90, note & 0x7F, value & 0x7F])
    if kind == "off":
        return bytes([0x80, note & 0x7F, 0])
    return b""


def rows(scenario: str, phase: int, label: str, clock: int, evs: Iterable[Event]) -> list[str]:
    """CSV lines for the events of one phase; `clock` is the tick at the phase start."""

    def num(x: float | None, digits: int = 6) -> str:
        return "" if x is None else f"{x:.{digits}g}"

    return [
        ",".join(
            [
                scenario,
                str(phase),
                label,
                str(clock + e.tick),
                f"{(clock + e.tick) * TICK_US:.0f}",
                e.kind,
                str(e.record),
                num(e.f16),
                num(e.tau_ms),
                num(e.level),
                num(e.rate_d),
                num(e.ramp),
            ]
        )
        for e in evs
    ]


CSV_HEADER = "scenario,phase,step,tick,us,event,record,f16,tau_ms,level,rate_d,ramp"


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - thin CLI
    import arm_keydown as kd
    import arm_piano_load as pl

    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("image", type=Path)
    ap.add_argument("sram", type=Path, nargs="?")
    ap.add_argument("--cache", type=Path)
    ap.add_argument("--ns4b", type=Path, required=True)
    ap.add_argument("--npno", required=True)
    ap.add_argument("--note", type=int, default=60)
    ap.add_argument("--scenario", action="append", choices=sorted(SCENARIOS), default=[])
    ap.add_argument("--cc", type=int, help="replace the non-zero CC64 values of the scenario")
    ap.add_argument(
        "--tail", type=int, help="ticks to run after the last step (default per scenario)"
    )
    ns = ap.parse_args(argv)
    raw = pl.read_npno(ns.ns4b, ns.npno)
    print(CSV_HEADER)
    for name in ns.scenario or sorted(SCENARIOS):
        rig = kd.build_rig(ns.image, ns.sram, ns.cache)
        pl.LibraryStub({1: pl.device_image(raw, "Library.npno")}, pl.KIND_FULL).install(rig.emu)
        pl.load_sound(rig.emu, pl.managers(rig.emu)[0], 1)
        pl.select_sound(rig.emu, 1)
        pl.install_dsp_ready(rig.emu)
        rig.tick(2)
        clock = 0
        for i, (kind, value, ticks) in enumerate(
            with_tail(with_cc(SCENARIOS[name], ns.cc), ns.tail)
        ):
            data = midi_bytes(kind, value, ns.note)
            if data:
                rig.send(data)
            evs = events(decode_writes(rig.tick(ticks)))
            print("\n".join(rows(name, i, f"{kind}:{value}", clock, evs)))
            clock += ticks
    return 0


if __name__ == "__main__":
    sys.exit(main())
