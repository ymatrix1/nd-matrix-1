#!/usr/bin/env python3
"""Which layers of a factory program the firmware lets sound at one key (census expectation).

The chain census (`tools/program_rank_chain.py`) flags an engine as silent when its tap stays 0.
That is only a defect when the firmware itself would sound the engine at that key. This tool
computes the firmware's answer from the decoded program (`ns4 inspect --json`), in four steps:

1. scene I mask: decoded layer enables gated by the section enable, plus the loader rule that an
   enabled section with no layer enabled plays layer A (`engine/routing.rs::default_layer_a`,
   VERIFIED by the program recall of 11 programs, `docs/research/engine-routing.md` s7);
2. layer key range: the `kKbZone` code, the active split points and the crossfades through the
   firmware zone map (`tools/arm_zone_map.py`, VERIFIED 384 of 384 programs, `zone-key-ranges.md`);
3. layer level: a layer whose stored level is 0 and has no morph is silent at rest;
4. arpeggiator: a synth layer with its arpeggiator on plays only from the first arp step (OQ40:
   Gravity Pattern voices after about 2,900 ticks, not within the 1,500 tick census hold).

    python tools/program_expect.py --programs factory_programs/Program --ns4 target/release/ns4 \
        --key 60 --out target/rank/expect.jsonl

Output: one JSON line per program with numbers and flags only.
"""

from __future__ import annotations

import argparse
import json
import subprocess
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import arm_zone_map as zm

NAMES = ("organ_a", "organ_b", "piano_a", "piano_b", "synth_a", "synth_b", "synth_c")
SECTIONS = (  # (section enable bit, layer bits of the 7-bit mask, layer A bit): organ, piano, synth
    (0b100, 0b0000_0011, 0),
    (0b010, 0b0000_1100, 2),
    (0b001, 0b0111_0000, 4),
)
WIDTH = {"Off": 0, "Small": 1, "Large": 2}
ENGINE_OF = {"organ_a": "organ", "organ_b": "organ", "piano_a": "piano", "piano_b": "piano"}


def effective_mask(g: dict) -> int:
    """Scene I layer mask with the loader rule: an enabled empty section plays layer A."""
    mask, enable = g["scene_1_mask"], g["section_enable_i"]
    for bit, layers, layer_a in SECTIONS:
        if enable & bit and not mask & layers:
            mask |= 1 << layer_a
    return mask


def levels(d: dict) -> list[float]:
    """Stored layer level of the seven layers (organ A B, piano A B, synth A B C)."""
    o, p, s = d["organ"], d["piano"], d["synth"]
    return [
        o["layer_a_level"], o["layer_b_level"], p["layer_a"]["level"], p["layer_b"]["level"],
        s["layer_a"]["level"], s["layer_b"]["level"], s["layer_c"]["level"],
    ]  # fmt: skip


def zone_gain(g: dict, layer: int, key: int) -> float:
    """Firmware key gain `0..1` of one layer at `key` (`0xC00C859C`)."""
    flags = tuple(g["split_zone_enabled"])
    pos = tuple(zm.SPLIT_NOTES.index(k) for k in g["split_points"])
    widths = tuple(WIDTH[w] for w in g["split_crossfades"])
    lower, upper = zm.zone_range(g["kb_zone"][layer], g["split_enabled"], flags, pos, widths)
    return zm.note_gain(lower, upper, key) / zm.UNITY


def synth_arp(d: dict, layer: int) -> bool:
    """Whether synth layer `layer` (4..6) has its arpeggiator on."""
    return bool(d["synth"][("layer_a", "layer_b", "layer_c")[layer - 4]]["arp_enabled"])


def expect(d: dict, key: int) -> dict:
    """Per layer in scene I: `on`, `gain` at `key`, `level`; per engine: audible at rest or not."""
    g, lv = d["global"], levels(d)
    mask = effective_mask(g)
    layers = {}
    audible = {"organ": False, "piano": False, "synth": False}
    for i, name in enumerate(NAMES):
        if not mask >> i & 1:
            continue
        gain = zone_gain(g, i, key)
        layers[name] = {"gain": round(gain, 4), "level": round(lv[i], 4)}
        if gain > 0 and lv[i] > 0:
            audible[ENGINE_OF.get(name, "synth")] = True
    return {
        "mask": mask,
        "layers": layers,
        "audible": audible,
        "arp": any(synth_arp(d, i) for i in (4, 5, 6) if mask >> i & 1),
    }


def silent_reason(row: dict, engine: str) -> str:
    """Why an engine of the scene I sections is silent at the key: `zone`, `level` or `none`."""
    names = [n for n in row["layers"] if ENGINE_OF.get(n, "synth") == engine]
    if not names:
        return "no_layer"
    if all(row["layers"][n]["gain"] == 0 for n in names):
        return "zone"
    if all(row["layers"][n]["level"] == 0 or row["layers"][n]["gain"] == 0 for n in names):
        return "level"
    return "audible"


def decode(ns4: Path, program: Path) -> dict:  # pragma: no cover - needs the local binary
    """`ns4 inspect --json` of one program."""
    out = subprocess.run(  # noqa: S603
        [str(ns4), "inspect", str(program), "--json"], capture_output=True, check=True
    )
    return json.loads(out.stdout)


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - thin CLI
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--programs", type=Path, required=True)
    ap.add_argument("--ns4", type=Path, required=True)
    ap.add_argument("--key", type=int, default=60)
    ap.add_argument("--out", type=Path, required=True)
    ns = ap.parse_args(argv)
    files = sorted(ns.programs.glob("Bank */*.ns4p"))

    def one(p: Path) -> str:
        entry = f"Program/{p.parent.name}/{p.stem}.ns4p"
        return json.dumps({"entry": entry, "key": ns.key, **expect(decode(ns.ns4, p), ns.key)})

    with ThreadPoolExecutor(8) as pool:
        lines = list(pool.map(one, files))
    ns.out.parent.mkdir(parents=True, exist_ok=True)
    ns.out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"{len(lines)} programs")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
