import json
import os
import sys

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import uop_gates as g  # noqa: E402


def report(**over):
    """The ORG Core2 counters of design 9.1, healthy: 125 of 6,280 steps are `Interp`."""
    stats = {
        "passes": 1,
        "blocks_run": 520,
        "uops": 8478,
        "steps_uop": 6280,
        "steps_interp_uop": 125,
        "steps_core_step": 0,
        "steps_native": 0,
        "exits": [500, 10, 0, 5, 0, 5],
        "guard_fail": [0] * 9,
        "smc": 0,
        "translations": 0,
        "poisoned": 0,
        "maintain_calls": 0,
        "shadow_checks": 0,
        "shadow_mismatch": 0,
        "shadow_skipped": 0,
    }
    model = {"tier1_cycles": 340_000, "tier2_cycles": 236_000}
    stats.update(over.pop("stats", {}))
    model.update(over.pop("model", {}))
    return {"schema": 1, "label": "ORG Core2", "stats": stats, "model": model, **over}


def failing(checks):
    return [c.name for c in checks if not c.ok]


def test_a_healthy_report_passes_both_gates():
    assert failing(g.evaluate(report())) == []


def test_g2_fails_when_uops_per_step_exceed_the_limit():
    r = report(stats={"uops": 9000})  # 1.43 uops per step
    assert failing(g.gate_g2(r)) == ["uops_per_step"]


def test_g2_fails_when_too_many_steps_run_as_interp():
    r = report(stats={"steps_interp_uop": 600})  # 9.6% of steps
    assert failing(g.gate_g2(r)) == ["interp_step_share"]  # fast share is still 90.4%


def test_g2_fails_when_a_store_hit_translated_code_in_a_steady_pass():
    r = report(stats={"smc": 3})
    assert failing(g.gate_g2(r)) == ["smc"]


def test_g2_counts_core_step_gaps_as_interpreter_steps():
    r = report(stats={"steps_core_step": 800})
    assert "interp_step_share" in failing(g.gate_g2(r))


def test_g2_fails_on_slow_exits_per_pass_and_maintain_calls():
    assert failing(g.gate_g2(report(stats={"exits": [0, 0, 0, 0, 0, 21]}))) == [
        "slow_exits_per_pass"
    ]
    assert failing(g.gate_g2(report(stats={"exits": [0, 0, 0, 0, 0, 40], "passes": 2}))) == []
    assert failing(g.gate_g2(report(stats={"maintain_calls": 1}))) == ["maintain_calls"]


def test_g2_fails_a_run_with_no_steps():
    r = report(stats={"steps_uop": 0, "steps_interp_uop": 0, "uops": 0})
    assert failing(g.gate_g2(r)) == ["steps"]


def test_k3_compares_modeled_cycles_per_pass_with_290k():
    assert failing(g.gate_k3(report(model={"tier2_cycles": 290_000}))) == []
    assert failing(g.gate_k3(report(model={"tier2_cycles": 290_001}))) == [
        "modeled_cycles_per_pass"
    ]
    two = report(stats={"passes": 2}, model={"tier2_cycles": 500_000})
    assert failing(g.gate_k3(two)) == []


@pytest.mark.parametrize(
    "bad",
    [
        [],
        {"schema": 2},
        {"schema": 1},
        {"schema": 1, "stats": {}, "model": {}},
        "text",
    ],
)
def test_validate_rejects_malformed_reports(bad):
    with pytest.raises(g.ReportError):
        g.validate(bad)


def test_validate_rejects_short_exit_arrays():
    r = report(stats={"exits": [1, 2, 3]})
    with pytest.raises(g.ReportError):
        g.validate(r)


def test_load_accepts_an_object_or_a_list_and_rejects_non_json():
    assert len(g.load(json.dumps(report()))) == 1
    assert len(g.load(json.dumps([report(), report()]))) == 2
    with pytest.raises(g.ReportError):
        g.load("{nope")
    with pytest.raises(g.ReportError):
        g.load("[]")


def test_main_exit_code_reflects_the_verdict(tmp_path, capsys):
    ok = tmp_path / "ok.json"
    ok.write_text(json.dumps(report()))
    bad = tmp_path / "bad.json"
    bad.write_text(json.dumps(report(stats={"maintain_calls": 3})))
    assert g.main([str(ok)]) == 0
    assert "verdict: PASS" in capsys.readouterr().out
    assert g.main([str(ok), str(bad)]) == 1
    out = capsys.readouterr().out
    assert "FAIL G2 maintain_calls" in out and "verdict: FAIL" in out
    assert g.main(["--gate", "K3", str(bad)]) == 0


def test_main_reports_unreadable_input_with_exit_code_2(tmp_path, capsys):
    junk = tmp_path / "junk.json"
    junk.write_text("not json")
    assert g.main([str(junk)]) == 2
    assert g.main([str(tmp_path / "missing.json")]) == 2
    assert "uop_gates" in capsys.readouterr().err


def test_the_skeleton_report_of_nop_steps_fails_g2_as_nothing_is_translated():
    r = report(
        stats={
            "uops": 0,
            "steps_uop": 0,
            "steps_interp_uop": 0,
            "steps_core_step": 40,
            "passes": 0,
            "exits": [0] * 6,
        },
        model={"tier1_cycles": 4000, "tier2_cycles": 4000},
    )
    assert "fast_step_share" in failing(g.gate_g2(r))
    assert failing(g.gate_k3(r)) == []
