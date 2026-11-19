"""Unit tests for tools/sample_gaps_scenes.py (scene masks of SAMPLE layers)."""

from __future__ import annotations

import json

import sample_gaps_scenes as sg


def test_scene_counts_by_layer_bits() -> None:
    res = {"synth_a": "a.nsmp4", "synth_b": None, "synth_c": "c.nsmp4"}
    # Synth A (16) in scene I only, Synth C (64) in scene II only
    c = sg.scene_counts((16, 64), res)
    assert c == {"sample_layers": 2, "scene1": 1, "scene2": 1, "scene2_only": 1, "neither": 0}
    # a SAMPLE layer in no scene
    assert sg.scene_counts((0, 0), res)["neither"] == 2
    # a layer without a member is not counted
    assert sg.scene_counts((16, 16), {"synth_a": None})["sample_layers"] == 0


def test_masks_of_skips_text_before_the_json() -> None:
    text = "note line\n" + json.dumps({"global": {"scene_1_mask": 4, "scene_2_mask": 20}})
    assert sg.masks_of(text) == (4, 20)
