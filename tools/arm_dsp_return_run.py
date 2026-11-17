#!/usr/bin/env python3
r"""Stub against return path: what the DSP answers change in the ARM firmware (D147).

One program, three modes from the same saved state (after the recall, the idle settle and the
sample bind):

* `legacy`: SPI1 is RAM (test pattern) and the voice status bit is set by hook (`DspAck`), the
  harness of D124;
* `neutral`: as `legacy` with the SPI read zeroed (D143, the default of `arm_dsp_capture`);
* `return`: the three SPI read routines answer from `arm_dsp_return.ReturnSource` (SY voice
  masks, shadow of the ARM words, initial DSP images), no `DspAck`.

Per mode: voice update calls, SY messages, ticks from each note-on to the first SY message
(key-down delay, 24 ticks = 1 ms), the SY state messages (prepare / go / reset) and when the
first reset follows the note-off, SPI words read at the note-on. Numbers only.

    uv run --with unicorn python tools/arm_dsp_return_run.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
        "factory_programs/Program/Bank A/X.ns4p" --eeprom target/oled/eeprom.bin \
        --ns4b "<restore>.ns4b" --images target/dsp_sram --out run.json
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

import arm_dsp_return as rt

KEYS, KEY_GAP, HOLD_TICKS, OFF_TICKS, VELOCITY = (60, 64, 67), 20, 100, 400, 100
VOICE_UPDATE = 0xC01E00AC
TICKS_PER_MS = 24


def first_tick(rows: list[tuple], pred) -> int | None:  # noqa: ANN001
    """Tick of the first row for which `pred(row)` holds (rows: tick, chip, path, w1, w2)."""
    return next((r[0] for r in rows if pred(r)), None)


def is_sy_state(row: tuple, states: tuple[int, ...]) -> bool:
    """True for an SY path 1 state message with a state in `states`."""
    return row[1] == "SY" and row[2] == 1 and rt.voice_of(row[3]) is not None and row[4] in states


def key_down_delay(sy_rows: list[tuple], note_tick: int, next_note_tick: int | None) -> int | None:
    """Ticks from a note-on to the first SY message before the next note-on (`None`: none)."""
    end = next_note_tick if next_note_tick is not None else 1 << 30
    t = first_tick([r for r in sy_rows if note_tick <= r[0] < end], lambda _r: True)
    return None if t is None else t - note_tick


def summarize(rows: list[tuple], note_ticks: list[int], off_tick: int) -> dict[str, object]:
    """Key-down delays, SY message counts and the first voice free after the note-off."""
    sy = [r for r in rows if r[1] == "SY"]
    nxt = [*note_ticks[1:], None]
    after_off = [r for r in rows if r[0] >= off_tick]
    free = first_tick(after_off, lambda r: is_sy_state(r, rt.IDLE_STATES))
    by_chip: dict[str, int] = {}
    for r in rows:
        by_chip[r[1]] = by_chip.get(r[1], 0) + 1
    return {
        "messages_by_chip": dict(sorted(by_chip.items())),
        "sy_messages": len(sy),
        "key_down_delay_ticks": [key_down_delay(sy, n, e) for n, e in zip(note_ticks, nxt)],
        "prepare": sum(is_sy_state(r, (rt.STATE_PREPARE,)) for r in rows),
        "go": sum(is_sy_state(r, (rt.STATE_GO,)) for r in rows),
        "free_after_off_ticks": None if free is None else free - off_tick,
        "frees": sum(is_sy_state(r, rt.IDLE_STATES) for r in after_off),
    }


def run_mode(ctx: dict, mode: str, keys: tuple[int, ...]) -> dict[str, object]:  # pragma: no cover
    """Reset to the saved state, run one mode, return its numbers."""
    import arm_keydown as kd
    import arm_steady as ast
    import arm_sy_voice as sv
    from arm_org_stream import TICK_INSNS

    session, emu, rig, flags = ctx["session"], ctx["emu"], ctx["rig"], ctx["flags"]
    session.reset()
    flags.update(ack=mode != "return", updates=0)
    ctx["spi"].update(neutral=mode == "neutral", spi=None)
    log, source = rt.ReadLog(), rt.ReturnSource(images=ctx["images"])
    handles = rt.install(emu, source, log) if mode == "return" else []
    rows: list[tuple] = []
    notes = {i * KEY_GAP: k for i, k in enumerate(keys)}
    try:
        for n in range(HOLD_TICKS + OFF_TICKS):
            log.tick = n
            if n in notes:
                rig.send(kd.note_on(notes[n], VELOCITY))
            if n == HOLD_TICKS:
                for k in keys:
                    rig.send(kd.note_off(k))
            emu.clear_messages()
            ast.run_ticking(emu, TICK_INSNS, per_tick=TICK_INSNS)
            sv.pump(emu)
            for m in emu.messages:
                rows.append((n, m.chip, m.path, m.word1, m.word2))
                source.observe(m.comm, m.core, m.path, m.word1, m.word2)
    finally:
        rt.uninstall(emu, handles)
    out = summarize(rows, list(notes), HOLD_TICKS)
    out.update(
        voice_updates=flags["updates"],
        spi_word=ctx["spi"]["spi"],
        peek_addresses=sorted({hex(c[2]) for c in log.calls if c[0] == rt.SPI_READ4}),
        peek_calls=sum(1 for c in log.calls if c[0] == rt.SPI_READ4),
        status_reads=sum(1 for c in log.calls if c[0] != rt.SPI_READ4),
        first_calls=[[hex(x) for x in c] for c in log.calls[:8]],
        answer_layers=dict(source.layer),
        stub_fallbacks=log.fallbacks,
        busy_voices_end=rt.VOICES - bin(source.sy.idle).count("1"),
    )
    return out


def prepare(ns) -> dict:  # noqa: ANN001  # pragma: no cover
    """Boot, recall, settle, bind the stand-in sample, install the toggled hooks, save the state."""
    import arm_dsp_capture as dc
    import arm_keydown as kd
    import arm_piano_load as pl
    import arm_program_load as apl
    import arm_screens as sc
    import arm_sy_voice as sv
    import arm_voice_gate as vg

    session = sc.Session(ns.image, ns.sram)
    program = ns.program.read_bytes()
    session.load_eeproms(apl.build_image(ns.eeprom.read_bytes(), program, ns.program.stem[:23]))
    kd.install_board_present(session.emu)
    session.boot()
    emu = session.emu
    rig = kd.Rig(emu, kd.Uart2().install(emu))
    emu.install_dsp_ports()
    flags = {"ack": True, "updates": 0}

    def ack(_uc, _a, _s, _u) -> None:  # noqa: ANN001
        if flags["ack"]:
            address, mask = sv.ack_address(emu.reg(5), emu.reg(6), emu.read32)
            emu.write32(address, emu.read32(address) | mask)

    def update(_uc, _a, _s, _u) -> None:  # noqa: ANN001
        flags["updates"] += 1

    for pc in sv.ACK_PCS:
        emu.uc.hook_add(emu._u.UC_HOOK_CODE, ack, begin=pc, end=pc)
    emu.uc.hook_add(emu._u.UC_HOOK_CODE, update, begin=VOICE_UPDATE, end=VOICE_UPDATE)
    spi = vg.install_spi_neutral(emu, {"neutral": False, "spi": None})
    dc.run_phase(rig, dc.LOAD_TICKS, "load")
    dc.run_phase(rig, dc.IDLE_TICKS, "idle")
    sample = pl.read_npno(ns.ns4b, dc.SAMPLE_MEMBER, 0x400000)
    layers = dc.bind_samples(emu, sample)
    session.load_state(session.state_bytes())
    return {
        "session": session,
        "emu": emu,
        "rig": rig,
        "flags": flags,
        "spi": spi,
        "layers": layers,
    }


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local firmware
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("image", type=Path)
    ap.add_argument("sram", type=Path)
    ap.add_argument("program", type=Path)
    ap.add_argument("--eeprom", type=Path, required=True)
    ap.add_argument("--ns4b", type=Path, required=True)
    ap.add_argument("--images", type=Path, help="directory with PNO/ and ORG/ core<N>_sram.bin")
    ap.add_argument("--modes", nargs="*", default=["legacy", "neutral", "return"])
    ap.add_argument("--out", type=Path)
    ns = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    t0 = time.perf_counter()
    ctx = prepare(ns)
    ctx["images"] = rt.load_images(ns.images, {1: "PNO", 2: "ORG"})
    result: dict = {"program": ns.program.stem, "flagged_layers": ctx["layers"], "modes": {}}
    for mode in ns.modes:
        result["modes"][mode] = run_mode(ctx, mode, KEYS)
        result["modes"][mode]["wall_s"] = round(time.perf_counter() - t0)
    text = json.dumps(result, indent=1)
    if ns.out:
        ns.out.write_text(text, encoding="utf-8")
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
