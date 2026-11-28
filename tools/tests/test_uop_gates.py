import io
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
    return {
        "schema": 1,
        "label": "ORG Core2",
        "window": "measured",
        "stats": stats,
        "model": model,
        **over,
    }


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


#: Schema 1 JSON of the ORG Core2 reference counters, byte for byte what
#: `UopReport::to_json` prints (pinned on the Rust side by
#: `the_json_of_schema_1_is_pinned_byte_for_byte` in `runner/uop_report/json.rs`, which holds the
#: same text): change both or neither.
GOLDEN = (
    '{"schema":1,"label":"ORG Core2","window":"measured",'
    '"replay":{"checked":0,"skipped":0},"stats":{"passes":1,'
    '"blocks_run":520,"uops":8478,"steps_uop":6280,"steps_interp_uop":125,'
    '"steps_core_step":0,"steps_native":0,"exits":[1,2,3,4,5,6],'
    '"guard_fail":[0,0,0,0,0,0,0,0,9],"smc":0,"translations":0,"poisoned":0,'
    '"maintain_calls":0,"shadow_checks":0,"shadow_mismatch":0,"shadow_skipped":0},'
    '"model":{"tier1_cycles":341638,"tier2_cycles":341638}}'
)

#: [`GOLDEN`] with the block-length histogram of the entries 12, 12 and 3 steps (Rust:
#: `the_profiled_json_is_pinned_byte_for_byte`): change both or neither.
GOLDEN_PROFILED = (
    GOLDEN[:-1] + ',"block_steps":{"edges":[1,2,3,4,6,8,12,16,24,32,48],'
    '"counts":[0,0,1,0,0,0,2,0,0,0,0,0],"entries":3,"steps":27,"chained":0,"empty":0}}'
)


def test_the_rust_golden_report_validates_and_reads_as_the_design_reference_pass():
    r = g.load(GOLDEN)[0]
    assert r["label"] == "ORG Core2"
    assert failing(g.gate_g2(r)) == []  # 1.35 uops per step, 2% Interp, 6 Slow exits, no smc
    # a Tier 1 report has no native steps: its Tier 2 model is the Tier 1 model, 341,638 > 290,000
    assert r["model"]["tier2_cycles"] == r["model"]["tier1_cycles"] == 341_638
    assert failing(g.gate_k3(r)) == ["modeled_cycles_per_pass"]


def test_the_checks_are_exactly_the_design_rows():
    """Design section 12: G2 and K3 and nothing else. A new check needs a new design row first."""
    assert g.GATES == ("G2", "K3")
    r = g.load(GOLDEN)[0]
    assert [c.name for c in g.gate_g2(r)] == [
        "uops_per_step",  # uops per step <= 1.35
        "fast_step_share",  # native (non-Interp) step share >= 90%
        "interp_step_share",  # Interp uops <= 6% of steps
        "slow_exits_per_pass",  # Slow exits <= 20 per pass
        "maintain_calls",  # == 0 in steady passes
        "smc",  # == 0 in steady passes
    ]
    assert [c.name for c in g.gate_k3(r)] == ["modeled_cycles_per_pass"]  # <= 1.4 x budget
    assert (
        g.MAX_UOPS_PER_STEP,
        g.MIN_FAST_SHARE,
        g.MAX_INTERP_SHARE,
        g.MAX_SLOW_EXITS_PER_PASS,
        g.K3_MAX_CYCLES_PER_PASS,
    ) == (1.35, 0.90, 0.06, 20, 290_000)


def test_validate_rejects_bools_and_negative_numbers_as_counts():
    for bad_stats in (
        {"passes": True},
        {"smc": -1},
        {"exits": [1, 2, 3, 4, 5, -6]},
        {"guard_fail": [True] * 9},
    ):
        with pytest.raises(g.ReportError):
            g.validate(report(stats=bad_stats))
    with pytest.raises(g.ReportError):
        g.validate(report(model={"tier2_cycles": False}))


def test_a_list_report_is_evaluated_per_label_and_one_failure_fails_the_run(tmp_path, capsys):
    good = report(label="PNO Core2")
    bad = report(label="ORG Core2", stats={"smc": 2})
    path = tmp_path / "bench.json"
    path.write_text(json.dumps([good, bad]))
    assert g.main(["--gate", "G2", str(path)]) == 1
    out = capsys.readouterr().out
    assert "FAIL G2 smc = 2 (limit == 0) [ORG Core2]" in out
    assert "PASS G2 smc = 0 (limit == 0) [PNO Core2]" in out
    assert out.strip().endswith("verdict: FAIL")


def test_main_reads_a_report_from_stdin(monkeypatch, capsys):
    monkeypatch.setattr(sys, "stdin", io.StringIO(json.dumps(report())))
    assert g.main(["-"]) == 0
    assert "verdict: PASS" in capsys.readouterr().out
    monkeypatch.setattr(sys, "stdin", io.StringIO("{not json"))
    assert g.main(["-"]) == 2


def test_the_gate_flag_selects_one_gate():
    r = report(model={"tier2_cycles": 999_999}, stats={"smc": 1})
    assert failing(g.evaluate(r, ("K3",))) == ["modeled_cycles_per_pass"]
    assert failing(g.evaluate(r, ("G2",))) == ["smc"]
    assert sorted(failing(g.evaluate(r))) == ["modeled_cycles_per_pass", "smc"]


def test_the_golden_report_is_a_measured_window_with_no_histogram():
    r = g.load(GOLDEN)[0]
    assert (r["window"], r["replay"], "block_steps" in r) == (
        "measured",
        {"checked": 0, "skipped": 0},
        False,
    )


