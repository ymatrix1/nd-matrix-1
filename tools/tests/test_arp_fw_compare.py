from __future__ import annotations

import arp_fw_compare as c
import pytest

HZ = c.TICK_HZ


def listener(layer: str, ms: float, kind: int, index: int = 5) -> dict:
    return {"ev": "listener", "layer": layer, "t": int(ms * HZ / 1000) + 1000, "type": kind,
            "index": index}  # fmt: skip


def run_with(events: list[dict], keys=(60,)) -> dict:
    return {"keys": list(keys), "key_on_tick": 1000, "events": events, "voice": []}


def test_edges_in_ms_after_the_key_on_and_pseudo_offs_dropped():
    # type 1 record: pseudo off (index 0) then on at the same tick, then the real off 100 ms later
    ev = [
        listener("C", 150, 0, 0), listener("C", 150, 1, 130),
        listener("C", 250, 0, 131), listener("C", 300, 1, 258),
        listener("A", 150, 1, 9),
    ]  # fmt: skip
    ons, offs = c.fw_edges(run_with(ev), "C")
    assert ons == pytest.approx([150.0, 300.0])
    assert offs == pytest.approx([250.0])


def test_summarize_reports_step_gate_and_fraction():
    out = c.summarize([100.0, 250.0, 400.0], [200.0, 350.0])
    assert out["first_on_ms"] == 100.0
    assert out["step_ms"] == 150.0
    assert out["gate_ms"] == 100.0
    assert out["gate_fraction"] == pytest.approx(0.667, abs=1e-3)
    assert c.summarize([], [])["step_ms"] is None


def test_gates_skip_an_on_without_a_later_off():
    assert c.gates([0.0, 100.0, 200.0], [50.0, 160.0]) == [50.0, 60.0]


def test_native_edges_and_notes_follow_the_arp_trace_layout():
    native = {"layers": [{"layer": "A", "events": []}, {"layer": "C", "events": [
        {"ms": 150.0, "on": True, "note": 60}, {"ms": 300.0, "on": False, "note": 60},
        {"ms": 300.0, "on": True, "note": 64}]}]}  # fmt: skip
    assert c.native_edges(native, "C") == ([150.0, 300.0], [300.0])
    assert c.native_notes(native, "C") == [60, 64]


def test_fw_notes_counts_a_note_when_it_enters_the_list():
    run = {"voice": [
        {"obj": "0x1", "notes": []}, {"obj": "0x1", "notes": [60]},
        {"obj": "0x1", "notes": [64]}, {"obj": "0x1", "notes": [64]},
        {"obj": "0x1", "notes": [67]}]}  # fmt: skip
    assert c.fw_notes(run) == [60, 64, 67]


def test_compare_and_run_selection():
    ev = [listener("C", 150, 1), listener("C", 250, 0), listener("C", 300, 1)]
    fw = {"runs": [run_with(ev), run_with(ev, keys=(60, 64))]}
    native = {"layers": [{"layer": "C", "events": [
        {"ms": 150.0, "on": True, "note": 60},
        {"ms": 300.0, "on": True, "note": 60}]}]}  # fmt: skip
    out = c.compare(fw, native, "C", [60])
    assert out["firmware"]["step_ms"] == pytest.approx(150.0, abs=0.1)
    assert out["native"]["step_ms"] == 150.0
    with pytest.raises(KeyError):
        c.fw_run(fw, [61])
