"""Coverage of the 119 SAMPLE programs by the SY voice start (D150).

Reads the `resolve` JSON of `smp_stream` (layer members per program) and the
`arm_dsp_capture one --member` captures (`cap/<stem>/*.txt.gz`, phases `k36 / k60 / k84`) and
classifies every program by what stops its sample audio. Numbers only: no sample data.

Provenance (`.agents/rules/provenance-in-code.md`): the prepare state word is `VERIFIED`
(`sy-smp-audio` s3); that a prepare in some key phase means "voice queued" is `EMPIRICAL`.
"""

from __future__ import annotations

import argparse
import json
from collections import Counter
from collections.abc import Iterable
from pathlib import Path

STATE_ADDR = 0x92414
STATE_PREPARE = 0x1C07CD
PHASES = ("k36", "k60", "k84")


def layer_count(resolved: dict[str, object]) -> int:
    """Number of sample layers (`synth_a/b/c` members) of a resolved program."""
    return sum(1 for k in ("synth_a", "synth_b", "synth_c") if resolved.get(k))


def prepared_phases(rows: Iterable[dict[str, object]]) -> list[str]:
    """Key phases whose SY messages contain a voice `prepare` (`0x92414 <- 0x1c07cd`)."""
    seen = {
        str(r["phase"])
        for r in rows
        if r["chip"] == "SY"
        and r["path"] == 1
        and r["dm"] == STATE_ADDR
        and r["w2"] == STATE_PREPARE
    }
    return [p for p in PHASES if p in seen]


def classify(resolved: dict[str, object], phases: list[str] | None) -> str:
    """Blocker class of one program (`phases` is `None` when no capture exists)."""
    if layer_count(resolved) == 0:
        return "no sample member resolved"
    if phases is None:
        return "no capture"
    if "k60" in phases:
        return "voices at key 60"
    if phases:
        return "voices only at other keys"
    return "no voice queued"


def summarize(rows: list[dict[str, object]]) -> dict[str, object]:
    """Counts per class and per layer count over `program_row`-like dicts."""
    return {
        "programs": len(rows),
        "classes": dict(Counter(str(r["class"]) for r in rows)),
        "layers": dict(Counter(int(r["layers"]) for r in rows)),  # type: ignore[call-overload]
    }


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - reads local captures
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("resolve", type=Path)
    ap.add_argument("cap_dir", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    ns = ap.parse_args(argv)
    import arm_dsp_capture as ac

    resolved = json.loads(ns.resolve.read_text(encoding="utf-8"))
    out: list[dict[str, object]] = []
    for stem, info in sorted(resolved.items()):
        gz = next((ns.cap_dir / stem).glob("*.txt.gz"), None)
        phases = prepared_phases(ac.read_rows(gz)) if gz else None
        out.append(
            {
                "program": stem,
                "layers": layer_count(info),
                "phases": phases,
                "class": classify(info, phases),
            }
        )
    ns.out.write_text(
        json.dumps({"summary": summarize(out), "programs": out}, indent=1), encoding="utf-8"
    )
    print(json.dumps(summarize(out), indent=1))
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