def test_the_profiled_golden_report_validates_and_its_histogram_adds_up():
    r = g.load(GOLDEN_PROFILED)[0]
    h = r["block_steps"]
    assert (sum(h["counts"]), h["entries"], h["steps"]) == (3, 3, 27)
    assert len(h["counts"]) == len(h["edges"]) + 1
    assert failing(g.evaluate(r, ("G2",))) == []  # the histogram gates nothing


# --- the steady-state counters are evaluated on measured passes only ---


def steady_checks(r):
    return [c for c in g.gate_g2(r) if c.name in ("maintain_calls", "smc")]


def test_a_report_of_every_pass_cannot_pass_the_steady_state_counters():
    # healthy numbers, but the window includes the warm-up: the checks fail and say why
    checks = steady_checks(report(window="all"))
    assert [c.ok for c in checks] == [False, False]
    assert all("window all" in c.limit and "warm-up" in c.limit for c in checks)
    # the other G2 checks read the same counters whichever the window
    assert failing(g.gate_g2(report(window="all"))) == ["maintain_calls", "smc"]
    assert failing(g.gate_k3(report(window="all"))) == []


def test_a_measured_report_evaluates_the_steady_state_counters_on_their_value():
    assert [c.ok for c in steady_checks(report(window="measured"))] == [True, True]
    r = report(window="measured", stats={"maintain_calls": 2, "smc": 1})
    assert failing(g.gate_g2(r)) == ["maintain_calls", "smc"]
    assert all("window" not in c.limit for c in steady_checks(r))


def test_a_report_without_a_window_is_read_as_before_the_member_existed():
    r = report()
    del r["window"]
    assert failing(g.gate_g2(g.validate(r))) == []
    r["stats"]["smc"] = 4
    assert failing(g.gate_g2(g.validate(r))) == ["smc"]


@pytest.mark.parametrize("window", ["warmup", "", 1, True, ["all"]])
def test_validate_rejects_a_window_that_is_not_measured_or_all(window):
    with pytest.raises(g.ReportError, match="window"):
        g.validate(report(window=window))


def test_the_main_verdict_fails_an_all_window_report(tmp_path, capsys):
    path = tmp_path / "bench.json"
    path.write_text(json.dumps([report(window="all", label="A")]))
    assert g.main(["--gate", "G2", str(path)]) == 1
    out = capsys.readouterr().out
    assert "FAIL G2 smc = 0 (limit == 0 (window all: includes the warm-up" in out
    assert "PASS G2 uops_per_step" in out


# --- steps_native is a subset of steps_uop ---


def test_native_steps_are_a_subset_of_the_uop_steps_and_are_not_added_to_the_total():
    plain = g.validate(report())
    all_native = g.validate(report(stats={"steps_native": 6280}))
    assert g.steps_of(all_native["stats"]) == g.steps_of(plain["stats"]) == 6280
    assert [(c.name, c.value) for c in g.gate_g2(all_native)] == [
        (c.name, c.value) for c in g.gate_g2(plain)
    ]


def test_validate_rejects_more_native_steps_than_uop_steps():
    with pytest.raises(g.ReportError, match="steps_native"):
        g.validate(report(stats={"steps_native": 6281}))
    # equal is the all-native pass and valid; the skeleton report (no steps at all) too
    g.validate(report(stats={"steps_native": 6280}))
    g.validate(report(stats={"steps_uop": 0, "steps_interp_uop": 0, "steps_native": 0}))
    with pytest.raises(g.ReportError, match="subset"):
        g.validate(report(stats={"steps_uop": 0, "steps_interp_uop": 0, "steps_native": 1}))


# --- the optional members: replay and the block-length histogram ---


def profiled(**over):
    hist = {
        "edges": [1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48],
        "counts": [0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0],
        "entries": 3,
        "steps": 27,
        "chained": 0,
        "empty": 0,
    }
    hist.update(over)
    return report(block_steps=hist)


def test_replay_and_block_steps_are_optional_and_shape_checked():
    g.validate(report(replay={"checked": 7, "skipped": 0}))
    g.validate(profiled())
    for bad in (
        {"checked": 1},
        {"checked": -1, "skipped": 0},
        {"checked": True, "skipped": 0},
        [],
        "x",
    ):
        with pytest.raises(g.ReportError, match="replay"):
            g.validate(report(replay=bad))


@pytest.mark.parametrize(
    "bad",
    [
        {"edges": []},
        {"edges": [2, 1, 3, 4, 6, 8, 12, 16, 24, 32, 48]},  # not increasing
        {"edges": [0, 1, 3, 4, 6, 8, 12, 16, 24, 32, 48]},  # zero edge
        {"counts": [0, 0, 1]},  # one count per edge plus the open bucket
        {"counts": [0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, "x"]},
        {"entries": 4},  # the buckets add up to 3
        {"steps": 2},  # each entry ran at least one step
        {"empty": None},
        {"chained": 300},  # 3 + 2 x 300 blocks, stats.blocks_run is 520
    ],
)
def test_validate_rejects_a_block_histogram_that_does_not_add_up(bad):
    with pytest.raises(g.ReportError, match="block_steps"):
        g.validate(profiled(**bad))


def test_validate_rejects_a_block_histogram_that_is_not_an_object():
    with pytest.raises(g.ReportError, match="block_steps"):
        g.validate(report(block_steps=[1, 2]))


def test_a_chained_entry_stands_for_two_or_more_blocks():
    # 3 + 1 empty + 2 x 258 = 520 blocks: the most the report's `blocks_run` allows
    g.validate(profiled(empty=1, chained=258))
    with pytest.raises(g.ReportError, match="blocks"):
        g.validate(profiled(empty=1, chained=259))
