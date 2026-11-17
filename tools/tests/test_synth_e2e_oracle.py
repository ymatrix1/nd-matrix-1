"""Unit tests for tools/synth_e2e_oracle.py (scripts, sample extraction, reports, CSV rows)."""

from __future__ import annotations

import numpy as np
import pytest
import synth_e2e_oracle as o


def row(phase: str, chip: str, core: int, dm: int, w2: int) -> dict:
    return {
        "phase": phase,
        "tick": 0,
        "chip": chip,
        "path": 2,
        "core": core,
        "dm": dm,
        "w2": w2,
        "pc": 0,
    }


ROWS = [
    row("load", "ORG", 2, 0xE0000, 0x84121C54),
    row("idle", "ORG", 2, 0xE0001, 0x8612549D),
    row("load", "ORG", 1, 0xE0002, 7),
    row("load", "PNO", 2, 0xE0003, 8),
    row("k60", "PNO", 2, 0xC1DCD, 0x0B00E280),
    row("k60", "PNO", 2, 0xC1DCC, 0x0B00E221),
    row("k60_off", "PNO", 2, 0xC1DCC, 0x0800E221),
]


def test_load_script_keeps_only_org_core2_recall_in_order() -> None:
    assert o.core2_load_script(ROWS) == "0 0xe0000 0x84121c54\n0 0xe0001 0x8612549d\n"


def test_key_script_delivers_every_word_in_capture_order() -> None:
    assert o.key_script(ROWS, 300, 900).splitlines() == [
        "300 0xc1dcd 0x0b00e280",
        "300 0xc1dcc 0x0b00e221",
        "900 0xc1dcc 0x0800e221",
    ]
    with pytest.raises(ValueError):
        o.key_script(ROWS, 1, 2, key=72)


def test_null_b_script_pokes_every_pass() -> None:
    assert o.null_b_script(3).splitlines() == [f"{p} 0xc0b8c 0x125b79" for p in range(3)]


def test_voice_samples_takes_the_pex_lane_four_per_pass() -> None:
    f = np.arange(16, dtype="<f4").reshape(2, 8)
    got = o.voice_samples(f.view("<u4"), 2)
    assert list(got) == [0, 2, 4, 6, 8, 10, 12, 14]


def test_rates_follow_the_assumed_pass_length() -> None:
    assert o.ORACLE_FS == pytest.approx(86_804.6875)
    assert o.PASS_S == pytest.approx(46.08e-6, rel=1e-3)


def test_steady_slice_is_inside_the_hold() -> None:
    x = np.arange(48_000, dtype=float)
    s = o.steady_slice(x, 48_000.0, 0.0, 0.5)
    assert s[0] == 7_200 and s[-1] == 21_599


def test_stage_report_of_identical_tones_has_zero_level_delta_and_matching_pitch() -> None:
    n = int(1.0 * o.NATIVE_FS)
    nat = 0.3 * np.sin(2 * np.pi * 440.0 * np.arange(n) / o.NATIVE_FS)
    m = int(1.0 * o.ORACLE_FS)
    orc = 0.3 * np.sin(2 * np.pi * 440.0 * np.arange(m) / o.ORACLE_FS)
    r = o.stage_report(orc, nat, 0.0, 0.6)
    assert abs(r["level_delta_db"]) < 0.1
    assert r["f0_oracle_hz_at_assumed_rate"] == pytest.approx(440.0, abs=1.0)
    assert r["f0_native_hz"] == pytest.approx(440.0, abs=1.0)
    assert r["band_delta_db_mean_abs"] < 1.0


def test_envelope_facts_read_the_ramp_slope_and_release_constant() -> None:
    y = np.zeros(2_000)
    y[300:] = np.minimum(1.0, 1e-3 * np.arange(1_700))
    y[1_000:] = 0.5 * 0.99 ** np.arange(1_000)
    f = o.envelope_facts(y, 300, 1_000)
    assert f["attack_passes_per_unit"] == pytest.approx(1_000.0, rel=1e-3)
    assert f["release_tau_s"] == pytest.approx(-1.0 / np.log(0.99) * o.PASS_S, rel=1e-3)
    assert o.envelope_facts(np.zeros(2_000), 300, 1_000)["attack_passes_per_unit"] == float("inf")


def test_out_gain_and_csv_rows() -> None:
    a = np.full(48_000, 0.2)
    assert o.out_gain(a, a * 2, 0.0) == pytest.approx(-6.02, abs=0.01)
    keys = (
        "oracle_rms_db native_rms_db level_delta_db f0_oracle_hz_at_assumed_rate f0_native_hz "
        "t90_oracle_ms t90_native_ms band_delta_db_mean_abs spectrum_null_db env_null_db"
    ).split()
    stage = {k: 1.0 for k in keys}
    doc = {
        "program": "X",
        "oscillator_path": stage,
        "filter_out": stage,
        "oracle_filter_gain_db": 2.0,
        "native_filter_gain_db": 0.5,
    }
    rows = o.csv_rows(doc, "default")
    assert [r["stage"] for r in rows] == ["oscillator_path", "filter_out"]
    assert rows[0]["program"] == "X" and rows[1]["native_filter_gain_db"] == 0.5
    assert set(rows[0]) == set(o.CSV_FIELDS)
