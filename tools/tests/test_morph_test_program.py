"""Unit tests for tools/morph_test_program.py (pure helpers, no ns4 binary)."""

from __future__ import annotations

import morph_test_program as mt
import pytest


def morph() -> dict:
    return {"wheel": 0.0, "aftertouch": 0.0, "ctrl_pedal": 0.0}


def program() -> dict:
    fx = {"mod1_rate_morph": morph(), "mix": 1}
    synth = {k: {t: morph() for t in mt.SYNTH_TARGETS} for k in ("layer_a", "layer_b", "layer_c")}
    synth.update(fx_a=dict(fx), fx_b=dict(fx), fx_c=dict(fx))
    piano = {
        "layer_a": {"level_morph": morph()},
        "layer_b": {"level_morph": morph()},
        "fx_a": dict(fx),
        "fx_b": dict(fx),
    }
    organ = {
        "layer_a_level_morph": morph(),
        "layer_b_level_morph": morph(),
        "drawbars_a_morph": [morph() for _ in range(9)],
        "drawbars_b_morph": [morph() for _ in range(9)],
        "fx": dict(fx),
    }
    return {"synth": synth, "piano": piano, "organ": organ}


def test_set_depth_writes_raw_steps_over_127() -> None:
    m = morph()
    mt.set_depth(m, (127, 64, -64))
    assert m["wheel"] == 1.0
    assert m["aftertouch"] == pytest.approx(64 / 127)
    assert m["ctrl_pedal"] == pytest.approx(-64 / 127)


def test_apply_test_morphs_sets_every_target() -> None:
    p = mt.apply_test_morphs(program())
    assert p["synth"]["layer_b"]["filter_cutoff_morph"]["wheel"] == pytest.approx(50 / 127)
    assert p["synth"]["layer_c"]["lfo_rate_morph"]["ctrl_pedal"] == pytest.approx(25 / 127)
    assert p["piano"]["layer_a"]["level_morph"]["wheel"] == 1.0
    assert p["organ"]["layer_a_level_morph"]["ctrl_pedal"] == pytest.approx(-40 / 127)
    assert p["organ"]["drawbars_b_morph"][8]["wheel"] == pytest.approx(32 / 127)
    assert p["synth"]["fx_c"]["mod1_rate_morph"]["aftertouch"] == pytest.approx(24 / 127)
    assert p["organ"]["fx"]["mix"] == 1


def test_apply_test_morphs_leaves_non_morph_fx_fields() -> None:
    p = mt.apply_test_morphs(program())
    assert p["piano"]["fx_a"]["mix"] == 1
