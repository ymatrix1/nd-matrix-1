#!/usr/bin/env python3
"""Transcribed-note replay for ``tools/emu_vs_hw.py`` (``--events-from``).

Segments normally replay the notes detected by ``video_ref.py notes`` at one uniform velocity.
With ``--events-from transkun`` they replay the full-length Transkun note stream
(``tools/transcribe_clip.py``) sliced to the segment: real onsets, offsets, per-note velocities
and the sustain pedal (CC 64). ``--events-from <path>`` names another stream JSON.

A variant that sets its own ``velocity`` (``vel-88``) still means "uniform": its replay copy
drops the per-note velocities so ``--velocity`` applies (the pedal is kept).
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import midi_events
import note_env
import note_offsets

STREAM_DIR = Path(__file__).resolve().parents[1] / "docs" / "research" / "data"
STREAM_NAME = "transkun"


def stream_path(events_from: str, video: str) -> Path:
    """``transkun`` resolves to the committed stream of the spec's video, else a path."""
    if events_from == STREAM_NAME:
        return STREAM_DIR / f"piano_events_{video}.json"
    return Path(events_from)


def segment_events(stream: Path, seg: dict[str, Any], out: Path, merge_gap: float = 0.0) -> Path:
    """Writes the stream sliced to the segment window (times re-based to 0).

    ``merge_gap`` > 0 joins same-key re-triggers closer than that many seconds
    (`note_offsets.merge_retriggers`): a Leslie makes a piano transcriber re-trigger one held
    organ key many times, and the replay then holds a different note set than the player did.
    """
    doc = json.loads(stream.read_text(encoding="utf-8"))
    cut = midi_events.slice_events(doc, float(seg["at_s"]), float(seg["duration_s"]))
    cut.pop("program_ranges", None)
    if merge_gap > 0:
        cut["events"] = note_offsets.merge_retriggers(cut["events"], merge_gap)
    out.write_text(json.dumps(cut), encoding="utf-8")
    return out


def variant_events(events: Path, var: dict[str, Any], out: Path) -> Path:
    """The replay file of a variant: per-note velocities, or none when the variant is uniform."""
    if "velocity" not in var:
        return events
    doc = json.loads(events.read_text(encoding="utf-8"))
    for e in doc["events"]:
        e.pop("velocity", None)
    out.write_text(json.dumps(doc), encoding="utf-8")
    return out


def env_row(hw: tuple[int, Any], emu: tuple[int, Any], events: Path) -> dict[str, Any]:
    """Hardware and emulator hold / release medians and level-vs-velocity slopes.

    ``hw`` and ``emu`` are ``(sample_rate, mono)``; velocities come from the replay file, so a
    uniform-velocity variant reports no slope (``None``).
    """
    notes = json.loads(events.read_text(encoding="utf-8"))["events"]
    row: dict[str, Any] = {}
    for tag, (sr, mono) in (("hw", hw), ("emu", emu)):
        m = note_env.envelope_metrics(mono, sr, notes)
        row[f"hold_{tag}_ms"] = m["hold_ms"]
        row[f"release_{tag}_ms"] = m["release_ms"]
        slope, n = note_env.velocity_slope_db(mono, sr, notes)
        row[f"vel_slope_{tag}_db"] = slope
        row["vel_slope_notes"] = n
    return row
