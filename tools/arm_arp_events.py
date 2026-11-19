#!/usr/bin/env python3
"""Arpeggiator note events of the ARM firmware for a program with held keys (OQ32 follow-up).

Boots the full machine with a `.ns4p` in the Store record (`arm_dsp_capture.capture` setup), lets
the firmware recall it, then holds keys through the UART2 MIDI model and logs, with the 24 kHz
timer counter `0x80001DC0` as the clock (D130):

* `player`    every call of the pattern player `0xC023B3EC` (pattern object, phase);
* `listener`  every event the player hands to the Glue listener `0xC0123A18`: `w[0]` = 1 note on,
              0 note off, `w[1]` = arp note index (`cycle * 128 + record tick`);
* `voice`     after every 25,000-instruction tick that had an event: the voice lists of the
              objects `0xC01E81DC` (note on) and `0xC01E80B8` (note off) work on (`[obj+0x14]`
              count, `[obj+0x18]` notes, `[obj+0x1C]` velocities).

**Why the position is fed.** With Master Clock sync (`kArpClkEnable`, `[Glue+0xCC] = 1`) the Glue
removes the pattern object from the listener list of its own clock (`0xC01E6368`, rate = dial table
`R[raw]`) and registers it with the Master Clock position updater `0xC023DE74` (called from
`0xC023DFF4` with the objects `[[0xC059B21C]+0xE8]` and `+0xEC`). The updater reads a 24-bit
position over SPI (`0xC0011B60`) and hands `phase` to the pattern. The harness has no device behind
that SPI read (RAM test pattern: phase jumps of `0x1000000` per call, 1,500 events per second), so
the run overrides the read at its two return sites with `position(now)` = master phase units at a
chosen tempo: `tempo / 60 * 2**19` units per second (`phase_units`). The scale is ASSUMED (a
firmware beat of `2**19` units is one quarter note at the Master Clock tempo, the only scale under
which grid index 6 gives the `1/16` the manual and the video show); the step time of a run is
therefore **an input, not a measurement**. What the firmware decides is the note sequence, the
velocity, the gate fraction of a step and the latency of the first note.

    uv run --with unicorn python tools/arm_arp_events.py CODE SRAM --eeprom target/oled/eeprom.bin \
        --out docs/research/data/arp-fw-events-a16.json --tempo 100 \
        "factory_programs/Program/Bank A/Horizon Whl.ns4p" 40 40,43,47 60 60,64,67

Only numbers are written. Needs the unpacked firmware and `unicorn` (about 70 s boot, then about
100 s CPU per second of ARM time at 25,000 instructions per timer interrupt).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

TICK_HZ = 24_000  # VERIFIED (D130): Timer0 PRD34 = 1000 at 24 MHz, counter `0x80001DC0` += 1
PHASE_PER_BEAT = 1 << 19  # firmware beat (`ns4_arp_gate.PHASE_PER_BEAT`)
INSNS_PER_TICK = 25_000  # 2x the 12,500 cycles the 300 MHz ARM has per tick; D130: no difference
PLAYER = 0xC023B3EC
LISTENER = 0xC0123A18
NOTE_ON = 0xC01E81DC
NOTE_OFF = 0xC01E80B8
POSITION_SITES = (0xC023DED8, 0xC023DFD8)  # `ldr r1, [sp, #0xc]` after the SPI position read
MASTER_ROOT = 0xC059B21C  # `[root]` -> object with the two master phase objects at +0xE8 / +0xEC
MASTER_OFFSETS = (0xE8, 0xEC)
PATTERN_TABLE = 0xC054CBD0  # 1-based table of pattern objects (position 1, 3, 5 = layer A, B, C)
LAYER_OF_POSITION = {1: "A", 3: "B", 5: "C"}
COUNTER = 0x80001DC0


def phase_units(ticks: float, tempo_bpm: float) -> float:
    """Master phase units after `ticks` timer interrupts at `tempo_bpm` (ASSUMED scale, see doc)."""
    return ticks * tempo_bpm / 60.0 * PHASE_PER_BEAT / TICK_HZ


def position_word(units: float) -> int:
    """The 32-bit value the SPI read returns for `units`: a 24-bit position in bits 8..31."""
    return (int(units) & 0xFFFFFF) << 8


def ticks_to_ms(ticks: float) -> float:
    return ticks * 1000.0 / TICK_HZ


class Recorder:  # pragma: no cover - needs the local firmware
    """Code hooks of one session: player, listener, note on / off and the position feed."""

    def __init__(self, emu, tempo: float) -> None:  # noqa: ANN001
        self.emu, self.tempo = emu, tempo
        self.player_obj = 0
        self.objects: set[int] = set()
        self.events: list[dict] = []
        self.voice: list[dict] = []
        self.t_ref = 0
        self.feed = False
        self.dirty = False
        self.pattern_layers = {
            emu.read32(PATTERN_TABLE + 4 * pos): name for pos, name in LAYER_OF_POSITION.items()
        }
        for pc, fn in ((PLAYER, self.on_player), (LISTENER, self.on_listener)):
            emu.uc.hook_add(emu._u.UC_HOOK_CODE, fn, begin=pc, end=pc)
        for pc in (NOTE_ON, NOTE_OFF):
            emu.uc.hook_add(emu._u.UC_HOOK_CODE, self.on_note, begin=pc, end=pc)
        for pc in POSITION_SITES:
            emu.uc.hook_add(emu._u.UC_HOOK_CODE, self.on_position, begin=pc, end=pc)

    def now(self) -> int:
        return self.emu.read32(COUNTER) | self.emu.read32(COUNTER + 4) << 32

    def on_player(self, _uc, _a, _s, _u) -> None:  # noqa: ANN001
        self.player_obj = self.emu.reg(0)
        self.events.append(
            {"t": self.now(), "ev": "player", "layer": self.pattern_layers.get(self.player_obj),
             "phase": self.emu.reg(2) | self.emu.reg(3) << 32}
        )  # fmt: skip

    def on_listener(self, _uc, _a, _s, _u) -> None:  # noqa: ANN001
        ev = self.emu.reg(1)
        self.dirty = True
        self.events.append(
            {"t": self.now(), "ev": "listener", "layer": self.pattern_layers.get(self.player_obj),
             "type": self.emu.read32(ev), "index": self.emu.read32(ev + 4)}
        )  # fmt: skip

    def on_note(self, _uc, _a, _s, _u) -> None:  # noqa: ANN001
        self.objects.add(self.emu.reg(0))
        self.dirty = True

    def on_position(self, _uc, _a, _s, _u) -> None:  # noqa: ANN001
        if self.feed:
            word = position_word(phase_units(self.now() - self.t_ref, self.tempo))
            self.emu.write32(self.emu.reg(13) + 0xC, word)

    def snapshot(self, tick: int) -> None:
        """Voice lists of every note object seen, after a tick with events."""
        if not self.dirty:
            return
        self.dirty = False
        for obj in sorted(self.objects):
            n = self.emu.read32(obj + 0x14)
            if n > 16:
                continue
            notes_at, vel_at = self.emu.read32(obj + 0x18), self.emu.read32(obj + 0x1C)
            self.voice.append(
                {"t": self.now(), "obj": hex(obj), "n": n,
                 "notes": [self.emu.read32(notes_at + 4 * i) for i in range(n)],
                 "vel": [self.emu.read32(vel_at + 4 * i) for i in range(n)]}
            )  # fmt: skip

    def restart_masters(self) -> None:
        """Zero the master phase accumulators and the pattern 'previous phase' words."""
        root = self.emu.read32(MASTER_ROOT)
        for off in MASTER_OFFSETS:
            obj = self.emu.read32(root + off)
            for field in (0x1C, 0x20, 0x24):
                self.emu.write32(obj + field, 0)
        for pat in self.pattern_layers:
            self.emu.write32(pat + 0x14, 0)
        self.t_ref = self.now()
        self.feed = True


def run_scenarios(ns) -> dict:  # noqa: ANN001  # pragma: no cover - needs the local firmware
    import arm_dsp_capture as dc
    import arm_keydown as kd
    import arm_program_load as apl
    import arm_screens as sc
    import arm_steady as ast
    import arm_sy_voice as sv
    import arm_voice_gate as vg

    program = ns.program
    session = sc.Session(ns.image, ns.sram)
    session.load_eeproms(
        apl.build_image(ns.eeprom.read_bytes(), program.read_bytes(), program.stem[:23])
    )
    kd.install_board_present(session.emu)
    session.boot()
    emu = session.emu
    rig = kd.Rig(emu, kd.Uart2().install(emu))
    emu.install_dsp_ports()
    dc.install_dsp_ack(emu)
    vg.install_spi_neutral(emu)
    rec = Recorder(emu, ns.tempo)
    dc.run_phase(rig, dc.LOAD_TICKS, "load")
    dc.run_phase(rig, dc.IDLE_TICKS, "idle")
    session.load_state(session.state_bytes())
    out = {"program": program.stem, "tempo": ns.tempo, "insns_per_tick": INSNS_PER_TICK, "runs": []}
    for text in ns.keys:
        keys = [int(k) for k in text.split(",")]
        session.reset()
        rec.events.clear()
        rec.voice.clear()
        rec.objects.clear()
        rec.restart_masters()
        ast.run_ticking(emu, INSNS_PER_TICK * ns.lead, per_tick=INSNS_PER_TICK)
        for key in keys:
            rig.send(kd.note_on(key, ns.velocity))
        t_on = rec.now()
        for tick in range(int(ns.seconds * TICK_HZ)):
            if ns.release_at is not None and tick == int(ns.release_at * TICK_HZ):
                for key in keys:
                    rig.send(kd.note_off(key))
            ast.run_ticking(emu, INSNS_PER_TICK, per_tick=INSNS_PER_TICK)
            sv.pump(emu)
            rec.snapshot(tick)
        out["runs"].append(
            {"keys": keys, "velocity": ns.velocity, "key_on_tick": t_on,
             "master_ref_tick": rec.t_ref, "events": list(rec.events), "voice": list(rec.voice)}
        )  # fmt: skip
        print(f"keys {keys}: {len(rec.events)} events", flush=True)
    return out


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local firmware
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("image", type=Path)
    ap.add_argument("sram", type=Path)
    ap.add_argument("--eeprom", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--tempo", type=float, default=100.0, help="Master Clock BPM fed to the sync")
    ap.add_argument("--seconds", type=float, default=1.5)
    ap.add_argument("--velocity", type=int, default=100)
    ap.add_argument("--lead", type=int, default=3, help="ticks run after the master restart")
    ap.add_argument("--release-at", type=float, default=None, help="second of the note-off")
    ap.add_argument("program", type=Path)
    ap.add_argument("keys", nargs="+", help="key sets, e.g. 60 60,64,67")
    ns = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    ns.out.write_text(json.dumps(run_scenarios(ns)) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
