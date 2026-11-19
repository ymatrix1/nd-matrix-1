"""Voices per key phase and layers lost, from full-member captures (D154).

Reads `sample_gaps_cap.py` captures (`cap/<stem>/*.txt.gz`, phases `k36 / k60 / k84`) and the
scene I layer counts of `docs/research/data/sample-gaps-scenes.json` (the scene the harness plays).
For each program it counts the SY voice records that reach the prepare state in each key phase.
Numbers only: no sample data.

Provenance (`.agents/rules/provenance-in-code.md`): the voice record stride `0x38` words and the
state word offset `0x14` are `VERIFIED` (`sy-smp-audio` s3, same layout as
`sample_stream_coverage`);
that a prepare means "voice queued" is `EMPIRICAL`.
"""

from __future__ import annotations

import argparse
import json
from collections import Counter
from collections.abc import Iterable
from pathlib import Path

RECORD_BASE = 0x92400
RECORD_STRIDE = 0x38
STATE_OFFSET = 0x14
STATE_PREPARE = 0x1C07CD
PHASES = ("k36", "k60", "k84")


def voice_of(dm: int) -> int | None:
    """Voice index of a state word address, `None` when `dm` is not a state word."""
    off = dm - RECORD_BASE - STATE_OFFSET
    if off < 0 or off % RECORD_STRIDE:
        return None
    return off // RECORD_STRIDE


def prepared_voices(rows: Iterable[dict[str, object]]) -> dict[str, list[int]]:
    """Voice indices that reach `prepare` per key phase."""
    seen: dict[str, set[int]] = {p: set() for p in PHASES}
    for r in rows:
        if r["chip"] != "SY" or r["path"] != 1 or r["w2"] != STATE_PREPARE:
            continue
        v = voice_of(int(r["dm"]))  # type: ignore[call-overload]
        if v is not None and str(r["phase"]) in seen:
            seen[str(r["phase"])].add(v)
    return {p: sorted(s) for p, s in seen.items()}


def classify(layers: int, voices: dict[str, list[int]]) -> str:
    """Class of a program: audio at k60 with all layers / some layers, other keys only, none."""
    at60 = len(voices.get("k60", []))
    if layers == 0:
        return "no scene I sample layer"
    if at60 >= layers:
        return "all layers at key 60"
    if at60:
        return "some layers at key 60"
    if any(voices.get(p) for p in PHASES):
        return "voices only at other keys"
    return "no voice queued"


def summarize(rows: list[dict[str, object]]) -> dict[str, object]:
    """Counts per class and per layer count."""
    return {
        "programs": len(rows),
        "classes": dict(Counter(str(r["class"]) for r in rows)),
        "layers": dict(Counter(int(r["layers"]) for r in rows)),  # type: ignore[call-overload]
    }


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - reads local captures
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("cap_dir", type=Path)
    ap.add_argument(
        "--scenes", type=Path, default=Path("docs/research/data/sample-gaps-scenes.json")
    )
    ap.add_argument("--out", type=Path, required=True)
    ns = ap.parse_args(argv)
    import arm_dsp_capture as ac

    scenes = json.loads(ns.scenes.read_text(encoding="utf-8"))
    progs = scenes
    out: list[dict[str, object]] = []
    for p in progs:
        stem = str(p["program"])
        gz = next((ns.cap_dir / stem).glob("*.txt.gz"), None)
        if gz is None:
            continue
        voices = prepared_voices(ac.read_rows(gz))
        layers = int(p["scene1"])
        out.append(
            {"program": stem, "layers": layers, "voices": voices, "class": classify(layers, voices)}
        )
    ns.out.write_text(
        json.dumps({"summary": summarize(out), "programs": out}, indent=1), encoding="utf-8"
    )
    print(json.dumps(summarize(out), indent=1))
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
