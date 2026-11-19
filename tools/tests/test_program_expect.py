"""Tests of `program_expect`: scene mask rule, zone gain, level and arp expectations."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import program_expect as pe  # noqa: E402


def glob(scene_1_mask: int = 0, section: int = 0, **kw) -> dict:
    """Decoded `global` block: no split, every layer on all zones (code 5)."""
    g = {
        "split_enabled": False, "split_zone_enabled": [False, True, False],
        "split_points": [41, 60, 77], "split_crossfades": ["Off", "Off", "Off"],
        "kb_zone": [5] * 7, "scene_1_mask": scene_1_mask, "section_enable_i": section,
    }  # fmt: skip
    return g | kw


def program(g: dict, lv: float = 1.0, arp: bool = False) -> dict:
    """Decoded program with equal levels on all layers."""
    syn = {"level": lv, "arp_enabled": arp}
    return {
        "global": g,
        "organ": {"layer_a_level": lv, "layer_b_level": lv},
        "piano": {"layer_a": {"level": lv}, "layer_b": {"level": lv}},
        "synth": {"layer_a": dict(syn), "layer_b": dict(syn), "layer_c": dict(syn)},
    }


def test_enabled_section_without_layers_plays_layer_a() -> None:
    # 70s CP Strings: piano and synth sections on, no layer enabled in scene I
    g = glob(scene_1_mask=0, section=0b011)
    assert pe.effective_mask(g) == 0b001_0100  # piano A (bit 2) and synth A (bit 4)
    # a section with layers keeps its mask, a disabled section stays empty
    assert pe.effective_mask(glob(0b000_0011, 0b100)) == 0b11
    assert pe.effective_mask(glob(0, 0)) == 0


def test_zone_gain_follows_the_split_state() -> None:
    # Mid point at 60 only: code 0 = lower half, code 6 = upper half (zone-key-ranges.md)
    g = glob(split_enabled=True, kb_zone=[0, 6, 5, 5, 5, 5, 5])
    assert pe.zone_gain(g, 0, 48) == 1.0 and pe.zone_gain(g, 0, 60) == 0.0
    assert pe.zone_gain(g, 1, 48) == 0.0 and pe.zone_gain(g, 1, 60) == 1.0
    assert pe.zone_gain(g, 2, 60) == 1.0  # code 5: whole keyboard
    # without a split every code spans the keyboard (all points inactive: case 9)
    assert pe.zone_gain(glob(kb_zone=[0, 6, 9, 10, 5, 5, 5]), 0, 100) == 1.0


def test_expectation_marks_zone_and_level_silences() -> None:
    # Wormhole Organ shape: organ A in the lower half only, key 60 is above it
    g = glob(0b001, 0b100, split_enabled=True, kb_zone=[0, 5, 5, 5, 5, 5, 5])
    row = pe.expect(program(g), 60)
    assert row["audible"] == {"organ": False, "piano": False, "synth": False}
    assert pe.silent_reason(row, "organ") == "zone"
    assert pe.expect(program(g), 40)["audible"]["organ"] is True
    # Ballad Layer Whl shape: organ A on its zone but stored level 0
    row = pe.expect(program(glob(0b001, 0b100), lv=0.0), 60)
    assert row["audible"]["organ"] is False and pe.silent_reason(row, "organ") == "level"


def test_expectation_of_a_sounding_program_and_arp_flag() -> None:
    row = pe.expect(program(glob(0b001_0001, 0b101), arp=True), 60)
    assert row["audible"] == {"organ": True, "piano": False, "synth": True}
    assert row["arp"] is True and pe.silent_reason(row, "synth") == "audible"
    assert pe.silent_reason(row, "piano") == "no_layer"
    assert pe.expect(program(glob(0b001_0000, 0b001)), 60)["arp"] is False
