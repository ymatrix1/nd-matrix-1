"""Tests for the Amp LP24 / HP24 routine model (no firmware needed)."""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import fx_model_lp24 as lp  # noqa: E402


def test_recorded_oracle_outputs_are_reproduced() -> None:
    cases = json.loads(lp.CASES.read_text(encoding="utf-8"))["cases"]
    assert len(cases) == 8
    assert {c["mode"] for c in cases} == {lp.LP, lp.HP}
    for case in cases:
        exp = lp.model_from_mem(case["mem"], case["mode"])
        for addr, val in exp.items():
            assert abs(lp.f32(case["oracle"][str(addr)]) - val) < 1e-5


def test_random_case_is_deterministic_per_seed() -> None:
    assert lp.random_case(3, lp.LP) == lp.random_case(3, lp.LP)
    assert lp.random_case(3, lp.LP) != lp.random_case(4, lp.LP)


def test_smoothing_constant_is_the_firmware_one() -> None:
    assert lp.bits(lp.K) == 0x3F7DA39E


def test_modes_differ_and_states_are_clipped() -> None:
    case = lp.random_case(1, lp.LP)
    frames = [0.3, -0.2, 0.5, 0.1]
    lo, s_lp, _ = lp.model(case["ctx"], frames, lp.LP)
    hi, _, _ = lp.model(case["ctx"], frames, lp.HP)
    assert lo != hi
    assert abs(s_lp[-16]) <= case["ctx"][-25] + 1e-9
