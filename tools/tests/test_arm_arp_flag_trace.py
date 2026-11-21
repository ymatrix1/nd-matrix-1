"""Unit tests for tools/arm_arp_flag_trace.py (pure helpers; the Unicorn run needs the firmware)."""

from __future__ import annotations

import arm_arp_flag_trace as ft
import ns4_arp_modes as modes
import ns4_bitscan as bs
import ns4_synth_fields as sf
import pytest


def synth_file() -> bytes:
    return bytes(sf.NS4P_SYNTH_OFFSET) + bytes(bs.SYNTH_PAYLOAD_LEN)


def layer_fields(raw: bytes, layer: str) -> dict[str, int]:
    block = bs.synth_blocks(sf.synth_payload_from_ns4p(raw), "layer")["ABC".index(layer)]
    return modes.decode_modes(block)


def test_set_bits_writes_msb_first_and_keeps_neighbours():
    assert ft.set_bits(b"\xff\xff", 4, 8, 0) == b"\xf0\x0f"
    assert ft.set_bits(b"\x00", 0, 1, 1) == b"\x80"
    assert ft.set_bits(b"\x00\x00", 7, 2, 3) == b"\x01\x80"


def test_set_bits_rejects_a_value_that_does_not_fit():
    with pytest.raises(ValueError, match="does not fit"):
        ft.set_bits(b"\x00", 0, 2, 4)


@pytest.mark.parametrize("layer", ["A", "B", "C"])
def test_patch_layer_changes_only_that_field_of_that_layer(layer):
    raw = ft.patch_layer(synth_file(), layer, "mode", 2)
    assert layer_fields(raw, layer)["mode"] == 2
    for other in "ABC".replace(layer, ""):
        assert layer_fields(raw, other) == layer_fields(synth_file(), other)
    assert layer_fields(raw, layer) == {**layer_fields(synth_file(), layer), "mode": 2}


def test_apply_patches_applies_every_patch_in_order():
    raw = ft.apply_patches(synth_file(), ["A.rate=12", "C.mst_clk=1", "A.rate=13"])
    assert layer_fields(raw, "A")["rate"] == 13
    assert layer_fields(raw, "C")["mst_clk"] == 1


@pytest.mark.parametrize("text", ["D.mode=1", "A.nope=1", "AB.mode=1", "A.mode"])
def test_parse_patch_rejects_bad_text(text):
    with pytest.raises(ValueError):
        ft.parse_patch(text)


def test_parse_patch_reads_hex_and_decimal():
    assert ft.parse_patch("C.range=0x10") == ("C", "range", 16)
