"""Unit tests for tools/fx_defaults_boot.py."""

import struct

import fx_defaults_boot as fb


def w(x: float) -> str:
    return hex(struct.unpack("<I", struct.pack("<f", x))[0])


def row(dm: int, word: str, core: int = 1) -> dict:
    return {"chip": "FX", "core": core, "dm": hex(dm), "w2": word}


def test_comp_amount_is_recovered_from_the_threshold_word():
    i1 = 0xB0168
    fx = [
        row(i1 - 0x20, fb.CMP_MARK),
        row(i1 - 0x1C, w(4.2006)),
        row(i1 - 0x1B, w(-36.0 * 95 / 127)),
    ]
    assert fb.comp_amounts(fx) == {95}


def test_reverb_amount_is_found_by_its_wet_word(tmp_path):
    t = tmp_path / "rvb.csv"
    t.write_text("id,value,dry_word,wet_word\nx,0,1,0\nx,38,1,1041936640\nx,64,1,12345\n")
    fx = [row(0xB27FA, hex(1041936640))]
    assert fb.reverb_amounts(fx, t) == {38}


def test_f32_round_trips():
    assert fb.f32(0x3F800000) == 1.0


def test_load_fx_keeps_only_fx_rows(tmp_path):
    p = tmp_path / "log.jsonl"
    p.write_text('{"chip": "FX", "core": 1}\n{"chip": "SMP"}\n')
    assert len(fb.load_fx(p)) == 1
