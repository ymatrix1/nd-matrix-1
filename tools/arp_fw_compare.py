#!/usr/bin/env python3
"""Arpeggiator note events of the ARM firmware against the native engine (numbers only).

Inputs: `tools/arm_arp_events.py` JSON (firmware: listener events in 24 kHz ticks, voice lists) and
`arp_trace` JSON (`cargo run --release -p nord-tools --bin arp_trace`, native: voice gate edges in
ms). For one layer and key set it reports

* `first_on_ms`   key-on to the first note of the arpeggio,
* `step_ms`       median and spread of the distance between two note-ons,
* `gate_ms`       median note-on to note-off distance, and `gate_fraction` of the step,
* `notes`         the note numbers in playing order.

Firmware step time is an input of the capture (the master position is fed at a tempo, see
`arm_arp_events.py`), so `step_ms` of the firmware equals the fed tempo by construction; the
gate fraction, the note order and the first-note latency are the firmware's own.

    python tools/arp_fw_compare.py FW.json NATIVE.json --layer C --keys 60,64,67 --out cmp.json
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from pathlib import Path

TICK_HZ = 24_000


def fw_run(fw: dict, keys: list[int], release: bool = False) -> dict:
    """The run of `fw` that played `keys` (`release`: the one with a note-off, if several)."""
    runs = [r for r in fw["runs"] if r["keys"] == keys]
    if not runs:
        raise KeyError(f"no firmware run for keys {keys}")
    return runs[-1] if release else runs[0]


def fw_edges(run: dict, layer: str) -> tuple[list[float], list[float]]:
    """`(on times, off times)` in ms after the key-on of the listener events of `layer`.

    The firmware sends a pseudo note-off (type 0, index 0) just before an on of a type 1 record;
    those carry no gate edge of their own and are dropped when an on follows at the same tick.
    """
    t0 = run["key_on_tick"]
    ons: list[float] = []
    offs: list[float] = []
    events = [e for e in run["events"] if e["ev"] == "listener" and e["layer"] == layer]
    for i, e in enumerate(events):
        ms = (e["t"] - t0) * 1000.0 / TICK_HZ
        if ms < 0:
            continue
        if e["type"] == 1:
            ons.append(ms)
        elif e["index"] != 0 or not any(
            n["type"] == 1 and n["t"] == e["t"] for n in events[i + 1 : i + 3]
        ):
            offs.append(ms)
    return ons, offs


def native_edges(native: dict, layer: str) -> tuple[list[float], list[float]]:
    """`(on times, off times)` in ms of one layer of an `arp_trace` document."""
    ev = next(layer_ for layer_ in native["layers"] if layer_["layer"] == layer)["events"]
    return [e["ms"] for e in ev if e["on"]], [e["ms"] for e in ev if not e["on"]]


def gates(ons: list[float], offs: list[float]) -> list[float]:
    """Distance from each on to the first off after it (ms); an on without an off is skipped."""
    out = []
    for t in ons:
        later = [o for o in offs if o > t]
        if later:
            out.append(later[0] - t)
    return out


def summarize(ons: list[float], offs: list[float]) -> dict:
    """Latency, step and gate statistics of one edge list."""
    steps = [b - a for a, b in zip(ons, ons[1:], strict=False)]
    gate = gates(ons, offs)
    step_med = statistics.median(steps) if steps else None
    gate_med = statistics.median(gate) if gate else None
    return {
        "notes_on": len(ons),
        "first_on_ms": round(ons[0], 2) if ons else None,
        "step_ms": round(step_med, 2) if step_med is not None else None,
        "step_spread_ms": round(max(steps) - min(steps), 2) if steps else None,
        "gate_ms": round(gate_med, 2) if gate_med is not None else None,
        "gate_fraction": round(gate_med / step_med, 3) if gate_med and step_med else None,
    }


def fw_notes(run: dict) -> list[int]:
    """Note numbers in playing order from the voice-list snapshots (a note counts when it
    appears in the list after not being there at the previous snapshot of the same object)."""
    seen: dict[str, list[int]] = {}
    out: list[int] = []
    for snap in run["voice"]:
        before = seen.get(snap["obj"], [])
        for note in snap["notes"]:
            if note not in before:
                out.append(note)
        seen[snap["obj"]] = snap["notes"]
    return out


def native_notes(native: dict, layer: str) -> list[int]:
    ev = next(layer_ for layer_ in native["layers"] if layer_["layer"] == layer)["events"]
    return [e["note"] for e in ev if e["on"]]


def compare(fw: dict, native: dict, layer: str, keys: list[int]) -> dict:
    """Both summaries and the note sequences of one layer and key set."""
    run = fw_run(fw, keys)
    return {
        "layer": layer,
        "keys": keys,
        "firmware": summarize(*fw_edges(run, layer)),
        "native": summarize(*native_edges(native, layer)),
        "firmware_notes": fw_notes(run)[:16],
        "native_notes": native_notes(native, layer)[:16],
    }


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("firmware", type=Path)
    ap.add_argument("native", type=Path)
    ap.add_argument("--layer", required=True, choices=("A", "B", "C"))
    ap.add_argument("--keys", required=True, help="comma separated, as captured")
    ap.add_argument("--out", type=Path)
    ns = ap.parse_args(argv)
    result = compare(
        json.loads(ns.firmware.read_text(encoding="utf-8")),
        json.loads(ns.native.read_text(encoding="utf-8")),
        ns.layer,
        [int(k) for k in ns.keys.split(",")],
    )
    text = json.dumps(result, indent=1)
    if ns.out:
        ns.out.write_text(text + "\n", encoding="utf-8")
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
