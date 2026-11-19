"""Unit tests for tools/morph_law_report.py (pure helpers)."""

from __future__ import annotations

import morph_law_report as ml
import pytest


def series(delta: float, base: float, weight, vs=(1, 8, 32, 64, 100, 126, 127)) -> dict[int, float]:
    return {v: base + delta * weight(v) for v in vs}


def fw(v: int) -> float:
    return 1.0 if v >= 127 else v / 128


def test_fw_series_fits_the_firmware_law_exactly() -> None:
    n, span, r_fw, r_nat = ml.fit_series(series(100.0, 10.0, fw), 0.0, 127.0)
    assert n == 7 and span == pytest.approx(100.0 * (1 - 1 / 128))
    assert r_fw < 1e-9
    assert r_nat > 0.3


def test_clamped_points_are_left_out() -> None:
    pts = series(100.0, 64.0, fw)  # 64 + 100 w clamps at 127 for w >= 0.63
    n, _span, r_fw, _r_nat = ml.fit_series(pts, 0.0, 127.0)
    assert n < len(pts)
    assert r_fw < 1e-9


def test_a_flat_or_single_point_series_has_no_fit() -> None:
    assert ml.fit_series({1: 5.0, 2: 5.0}, 0.0, 127.0)[1] == 0.0
    assert ml.fit_series({1: 5.0}, 0.0, 127.0)[0] == 1


def test_bounds_of_the_delay_rate_id() -> None:
    assert ml.bounds(0x134) == (0.0, 127.0)
    assert ml.bounds(0x1AF) == (0.0, 16383.0)


def test_read_series_ignores_multi_event_scenarios() -> None:
    text = (
        "scenario,store,client,kind,id,name,value\n"
        "wheel:64,0x1,4,synth,0x134,Edt_Syn_kFltFreq,57.0000\n"
        "wheel:127,at:127,0x1,4,synth,0x134,Edt_Syn_kFltFreq,62.0000\n"
    )
    out = ml.read_series(text.replace("wheel:127,at:127,0x1", '"wheel:127,at:127",0x1'))
    assert out == {("0x1", 0x134, "Edt_Syn_kFltFreq", "wheel"): {64: 57.0}}


def test_report_rows_reports_only_moving_series() -> None:
    lines = ["scenario,store,client,kind,id,name,value"]
    for v in (1, 64, 127):
        lines.append(f"wheel:{v},0x1,4,synth,0x134,n,{10 + 20 * fw(v):.4f}")
        lines.append(f"wheel:{v},0x1,4,synth,0x135,m,3.0000")
    rows = ml.report_rows("\n".join(lines) + "\n")
    assert [r[1] for r in rows] == ["0x134"]
    assert float(rows[0][6]) < 1e-3
    assert ml.rows_csv(rows).splitlines()[0] == ",".join(ml.FIELDS)


def table() -> list[float]:
    return [100.0 * 2 ** (i / 16) for i in range(128)]


def test_cutoff_table_reads_128_entries_from_rust() -> None:
    body = ", ".join(f"{v:.1f}" for v in table())
    rust = f"pub const FILTER_CUTOFF_HZ_TABLE: [f32; 128] = [\n{body},\n];"
    assert len(ml.cutoff_table(rust)) == 128
    with pytest.raises(ValueError):
        ml.cutoff_table("nothing")


def test_interp_hz_is_geometric_between_steps() -> None:
    t = table()
    assert ml.interp_hz(t, 10.0) == pytest.approx(t[10])
    assert ml.interp_hz(t, 10.5) == pytest.approx((t[10] * t[11]) ** 0.5)
    assert ml.interp_hz(t, 200.0) == pytest.approx(t[127])


def test_sender_knob_drops_a_step_from_half_a_knob() -> None:
    assert ml.sender_knob(54.0) == 54.0
    assert ml.sender_knob(54.4375) == 54.4375
    assert ml.sender_knob(54.5) == 53.5
    assert ml.sender_knob(54.9375) == 53.9375


def test_cutoff_word_is_q8_24_twos_complement() -> None:
    assert ml.cutoff_word(52.0) == 0
    assert ml.cutoff_word(62.0) == 0x0A000000
    assert ml.cutoff_word(34.1015625) == 0xEE1A0000


def test_cutoff_rows_reproduce_the_measured_horizon_words() -> None:
    # Measured in the ORG core 2 words of `morph-fw-horizon.csv` (DM 0xE02E9 / 0xE0329).
    layers = {"A": (52, 10), "C": (34, 13)}
    rows = ml.cutoff_rows(table(), layers, (1, 32, 64, 96, 127))
    got = {(r[0], r[1]): r[4] for r in rows}
    want = {
        ("A", 1): "0x00140000",
        ("A", 32): "0x01800000",
        ("A", 64): "0x05000000",
        ("A", 96): "0x06800000",
        ("A", 127): "0x0a000000",
        ("C", 1): "0xee1a0000",
        ("C", 32): "0xf1400000",
        ("C", 64): "0xf3800000",
        ("C", 96): "0xf6c00000",
        ("C", 127): "0xfb000000",
    }
    assert got == want


def test_cutoff_rows_compare_both_laws() -> None:
    rows = ml.cutoff_rows(table(), {"A": (52, 10)}, (0, 64, 127))
    assert rows[0][2] == "52.0000" and rows[0][4] == "0x00000000"
    assert float(rows[2][8]) == pytest.approx(0.0, abs=1e-9)  # both laws agree at full scale
    assert rows[1][3] == "57.0000" and float(rows[1][8]) > 0.0
