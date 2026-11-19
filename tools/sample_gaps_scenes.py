#!/usr/bin/env python3
"""Which scene plays the SAMPLE layers of each program (D154).

The ARM harness recalls a program and plays scene I. A SAMPLE synth layer that only scene II
contains never starts a voice there. This tool reads `scene_1_mask` / `scene_2_mask` of
`ns4 inspect --json` and the sample members of `smp_stream resolve` and counts, per program, the
SAMPLE layers in each scene. Layer bits: Organ A 1, B 2, Piano A 4, B 8, Synth A 16, B 32, C 64
(VERIFIED: `engine::LayerId` order, `Telemetry::active_mask`).

    python tools/sample_gaps_scenes.py --backup B.ns4b --ns4 target/release/ns4.exe \
        --smp-stream target/release/smp_stream.exe --out docs/research/data/sample-gaps-scenes.json
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

SYNTH_BITS = {"synth_a": 16, "synth_b": 32, "synth_c": 64}


def scene_counts(masks: tuple[int, int], resolved: dict[str, object]) -> dict[str, int]:
    """Sample layers in scene I, scene II, both and neither, from the scene masks."""
    s1, s2 = masks
    layers = [bit for k, bit in SYNTH_BITS.items() if resolved.get(k)]
    return {
        "sample_layers": len(layers),
        "scene1": sum(1 for b in layers if s1 & b),
        "scene2": sum(1 for b in layers if s2 & b),
        "scene2_only": sum(1 for b in layers if s2 & b and not s1 & b),
        "neither": sum(1 for b in layers if not (s1 | s2) & b),
    }


def masks_of(inspect_text: str) -> tuple[int, int]:
    """`(scene_1_mask, scene_2_mask)` of the text `ns4 inspect --json` prints."""
    data = json.loads(inspect_text[inspect_text.index("{") :])
    g = data["global"]
    return int(g["scene_1_mask"]), int(g["scene_2_mask"])


def run(ns: argparse.Namespace) -> list[dict[str, object]]:  # pragma: no cover - spawns tools
    cov = json.loads(Path(ns.coverage).read_text(encoding="utf-8"))["programs"]
    rows = []
    for c in cov:
        bank, name = c["program"].split("__", 1)
        entry = f"Program/{bank.replace('_', ' ')}/{name.replace('_', ' ')}.ns4p"
        res = json.loads(
            subprocess.run(
                [ns.smp_stream, "resolve", ns.backup, entry],
                capture_output=True,
                text=True,
                check=True,
            ).stdout
        )
        prog = Path(ns.programs) / bank.replace("_", " ") / f"{name.replace('_', ' ')}.ns4p"
        txt = subprocess.run(
            [ns.ns4, "inspect", str(prog), "--json"], capture_output=True, text=True, check=True
        ).stdout
        rows.append({"program": c["program"], **scene_counts(masks_of(txt), res)})
    return rows


def main(argv: list[str] | None = None) -> int:  # pragma: no cover
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--backup", required=True)
    ap.add_argument("--ns4", default="target/release/ns4.exe")
    ap.add_argument("--smp-stream", default="target/release/smp_stream.exe")
    ap.add_argument("--programs", default="factory_programs/Program")
    ap.add_argument("--coverage", default="docs/research/data/sample-stream-coverage.json")
    ap.add_argument("--out", type=Path, required=True)
    ns = ap.parse_args(argv)
    ns.ns4, ns.smp_stream = str(Path(ns.ns4).resolve()), str(Path(ns.smp_stream).resolve())
    rows = run(ns)
    ns.out.write_text(json.dumps(rows, indent=1) + "\n", encoding="utf-8")
    print(f"wrote {ns.out}: {len(rows)} programs")
    return 0


if __name__ == "__main__":
    sys.exit(main())
