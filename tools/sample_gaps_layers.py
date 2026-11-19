#!/usr/bin/env python3
"""Which SAMPLE layers start a voice when only some are bound (D154).

Boots one program like `arm_dsp_capture` (sample bound, SPI neutral, DSP ack), binds the loaded
member to the chosen synth layer table indexes only (`0 2 4` = synth A B C) and plays one key for
60 ticks. Prints the SY voice records that reach prepare. Used to show that the harness starts at
most 2 voices for 3 bound layers (`sample-gaps.md` s4.3).

    uv run --with unicorn python tools/sample_gaps_layers.py CODE SRAM --eeprom E --ns4b B \
        --member "Samp Lib/X.nsmp4" --key 72 --bind 0,2,4 PROGRAM.ns4p
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

STATE_PREPARE = "0x001c07cd"
TICKS = 60  # EMPIRICAL: the prepare of every voice that starts arrives within 40 ticks of the key


def prepared_records(rows: list[str]) -> list[int]:
    """Sorted state word addresses that received prepare in formatted SY rows."""
    out = set()
    for line in rows:
        p = line.split()
        if p[2] == "SY" and p[6] == STATE_PREPARE:
            out.add(int(p[5], 16))
    return sorted(out)


def parse_bind(text: str) -> list[int]:
    """`"0,2,4"` -> `[0, 2, 4]`; `"-"` or empty -> none."""
    return [int(x) for x in text.split(",") if x not in ("", "-")]


def run(ns) -> list[str]:  # noqa: ANN001  # pragma: no cover - needs the firmware
    import arm_dsp_capture as ac
    import arm_keydown as kd
    import arm_piano_load as pl
    import arm_program_load as apl
    import arm_sample_probe as sp
    import arm_screens as sc
    import arm_voice_gate as vg

    data = pl.read_npno(ns.ns4b, ns.member, ns.limit)
    session = sc.Session(ns.image, ns.sram)
    program = ns.program
    session.load_eeproms(
        apl.build_image(ns.eeprom.read_bytes(), program.read_bytes(), program.stem[:23])
    )
    kd.install_board_present(session.emu)
    session.boot()
    rig = kd.Rig(session.emu, kd.Uart2().install(session.emu))
    session.emu.install_dsp_ports()
    ac.install_dsp_ack(session.emu)
    vg.install_spi_neutral(session.emu)
    ac.run_phase(rig, ac.LOAD_TICKS, "load")
    ac.run_phase(rig, ac.IDLE_TICKS, "idle")
    sp.serve_sample(session.emu, sp.sample_device_image(data, sp.sample_name(data)))
    if sp.load_sample(session.emu) != 0:
        raise RuntimeError("sample loader failed")
    for layer in parse_bind(ns.bind):
        sp.bind_layer(session.emu, layer)
    session.load_state(session.state_bytes())
    session.reset()
    return ac.run_phase(rig, TICKS, "key", lambda: rig.send(kd.note_on(ns.key, ac.VELOCITY)))


def main(argv: list[str] | None = None) -> int:  # pragma: no cover
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("image", type=Path)
    ap.add_argument("sram", type=Path)
    ap.add_argument("--eeprom", type=Path, required=True)
    ap.add_argument("--ns4b", type=Path, required=True)
    ap.add_argument("--member", required=True)
    ap.add_argument("--limit", type=lambda v: int(v, 0), default=0x1000000)
    ap.add_argument("--key", type=int, default=60)
    ap.add_argument("--bind", default="0,2,4")
    ap.add_argument("program", type=Path)
    ns = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    recs = prepared_records(run(ns))
    print("bind", ns.bind, "key", ns.key, "voices", [hex(r) for r in recs])
    return 0


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
