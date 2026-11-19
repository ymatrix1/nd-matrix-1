#!/usr/bin/env python3
"""Long key phase of one SAMPLE program in the ARM harness (D154).

Prints which SY messages follow the note-on.

`arm_dsp_capture` stops 40 ticks after the note-on. This probe boots one program the same way
(sample bound, SPI neutral, DSP ack) and keeps ticking for `--ticks` after one note-on, printing
every SY / SMP path 1 message with its tick and sender pc, so the messages after the prepare state
(go, gain target) can be found.

    uv run --with unicorn python tools/sample_gaps_probe.py CODE SRAM --eeprom E --ns4b B \
        --member "Samp Lib/X.nsmp4" --key 60 --ticks 400 PROGRAM.ns4p
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

KEY_PHASE = "key"


def summarize(rows: list[str]) -> dict[str, int]:
    """Message count per `chip:path` of formatted capture rows (`phase tick chip path ...`)."""
    out: dict[str, int] = {}
    for line in rows:
        p = line.split()
        k = f"{p[2]}:{p[3]}"
        out[k] = out.get(k, 0) + 1
    return out


def sy_rows(rows: list[str]) -> list[str]:
    """Only the SY rows of formatted capture rows."""
    return [r for r in rows if r.split()[2] == "SY"]


def probe(ns) -> list[str]:  # noqa: ANN001  # pragma: no cover - needs the firmware
    import arm_dsp_capture as ac
    import arm_keydown as kd
    import arm_piano_load as pl
    import arm_program_load as apl
    import arm_screens as sc

    program = ns.program
    sample = pl.read_npno(ns.ns4b, ns.member, ns.limit) if ns.ns4b else None
    session = sc.Session(ns.image, ns.sram)
    session.load_eeproms(
        apl.build_image(ns.eeprom.read_bytes(), program.read_bytes(), program.stem[:23])
    )
    kd.install_board_present(session.emu)
    session.boot()
    rig = kd.Rig(session.emu, kd.Uart2().install(session.emu))
    session.emu.install_dsp_ports()
    ac.install_dsp_ack(session.emu)
    import arm_voice_gate as vg

    vg.install_spi_neutral(session.emu)
    ac.run_phase(rig, ac.LOAD_TICKS, "load")
    ac.run_phase(rig, ac.IDLE_TICKS, "idle")
    if sample is not None:
        ac.bind_samples(session.emu, sample)
    session.load_state(session.state_bytes())
    session.reset()
    rows = ac.run_phase(rig, ns.ticks, KEY_PHASE, lambda: rig.send(kd.note_on(ns.key, ac.VELOCITY)))
    return rows


def main(argv: list[str] | None = None) -> int:  # pragma: no cover
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("image", type=Path)
    ap.add_argument("sram", type=Path)
    ap.add_argument("--eeprom", type=Path, required=True)
    ap.add_argument("--ns4b", type=Path)
    ap.add_argument("--member", default="")
    ap.add_argument(
        "--limit", type=lambda v: int(v, 0), default=0x400000, help="bytes of the member served"
    )
    ap.add_argument("--key", type=int, default=60)
    ap.add_argument("--ticks", type=int, default=400)
    ap.add_argument("program", type=Path)
    ns = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    rows = probe(ns)
    print("summary", summarize(rows))
    for r in sy_rows(rows):
        print(r)
    return 0


if __name__ == "__main__":
    sys.exit(main())
