"""Tests for tools/emu_events.py and the transcribe_clip.py helpers that need no torch."""

from __future__ import annotations

import json
import sys
import wave
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import emu_events as ev  # noqa: E402
import transcribe_clip as tc  # noqa: E402

STREAM = {
    "source": "transkun-2.0",
    "events": [
        {"midi": 60, "onset_s": 9.0, "offset_s": 9.5, "velocity": 40},
        {"midi": 64, "onset_s": 12.0, "offset_s": 14.0, "velocity": 90},
        {"midi": 67, "onset_s": 40.0, "offset_s": 41.0, "velocity": 70},
    ],
    "pedal": [{"t": 10.0, "value": 127}, {"t": 13.0, "value": 0}],
    "program_ranges": [],
}


def test_stream_path():
    assert ev.stream_path("transkun", "abc").name == "piano_events_abc.json"
    assert ev.stream_path("x/y.json", "abc") == Path("x/y.json")


def test_segment_events_slices_and_rebases(tmp_path):
    src = tmp_path / "s.json"
    src.write_text(json.dumps(STREAM))
    out = ev.segment_events(src, {"at_s": 11.0, "duration_s": 10.0}, tmp_path / "o.json")
    doc = json.loads(out.read_text())
    assert [(e["midi"], e["onset_s"], e["velocity"]) for e in doc["events"]] == [(64, 1.0, 90)]
    assert doc["pedal"] == [{"t": 0.0, "value": 127}, {"t": 2.0, "value": 0}]
    assert "program_ranges" not in doc


def test_variant_events_uniform_drops_velocity(tmp_path):
    src = tmp_path / "e.json"
    src.write_text(json.dumps(STREAM))
    assert ev.variant_events(src, {"label": "default"}, tmp_path / "u.json") == src
    out = ev.variant_events(src, {"label": "vel-88", "velocity": 88}, tmp_path / "u.json")
    doc = json.loads(out.read_text())
    assert all("velocity" not in e for e in doc["events"]) and len(doc["pedal"]) == 2


def test_env_row_keys(tmp_path):
    sr = 8000
    t = np.arange(sr * 4) / sr
    x = np.sin(2 * np.pi * 440 * t) * np.exp(-3 * t)
    notes = {"events": [{"midi": 69, "onset_s": 0.5, "offset_s": 1.5, "velocity": 80}]}
    p = tmp_path / "n.json"
    p.write_text(json.dumps(notes))
    row = ev.env_row((sr, x), (sr, x), p)
    assert {"hold_hw_ms", "release_emu_ms", "vel_slope_hw_db"} <= set(row)


def test_wav_roundtrip_and_window_outputs(tmp_path):
    sr = tc.SAMPLE_RATE
    t = np.arange(sr * 2) / sr
    pcm = (0.3 * np.sin(2 * np.pi * 261.63 * t) * 32767).astype("<i2")
    wav = tmp_path / "a.wav"
    with wave.open(str(wav), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes(pcm.tobytes())
    rate, mono = tc.read_wav_mono(wav)
    assert rate == sr and abs(float(mono.max()) - 0.3) < 0.01
    rows = [
        {
            "t": 5.0,
            "program": "A:61",
            "class": "piano",
            "rms_dbfs": -20.0,
            "notes": 2,
            "supported": None,
            "piano_like": True,
        }
    ]
    tc.write_windows_csv(rows, tmp_path / "w.csv")
    assert (tmp_path / "w.csv").read_text().splitlines()[1] == "5.0,A:61,piano,-20.0,2,,1"


def test_build_outputs_and_parser(tmp_path):
    import test_midi_events as tme

    mid = tmp_path / "a.mid"
    mid.write_bytes(tme.fixture())
    sr = tc.SAMPLE_RATE
    t = np.arange(sr * 2) / sr
    pcm = (0.3 * np.sin(2 * np.pi * 261.63 * t) * 32767).astype("<i2")
    wav = tmp_path / "a.wav"
    with wave.open(str(wav), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes(pcm.tobytes())
    progs = [{"first_seen_s": 0, "program": "A:61", "name": "Velvet Grand"}]
    doc, rows = tc.build_outputs(mid, wav, "t", 100.0, progs)
    assert doc["events"][0]["onset_s"] == 100.0 and "snr_db" in doc["events"][0]
    assert rows[0]["t"] == 100.0 and rows[0]["program"] == "A:61"
    args = tc.build_parser().parse_args(
        ["slice", "e.json", "--at", "1", "--duration", "2", "--out", "o"]
    )
    assert args.fn is tc.cmd_slice


def test_segment_events_merge_gap_joins_retriggers(tmp_path):
    doc = {
        "events": [
            {"midi": 60, "onset_s": 1.0, "offset_s": 1.2, "velocity": 50},
            {"midi": 60, "onset_s": 1.4, "offset_s": 1.9, "velocity": 70},
            {"midi": 60, "onset_s": 3.0, "offset_s": 3.5, "velocity": 60},
            {"midi": 64, "onset_s": 1.1, "offset_s": 1.3, "velocity": 80},
        ],
        "pedal": [],
    }
    src = tmp_path / "s.json"
    src.write_text(json.dumps(doc))
    seg = {"at_s": 0.0, "duration_s": 10.0}
    plain = json.loads(ev.segment_events(src, seg, tmp_path / "a.json").read_text())
    assert len(plain["events"]) == 4
    merged = json.loads(ev.segment_events(src, seg, tmp_path / "b.json", 0.3).read_text())
    got = sorted((e["midi"], e["onset_s"], e["offset_s"]) for e in merged["events"])
    assert got == [(60, 1.0, 1.9), (60, 3.0, 3.5), (64, 1.1, 1.3)]
    # the first velocity is kept
    assert [e["velocity"] for e in merged["events"] if e["midi"] == 60][0] == 50
