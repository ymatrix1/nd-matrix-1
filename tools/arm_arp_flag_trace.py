#!/usr/bin/env python3
"""Who writes the arpeggiator gate flag `[pattern object + 0xC]` at run time (OQ32, D168).

D130 scanned 37 static `str rX, [rN, #0xC]` sites and traced none. This tool replaces the scan
with a **dynamic** check that does not depend on the store form (`str`, `strb`, `stm`, a
`memcpy`): a Unicorn write hook over the six 0xE0 byte pattern objects (`PATTERN_TABLE`,
`0xC054CBD0`, 1-based; layers A, B, C at positions 1, 3, 5, the others are the pattern slots the
Glue keeps per layer pair) logs every store with its PC and LR, for a program recall, the idle
run, the key-on, the held arpeggio and the release. It also reads the flag of all six objects
after each phase and measures the gate of every layer from the listener events (the on / off
edges the player hands to the Glue), so one run answers "which writes, which gate".

Program variants are made by patching the layer block of the original `.ns4p` bits (the JSON
round trip is lossy, D166): `--patch A.mode=2 --patch C.pattern_on=1 --patch A.rate=12`. Fields
are the ones of `ns4_arp_modes.FIELDS` plus `gate`, `accent`, `length` of `ns4_arp_patterns`.

    uv run --with unicorn python tools/arm_arp_flag_trace.py CODE SRAM \
        --eeprom target/oled/eeprom.bin --out run.json PROGRAM.ns4p

Only numbers are written. Needs the unpacked firmware and `unicorn` (about 70 s boot, then about
100 s CPU per second of ARM time at 25,000 instructions per timer interrupt).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import arm_arp_events as ev
import arp_fw_compare as cmp
import ns4_arp_modes as modes
import ns4_arp_patterns as pats
import ns4_bitscan as bs
import ns4_synth_fields as sf

OBJECT_SIZE = 0xE0  # VERIFIED (D130): the pattern constructor 0xC00FD7B8 allocates 0xE0 bytes
FLAG_OFFSET = 0xC  # `[obj + 0xC]`, set to 1 by the constructor at 0xC00FD814
CONSTRUCTOR = 0xC00FD7B8
FLAGSCAN = Path(__file__).resolve().parent.parent / "docs/research/data/arm-tick-flagscan.json"
TABLE_POSITIONS = (1, 2, 3, 4, 5, 6)  # the six pattern objects of the 1-based table
LAYER_FIELDS: dict[str, tuple[int, int]] = {
    **modes.FIELDS,
    "length": pats.LENGTH,
    "accent": pats.ACCENT,
    "gate": pats.GATE,
    "pan": pats.PAN,
}


def set_bits(raw: bytes, bit: int, width: int, value: int) -> bytes:
    """`raw` with the `width` bits at MSB-first bit offset `bit` replaced by `value`."""
    if value < 0 or value >> width:
        raise ValueError(f"value {value} does not fit {width} bits")
    out = bytearray(raw)
    for i in range(width):
        idx = bit + i
        want = (value >> (width - 1 - i)) & 1
        mask = 0x80 >> (idx % 8)
        out[idx // 8] = (out[idx // 8] & ~mask & 0xFF) | (mask if want else 0)
    return bytes(out)


def patch_layer(raw: bytes, layer: str, field: str, value: int) -> bytes:
    """A `.ns4p` with one layer field of the synth block replaced (`layer` A, B or C)."""
    base_bit = (sf.NS4P_SYNTH_OFFSET * 8) + bs.LAYER_BASES["ABC".index(layer)]
    start, width = LAYER_FIELDS[field]
    return set_bits(raw, base_bit + start, width, value)


def parse_patch(text: str) -> tuple[str, str, int]:
    """`C.pattern_on=1` -> `("C", "pattern_on", 1)`."""
    left, value = text.split("=")
    layer, field = left.split(".")
    if layer not in "ABC" or len(layer) != 1 or field not in LAYER_FIELDS:
        raise ValueError(f"bad patch {text!r}")
    return layer, field, int(value, 0)


def apply_patches(raw: bytes, patches: list[str]) -> bytes:
    for text in patches:
        raw = patch_layer(raw, *parse_patch(text))
    return raw


def store_base_register(word: int) -> int:
    """Base register `Rn` of an A32 single data transfer word (bits 19:16)."""
    return (word >> 16) & 0xF


def load_sites(path: Path = FLAGSCAN) -> list[int]:
    """Candidate `str [rN, #0xC]` addresses of the D130 scan (`arm_tick_probe flagscan`)."""
    return [int(row["address"], 16) if isinstance(row["address"], str) else row["address"]
            for row in json.loads(path.read_text(encoding="utf-8"))]  # fmt: skip


def layer_summary(run: dict, layer: str) -> dict:
    """Edge statistics of one layer of a run (`arp_fw_compare.summarize`)."""
    return cmp.summarize(*cmp.fw_edges(run, layer))


class FlagWatch:  # pragma: no cover - needs the local firmware
    """Write hooks over the pattern objects; keeps `[obj + 0xC]` writes and per-offset counts."""

    def __init__(self, emu, rec: ev.Recorder) -> None:  # noqa: ANN001
        self.emu, self.rec = emu, rec
        self.phase = "boot"
        self.table = {pos: emu.read32(ev.PATTERN_TABLE + 4 * pos) for pos in TABLE_POSITIONS}
        self.flag_writes: list[dict] = []
        self.offset_pcs: dict[str, set[str]] = {}
        for pos, obj in self.table.items():
            emu.uc.hook_add(
                emu._u.UC_HOOK_MEM_WRITE, self.on_write, begin=obj, end=obj + OBJECT_SIZE - 1,
                user_data=pos,
            )  # fmt: skip

    def on_write(self, _uc, _acc, addr, size, value, pos) -> None:  # noqa: ANN001
        obj = self.table[pos]
        off = addr - obj
        pc = self.emu.reg(15)
        if off <= FLAG_OFFSET < off + size:
            self.flag_writes.append(
                {"phase": self.phase, "t": self.rec.now(), "pos": pos, "size": size,
                 "value": value & ((1 << (8 * size)) - 1), "pc": hex(pc),
                 "lr": hex(self.emu.reg(14))}
            )  # fmt: skip
        self.offset_pcs.setdefault(f"{pos}:{off:#x}", set()).add(hex(pc))

    def flags(self) -> dict[str, int]:
        return {str(p): self.emu.read32(o + FLAG_OFFSET) for p, o in self.table.items()}

    def grid(self) -> dict[str, int]:
        """`[obj + 0x10]` grid pointers (the table the player reads: flag 0 or 1 picks it)."""
        return {str(p): self.emu.read32(o + 0x10) for p, o in self.table.items()}


class SiteWatch:  # pragma: no cover - needs the local firmware
    """Code hooks on the candidate stores and the constructor: hits and pattern-object hits."""

    def __init__(self, emu, sites: list[int]) -> None:  # noqa: ANN001
        self.emu, self.objects = emu, set()
        self.phase = "boot"
        self.hits = {s: {"hits": 0, "on_pattern_object": 0} for s in sites}
        self.ctor_calls = {"boot": 0, "after_boot": 0}
        for site in sites:
            emu.uc.hook_add(emu._u.UC_HOOK_CODE, self.on_site, begin=site, end=site)
        emu.uc.hook_add(emu._u.UC_HOOK_CODE, self.on_ctor, begin=CONSTRUCTOR, end=CONSTRUCTOR)

    def on_ctor(self, _uc, _a, _s, _u) -> None:  # noqa: ANN001
        self.ctor_calls["boot" if self.phase == "boot" else "after_boot"] += 1

    def on_site(self, _uc, addr, _s, _u) -> None:  # noqa: ANN001
        word = self.emu.read32(addr)
        base = self.emu.reg(store_base_register(word))
        row = self.hits[addr]
        row["hits"] += 1
        if any(o <= base < o + OBJECT_SIZE for o in self.objects):
            row["on_pattern_object"] += 1

    def rows(self) -> dict[str, dict[str, int]]:
        return {hex(a): r for a, r in sorted(self.hits.items())}


def run_trace(ns, raw: bytes) -> dict:  # noqa: ANN001  # pragma: no cover - needs the firmware
    import arm_dsp_capture as dc
    import arm_keydown as kd
    import arm_program_load as apl
    import arm_screens as sc
    import arm_steady as ast
    import arm_sy_voice as sv
    import arm_voice_gate as vg

    session = sc.Session(ns.image, ns.sram)
    session.load_eeproms(apl.build_image(ns.eeprom.read_bytes(), raw, ns.program.stem[:23]))
    kd.install_board_present(session.emu)
    sites = SiteWatch(session.emu, load_sites())
    session.boot()
    emu = session.emu
    rig = kd.Rig(emu, kd.Uart2().install(emu))
    emu.install_dsp_ports()
    dc.install_dsp_ack(emu)
    vg.install_spi_neutral(emu)
    rec = ev.Recorder(emu, ns.tempo)
    watch = FlagWatch(emu, rec)
    sites.objects = set(watch.table.values())
    sites.phase = "run"
    if ns.force_flag is not None:  # counterfactual: what does the flag select?
        for obj in watch.table.values():
            emu.write32(obj + FLAG_OFFSET, ns.force_flag)
    out: dict = {
        "force_flag": ns.force_flag, "program": ns.program.stem, "patches": ns.patch,
        "table": {str(k): hex(v) for k, v in watch.table.items()}, "flags": {}, "grid": {},
    }  # fmt: skip
    out["flags"]["boot"] = watch.flags()
    watch.phase = "load"
    dc.run_phase(rig, dc.LOAD_TICKS, "load")
    out["flags"]["load"] = watch.flags()
    watch.phase = "idle"
    dc.run_phase(rig, dc.IDLE_TICKS, "idle")
    out["flags"]["idle"] = watch.flags()
    out["grid"]["idle"] = watch.grid()
    session.load_state(session.state_bytes())
    keys = [int(k) for k in ns.keys.split(",")]
    session.reset()
    rec.events.clear()
    rec.restart_masters()
    ast.run_ticking(emu, ev.INSNS_PER_TICK * ns.lead, per_tick=ev.INSNS_PER_TICK)
    watch.phase = "keys"
    for key in keys:
        rig.send(kd.note_on(key, ns.velocity))
    t_on = rec.now()
    for tick in range(int(ns.seconds * ev.TICK_HZ)):
        if ns.release_at is not None and tick == int(ns.release_at * ev.TICK_HZ):
            watch.phase = "release"
            for key in keys:
                rig.send(kd.note_off(key))
        ast.run_ticking(emu, ev.INSNS_PER_TICK, per_tick=ev.INSNS_PER_TICK)
        sv.pump(emu)
    out["flags"]["end"] = watch.flags()
    out["grid"]["end"] = watch.grid()
    out["flag_writes"] = watch.flag_writes
    out["store_sites"] = sites.rows()
    out["constructor_calls"] = sites.ctor_calls
    out["offset_pcs"] = {k: sorted(v) for k, v in sorted(watch.offset_pcs.items())}
    run = {"key_on_tick": t_on, "events": list(rec.events)}
    out["layers"] = {name: layer_summary(run, name) for name in "ABC"}
    return out


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - thin CLI
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("image", type=Path)
    ap.add_argument("sram", type=Path)
    ap.add_argument("--eeprom", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--tempo", type=float, default=100.0)
    ap.add_argument("--seconds", type=float, default=1.0)
    ap.add_argument("--velocity", type=int, default=100)
    ap.add_argument("--lead", type=int, default=3)
    ap.add_argument("--release-at", type=float, default=None)
    ap.add_argument("--keys", default="40,43,47,60,64,67", help="held together, comma separated")
    ap.add_argument(
        "--force-flag", type=int, default=None, help="write this to every flag after boot"
    )
    ap.add_argument("--patch", action="append", default=[], help="LAYER.field=value, repeatable")
    ap.add_argument("program", type=Path)
    ns = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    raw = apply_patches(ns.program.read_bytes(), ns.patch)
    ns.out.write_text(json.dumps(run_trace(ns, raw)) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
