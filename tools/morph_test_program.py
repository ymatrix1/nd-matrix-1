#!/usr/bin/env python3
"""A program with a distinct morph depth on every target, for `arm_program_morph.py` (ns4-kq1o).

Decodes a factory `.ns4p` with `ns4 convert --to-json`, sets the three morph depths of every
Synth layer target, layer level, FX slot and organ drawbar to known raw steps (see `DEPTHS`), and
packs it back with `ns4 convert --to-ns4p`. The output is a modified copy of a factory program:
keep it under `target/` (never commit it).

    python tools/morph_test_program.py --ns4 target/release/ns4 \
        --program-file horizon.ns4p --out target/morph_test.ns4p
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

# Raw steps `(wheel, aftertouch, pedal)` of a 7-bit morph (`delta = step / 127`).
DEPTHS = {
    "synth_a": (10, 20, -30),
    "synth_b": (50, 100, -100),
    "synth_c": (5, -15, 25),
    "fx": (12, 24, -36),
    "piano_a_level": (127, 64, -64),
    "piano_b_level": (20, 30, 40),
    "organ_a_level": (20, 30, -40),
    "organ_b_level": (10, -20, 30),
    "drawbar": (32, -32, 64),  # 0.25 / -0.25 / 0.5 of the 4-bit drawbar range
}
SYNTH_TARGETS = (
    "level_morph",
    "osc_ctrl_morph",
    "filter_cutoff_morph",
    "filter_resonance_morph",
    "lfo_rate_morph",
    "lfo_amount_morph",
    "filter_env_amount_morph",
)


def set_depth(morph: dict, steps: tuple[int, int, int]) -> None:
    """Writes raw steps into a decoded `MorphDeltas` dict (`wheel`, `aftertouch`, `ctrl_pedal`)."""
    morph["wheel"], morph["aftertouch"], morph["ctrl_pedal"] = (s / 127.0 for s in steps)


def apply_test_morphs(program: dict) -> dict:
    """Sets every depth of `DEPTHS` in a decoded program dict (in place); returns it."""
    synth = program["synth"]
    for layer, key in (("layer_a", "synth_a"), ("layer_b", "synth_b"), ("layer_c", "synth_c")):
        for target in SYNTH_TARGETS:
            set_depth(synth[layer][target], DEPTHS[key])
    sections = [
        synth["fx_a"],
        synth["fx_b"],
        synth["fx_c"],
        program["piano"]["fx_a"],
        program["piano"]["fx_b"],
        program["organ"]["fx"],
    ]
    for fx in sections:
        for name, morph in fx.items():
            if name.endswith("_morph"):
                set_depth(morph, DEPTHS["fx"])
    piano = program["piano"]
    set_depth(piano["layer_a"]["level_morph"], DEPTHS["piano_a_level"])
    set_depth(piano["layer_b"]["level_morph"], DEPTHS["piano_b_level"])
    organ = program["organ"]
    set_depth(organ["layer_a_level_morph"], DEPTHS["organ_a_level"])
    set_depth(organ["layer_b_level_morph"], DEPTHS["organ_b_level"])
    for key in ("drawbars_a_morph", "drawbars_b_morph"):
        for morph in organ[key]:
            set_depth(morph, DEPTHS["drawbar"])
    return program


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the ns4 binary
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--ns4", type=Path, required=True, help="the `ns4` binary")
    ap.add_argument("--program-file", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ns = ap.parse_args(argv)
    with tempfile.TemporaryDirectory() as tmp:
        raw = Path(tmp) / "program.json"
        subprocess.run(
            [ns.ns4, "convert", ns.program_file, "--to-json", raw], check=True, capture_output=True
        )
        program = apply_test_morphs(json.loads(raw.read_text(encoding="utf-8")))
        raw.write_text(json.dumps(program), encoding="utf-8")
        subprocess.run(
            [ns.ns4, "convert", raw, "--to-ns4p", ns.out], check=True, capture_output=True
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
