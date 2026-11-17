#!/usr/bin/env python3
"""Program recall to the three synth velocity edit values, no poke (D104 follow-up, D142).

Puts a factory `.ns4p` into the Store record of the I2C EEPROM image
(`arm_program_load.build_image`), boots the full machine so the firmware recalls the program
itself, then reads the recalled value of `Edt_Syn_kAmpEnvVel` (0x131), `Edt_Syn_kFltEnvVel`
(0x13C) and `Edt_Syn_kOscEnvVel` (0x124) of every synth layer store through the layer value table
(`arm_edit_capture.value_cell`). The file value is the one the Rust decoder reads
(`cbin/synth_preset.rs`: amp `+216` 2b, filter `+339` 1b, osc env velocity bit); this tool
compares the two per layer.

    python tools/arm_program_synth_velocity.py <CODE> <SRAM> --eeprom target/oled/eeprom.bin \
        --ns4b "<backup>.ns4b" --program "Program/Bank A/Leaky Stabs II.ns4p" \
        --expect A=1,0,0 B=0,0,0 C=0,0,0

`--expect LAYER=amp,flt,osc` is the file value (census CSV columns
`amp_setting,flt_vel,osc_env_vel`).
Reads the local firmware only; output is derived numbers (never commit the firmware).
"""

from __future__ import annotations

import argparse
import sys
import zipfile
from pathlib import Path

# VERIFIED (`docs/research/arm-edit-params.csv`): the three synth velocity edit ids.
EDIT_IDS = {"amp": 0x131, "flt": 0x13C, "osc": 0x124}
LAYER_NAMES = ("A", "B", "C")


def parse_expect(items: list[str]) -> dict[str, tuple[int, int, int]]:
    """`["A=1,0,0", ...]` to `{"A": (amp, flt, osc)}`."""
    out = {}
    for item in items:
        layer, _, vals = item.partition("=")
        nums = tuple(int(v) for v in vals.split(","))
        if layer not in LAYER_NAMES or len(nums) != 3:
            raise ValueError(f"bad --expect {item!r}, want LAYER=amp,flt,osc")
        out[layer] = nums  # type: ignore[assignment]
    return out


def compare(
    recalled: dict[str, tuple[int, int, int]], expect: dict[str, tuple[int, int, int]]
) -> list[str]:
    """Mismatch lines between recalled and file values (empty when equal)."""
    return [
        f"layer {layer}: recalled {recalled.get(layer)} != file {want}"
        for layer, want in expect.items()
        if recalled.get(layer) != want
    ]


def read_member(ns4b: Path, member: str) -> bytes:
    """Bytes of an archive member."""
    with zipfile.ZipFile(ns4b) as archive, archive.open(member) as handle:
        return handle.read()


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local firmware
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("image", type=Path)
    ap.add_argument("sram", type=Path)
    ap.add_argument("--eeprom", type=Path, required=True)
    ap.add_argument("--ns4b", type=Path, required=True)
    ap.add_argument("--program", required=True, help="archive member of the .ns4p")
    ap.add_argument("--expect", nargs="*", default=[], help="LAYER=amp,flt,osc file values")
    ap.add_argument("--ticks", type=int, default=300)
    ns = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import arm_edit_capture as aec
    import arm_program_load as pg
    import arm_screens as sc

    program = read_member(ns.ns4b, ns.program)
    session = sc.Session(ns.image, ns.sram)
    session.load_eeproms(
        pg.build_image(ns.eeprom.read_bytes(), program, Path(ns.program).stem[:23])
    )
    session.boot()
    session.run(ns.ticks)
    emu = session.emu
    synth = [s for s in aec.find_stores(emu) if s.kind == "synth"]
    # VERIFIED (run, D142): each synth layer client owns two stores (client 4, 5, 6 = layer A, B, C)
    # that hold the same three values; the clients are listed in layer order.
    by_client: dict[int, list[tuple[int, ...]]] = {}
    for store in synth:
        vals = []
        for eid in EDIT_IDS.values():
            cell = aec.value_cell(emu, store.address, eid)
            vals.append(emu.read32(cell) if cell else -1)
        by_client.setdefault(store.layer, []).append(tuple(vals))
        print(f"store {store.address:#x} client {store.layer}: amp,flt,osc = {vals}")
    recalled = {}
    for name, client in zip(LAYER_NAMES, sorted(by_client)):
        vals = by_client[client]
        if len(set(vals)) != 1:
            print(f"MISMATCH layer {name}: its stores disagree {vals}")
        recalled[name] = vals[0]
    problems = compare(recalled, parse_expect(ns.expect))
    for p in problems:
        print("MISMATCH", p)
    return int(bool(problems))


if __name__ == "__main__":
    sys.exit(main())
