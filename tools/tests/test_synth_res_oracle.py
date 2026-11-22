"""Tests for tools/synth_res_oracle.py (pure helpers and the committed derived CSVs)."""

import csv

import synth_res_oracle as m


def test_damping_from_frame_inverts_the_prologue() -> None:
    # g = 2 R N, k = P N  ->  P = 2 R k / g
    r, p = 0.3, 0.7071
    n = 1.0 / (1.0 + r * p + r * r) ** 0.5
    assert abs(m.damping_from_frame(r, 2 * r * n, p * n) - p) < 1e-12


def test_quadratic_matches_table_b_only_below_index_38() -> None:
    b = m.read_tables()["B"]
    lead = 0
    while lead < 129 and abs(b[lead] / m.quadratic_k(lead) - 1) < 1e-4:
        lead += 1
    assert lead == 38
    # beyond the splice the table is linear, `sqrt(0.5) (128 - i) / 64`, not the quadratic
    assert abs(b[64] - 0.70710678) < 1e-6
    assert abs(m.quadratic_k(64) - 0.5) < 1e-12
    assert all(
        abs(b[i] / (0.70710678 * (128 - i) / 64) - 1) < 1e-4 for i in range(38, 128)
    )  # EMPIRICAL form, 2e-5


def test_quadratic_is_the_lp12_damping_table_d_over_all_128_entries() -> None:
    # the OpenNord closed form is table D (`0xB0F94`, LP12), not table B (LP24, BP, HP)
    d = m.read_tables()["D"]
    assert max(abs(d[i] / m.quadratic_k(i) - 1) for i in range(128)) < 1e-5
    assert abs(d[128] / d[127] - 1) < 1e-3  # the extra entry repeats the last one


def test_interp_is_linear_and_clamps_at_the_last_entry() -> None:
    tab = [float(i) for i in range(129)]
    assert m.interp(tab, 10.25) == 10.25
    assert abs(m.interp(tab, 127.999) - 127.999) < 1e-9


def test_stage_tables() -> None:
    assert [m.table_for(s) for s in ("LP12", "LP24", "BP", "HP")] == ["D", "B", "B", "B"]
    assert [m.gain_table_for(s) for s in ("LP12", "LP24", "BP", "HP")] == ["A", "A", "C", "A"]


def test_sweep_errors_of_an_exact_run_are_zero() -> None:
    t = m.read_tables()
    rows = []
    for pos in (3.5, 40.25, 64.0, 90.9, 127.5):
        p = m.interp(t["B"], pos)
        rows.append({"pos": pos, "p": p, "cin": m.interp(t["A"], pos)})
    ep, ec, eq = m.sweep_errors(rows, t["B"], t["A"])
    assert ep == 0.0 and ec == 0.0
    assert eq > 1.0  # the quadratic is far from the table at 90.9 and 127.5


def test_settled_csv_follows_the_firmware_position_and_table() -> None:
    t = m.read_tables()
    with (m.DATA / "synth-res-settled.csv").open(encoding="utf-8") as fh:
        rows = list(csv.DictReader(fh))
    assert [int(r["knob"]) for r in rows] == list(range(128))
    for r in rows:
        k = int(r["knob"])
        pos = float(r["pos_lp24"])
        # settled position 128 knob / 127 (clipped at 127.999); the 1500 pass run leaves 0.035
        assert abs(pos - min(128.0 * k / 127.0, 127.999)) < 0.04, k
        assert abs(float(r["pos_lp12"]) - pos) < 1e-3
        assert abs(float(r["p_lp24"]) / m.interp(t["B"], pos) - 1) < 1e-4, k
        assert abs(float(r["p_lp12"]) / m.interp(t["D"], pos) - 1) < 1e-4, k
        assert abs(float(r["cin_lp24"]) / m.interp(t["A"], pos) - 1) < 1e-4, k


def test_sweep_csv_records_agreement_for_every_stage() -> None:
    with (m.DATA / "synth-res-sweep.csv").open(encoding="utf-8") as fh:
        rows = list(csv.DictReader(fh))
    assert {r["stage"] for r in rows} == {"LP12", "LP24", "BP", "HP", "Ladder"}
    for r in rows:
        if r["stage"] == "Ladder":
            assert r["p_rel_err"].startswith("mismatch 0/")
        else:
            assert float(r["p_rel_err"]) < 5e-6 and float(r["cin_rel_err"]) < 5e-6
