#!/usr/bin/env python3
"""Decode and log the synth key words the ARM sends to PNO core 2 (`DM(0xC1DCC + n)`, OQ40).

A synth key word is one 32-bit path 2 store per voice to PNO core 2:

- bits 31..24, flags (EMPIRICAL): `0x08` = played by a keyboard key (the arpeggiator clears it);
  bits 1..0 = voice bank bits (`0x03` both banks, `0x01` / `0x02` one bank); bits 6..5 = voice
  index inside a key group (`0x20`, `0x40`: 2nd and 3rd voice of a chord note);
- bits 23..16, signed note offset from key 60 (`0xE0` = 28, `0xF4` = 48). VERIFIED:
  `docs/research/data/keydown-synth-keys.csv`, checked by `tools/tests/test_arm_key_words.py`;
- bits 15..8, `0x80 + g(velocity)` (`0xE2` at velocity 100), the same in the on and off word.
  VERIFIED: `synth-strip-path.md` s6;
- bits 7..0, 0 (EMPIRICAL).

Plain key: on `0x0B`, off `0x08`; arp step: on `0x03` (both banks) or `0x01` / `0x02` (one bank),
off `0x00` (`docs/research/data/keydown-synth-c4e4g4.script`, `silent-programs.md` s6).

Run as a script it wraps `tools/arm_dsp_live.py` (same arguments after the first two) and writes
every ARM message of the given comm indexes with its ARM tick to a JSON file, numbers only:

    python tools/arm_key_words.py 1 key_words.json CODE SRAM --eeprom E --program P \
        --live L --chip "PNO:2=org=IMAGE=0xC1F89:4:2"
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

KEYBOARD_BIT = 0x08
BANK_MASK = 0x03
GROUP_SHIFT, GROUP_MASK = 5, 0x03


def decode(value: int) -> dict[str, int | bool]:
    """Fields of one key word (see the module table)."""
    flags = (value >> 24) & 0xFF
    note = (value >> 16) & 0xFF
    return {
        "keyboard": bool(flags & KEYBOARD_BIT),
        "banks": flags & BANK_MASK,
        "group_voice": (flags >> GROUP_SHIFT) & GROUP_MASK,
        "note": 60 + (note - 256 if note >= 128 else note),
        "velocity_byte": (value >> 8) & 0xFF,
    }


def is_on(value: int) -> bool:
    """An on word has at least one bank bit set (the off word clears both)."""
    return bool((value >> 24) & BANK_MASK)


def with_both_banks(value: int) -> int:
    """The word with both bank bits set (the poke that made Synth Engine sound, s6 of the note)."""
    return value | (BANK_MASK << 24) if is_on(value) else value


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local firmware
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import arm_dsp_live as live
    import arm_dsp_route as rt

    args = list(sys.argv[1:] if argv is None else argv)
    comms, out = {int(x) for x in args.pop(0).split(",")}, Path(args.pop(0))
    log: list[list[int]] = []
    cur = {"tick": -1}
    feed, dispatch = live.Feeder.feed, rt.Router.dispatch

    def tick_feed(self) -> None:  # noqa: ANN001
        cur["tick"] = self.tick
        feed(self)

    def logged(self, path, comm, core, dm_word, value):  # noqa: ANN001, ANN202
        if comm in comms:
            log.append([cur["tick"], comm, core, path, dm_word, value])
        return dispatch(self, path, comm, core, dm_word, value)

    live.Feeder.feed, rt.Router.dispatch = tick_feed, logged
    rc = live.main(args)
    out.write_text(json.dumps(log), encoding="utf-8")
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
