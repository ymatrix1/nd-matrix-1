"""Tests for tools/smp_pitch_audit.py (D230)."""

import smp_pitch_audit as audit


def row(entry: str, cycles: str) -> dict:
    return {"entry": entry, "smp_cycles": cycles}


def test_active_octave_takes_the_first_enabled_layer_else_layer_a() -> None:
    a = {"enabled": True, "octave": 1}
    b = {"enabled": True, "octave": -1}
    off = {"enabled": False, "octave": 0}
    assert audit.active_octave({"piano": {"layer_a": a, "layer_b": b}}) == 1
    assert audit.active_octave({"piano": {"layer_a": off, "layer_b": b}}) == -1
    assert audit.active_octave({"piano": {"layer_a": {**off, "octave": 2}, "layer_b": off}}) == 2


def test_audit_separates_ok_octave_shift_mismatch_and_unknown() -> None:
    octaves = {"a": 0, "b": 1, "c": -1, "d": 0, "e": 0, "f": None}
    rows = [
        row("a", "0.00601"),  # octave 0, measured 1x: ok
        row("b", "0.01213"),  # octave +1, measured 2x: a real shift
        row("c", "0.00301"),  # octave -1, measured 0.5x
        row("d", "0.00302"),  # octave 0 but half: a sub-octave detector fault
        row("e", "0.00031"),  # the old detector floor
        row("f", "0.00603"),
        {"entry": "g", "smp_cycles": ""},  # no tap: skipped
    ]
    got = {r["entry"]: r["verdict"] for r in audit.audit(rows, octaves.get)}
    assert got == {
        "a": "ok",
        "b": "octave_shift",
        "c": "octave_shift",
        "d": "mismatch",
        "e": "mismatch",
        "f": "unknown",
    }


def test_audit_reports_the_measured_octaves() -> None:
    (r,) = audit.audit([row("x", "0.01206")], lambda _e: 1)
    assert r["octaves"] == 1.0 and r["expected"] == 1 and r["cycles"] == 0.01206
