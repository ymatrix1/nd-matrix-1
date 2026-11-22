#!/usr/bin/env python3
"""Which morph targets do the factory programs use? (task: uncaptured morph targets).

Reads the raw program payload (`48`-byte `CBIN` header, then global 45 B, organ 135 B, piano 147 B,
synth 493 B) of every `Program/**.ns4p` of a backup archive and counts programs with a non-zero
wheel / aftertouch / pedal depth on: organ drawbars (`Edt_Org_Preset_kDb1..9`), FX delay rate
(14 bit), synth arpeggiator rate and range, synth oscillator envelope amount. Offsets are the
decoder's
(`cbin/organ_preset.rs`, `cbin/fx_preset.rs`, `cbin/synth_preset.rs`, `cbin/synth_arp.rs`); a morph
field is `base (W bits)` then three `(W + 1)`-bit words that store `delta + max_val`.

    python tools/morph_usage_survey.py ".reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b"

Findings: `docs/research/morph-fw-vs-native.md`. Only counts are printed.
"""

from __future__ import annotations

import argparse
import sys
import zipfile
from pathlib import Path

HEADER = 48
# Section slices of the 820-byte program payload (after the header): global, organ, piano, synth.
ORGAN = (45, 180)
PIANO = (180, 327)
SYNTH = (327, 820)
# `cbin/fx_preset.rs`: delay rate at bit 242 of a 55-byte FX block, 14 bit, max 16383.
FX_BLOCK_LEN = 55
FX_DELAY_RATE = (242, 14, 16383)
# FX block offsets inside each section payload (`organ_preset.rs`, `piano_preset.rs`,
# `synth_preset.rs`).
FX_BLOCKS = {"organ": [80], "piano": [37, 92], "synth": [328, 383, 438]}
# `organ_preset.rs`: layer blocks at 144 / 392, drawbar `i` at `+40 + 19 i`, width 4, max 8.
ORGAN_LAYER_BASES = (144, 392)
# Leslie speed: 2-bit code at organ bit 81 followed by three 3-bit morph words (bits 83..91 read
# `011` = 3 = zero depth in every factory program; EMPIRICAL layout,
# `Edt_OrgCom_kEffectsLeslieSpeed`).
LESLIE_SPEED = (81, 2, 3)
DRAWBAR_COUNT = 9
# `synth_preset.rs`: layer blocks at 248, 656, 1064 (stride 408); arp rate `+175`, range `+140`.
SYNTH_LAYER_BASES = (248, 656, 1064)
# `0x2F` sound blocks at 1472, 1856, 2240; osc env amount `+92`, 7 bit signed base, three 8 bit
# depth words that store `depth + 120` (`cbin-and-presets.md` s5; checked against the ARM cells of
# FM Harpies: layer A `(2, 0, 0)`, layer C `(10, 0, 0)`).
SOUND_BASES = (1472, 1856, 2240)
OSC_ENV_AMOUNT = (92, 7, 120)
ARP_RATE = (175, 7, 127)
ARP_RANGE = (140, 7, 127)
KINDS = ("wheel", "at", "pedal")
TARGETS = ("drawbar", "leslie_speed", "delay_rate", "arp_rate", "arp_range", "osc_env_amount")


def bits_at(data: bytes, pos: int, width: int) -> int:
    """`width` bits of `data` starting at bit `pos`, MSB first."""
    value = 0
    for i in range(pos, pos + width):
        value = (value << 1) | ((data[i >> 3] >> (7 - (i & 7))) & 1)
    return value


def morph_depths(data: bytes, pos: int, width: int, max_val: int) -> tuple[int, int, int]:
    """Raw `(wheel, at, pedal)` depths (steps, `stored - max_val`) of the morph field at `pos`."""
    return tuple(
        bits_at(data, pos + width + k * (width + 1), width + 1) - max_val for k in range(3)
    )  # type: ignore[return-value]


def used(depths: tuple[int, int, int]) -> list[str]:
    """Source names with a non-zero depth."""
    return [k for k, d in zip(KINDS, depths, strict=True) if d != 0]


def survey_program(payload: bytes) -> dict[str, list[str]]:
    """`{target: [source, ...]}` of one 820-byte program payload (only non-empty targets)."""
    out: dict[str, list[str]] = {}

    def add(key: str, depths: tuple[int, int, int]) -> None:
        sources = used(depths)
        if sources:
            out.setdefault(key, [])
            out[key] = sorted(set(out[key]) | set(sources))

    organ = payload[slice(*ORGAN)]
    for base in ORGAN_LAYER_BASES:
        for i in range(DRAWBAR_COUNT):
            add("drawbar", morph_depths(organ, base + 40 + 19 * i, 4, 8))
    add("leslie_speed", morph_depths(organ, *LESLIE_SPEED))
    synth = payload[slice(*SYNTH)]
    for base in SYNTH_LAYER_BASES:
        add("arp_rate", morph_depths(synth, base + ARP_RATE[0], ARP_RATE[1], ARP_RATE[2]))
        add("arp_range", morph_depths(synth, base + ARP_RANGE[0], ARP_RANGE[1], ARP_RANGE[2]))
    for base in SOUND_BASES:
        add(
            "osc_env_amount",
            morph_depths(synth, base + OSC_ENV_AMOUNT[0], OSC_ENV_AMOUNT[1], OSC_ENV_AMOUNT[2]),
        )
    sections = {"organ": organ, "piano": payload[slice(*PIANO)], "synth": synth}
    for name, blocks in FX_BLOCKS.items():
        for off in blocks:
            fx = sections[name][off : off + FX_BLOCK_LEN]
            add("delay_rate", morph_depths(fx, *FX_DELAY_RATE))
    return out


def survey_archive(path: Path) -> dict[str, dict[str, list[str]]]:
    """`{program member: survey}` for every program of an `.ns4b` backup."""
    out: dict[str, dict[str, list[str]]] = {}
    with zipfile.ZipFile(path) as archive:
        for member in sorted(archive.namelist()):
            if member.startswith("Program/") and member.endswith(".ns4p"):
                raw = archive.read(member)
                out[member] = survey_program(raw[HEADER : HEADER + 820])
    return out


def summary(rows: dict[str, dict[str, list[str]]]) -> list[str]:
    """Count lines per target and per source."""
    lines = [f"{len(rows)} programs"]
    for target in (
        "drawbar",
        "leslie_speed",
        "delay_rate",
        "arp_rate",
        "arp_range",
        "osc_env_amount",
    ):
        hit = [m for m, r in rows.items() if target in r]
        per_source = {k: sum(1 for m in hit if k in rows[m][target]) for k in KINDS}
        lines.append(f"{target:10s} {len(hit):3d} programs  {per_source}")
    return lines


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local backup
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("backup", type=Path)
    ap.add_argument("--list", action="store_true", help="print the programs per target")
    ns = ap.parse_args(argv)
    rows = survey_archive(ns.backup)
    print("\n".join(summary(rows)))
    if ns.list:
        for member, row in rows.items():
            if row:
                print(member, row)
    return 0


if __name__ == "__main__":
    sys.exit(main())
