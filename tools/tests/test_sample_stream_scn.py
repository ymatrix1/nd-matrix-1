"""Unit tests for tools/sample_stream_scn.py (message selection, ring signal, scenarios, dump)."""

from __future__ import annotations

import struct
from pathlib import Path

import pytest
import sample_stream_scn as s


def row(dm: int, w2: int, chip: str = "SY", path: int = 1) -> dict[str, object]:
    return {"chip": chip, "path": path, "dm": dm, "w2": w2}


def test_voice_and_field_of_record_addresses() -> None:
    assert s.voice_of(0x92414) == 0 and s.field_of(0x92414) == 0x14
    assert s.voice_of(0x9244C) == 1 and s.field_of(0x9244C) == 0x14
    # kernel base of voice 0 reaches into the next record: remainder 3 -> voice 0, field 0x3B
    assert s.voice_of(0x9243B) == 0 and s.field_of(0x9243B) == 0x3B
    assert s.voice_of(0x921A6) is None
    assert s.voice_of(0x92400 + 0x38 * 56) is None


def test_voice0_messages_orders_prepare_first_and_drops_unpacker_fields() -> None:
    rows = [
        row(0x92414, 0x1C0794),  # reset
        row(0x92416, 1),  # unpacker field
        row(0x92420, 0x3F128F5C),  # coefficient
        row(0x9240A, 0x22),  # zone pair
        row(0x92414, 0x1C07CD),  # ARM prepare (re-added first)
        row(0x92433, 0),
        row(0x9243B, 0xE4C00),
        row(0x921A6, 0x3C74DA00),
        row(0x92432, 0xBF7FFD3E),
        row(0x9244C, 0x1C0794),  # voice 1
        row(0x92473, 0xE5800),
        row(0x9219E, 0x04000000),  # index word of voice 0
        row(0x92100, 5, chip="SMP"),
    ]
    msgs, index = s.voice0_messages(rows)
    assert msgs[0] == (0x92414, s.STATE_PREPARE)
    assert (0x9243B, 0xE4C00) in msgs and (0x921A6, 0x3C74DA00) in msgs
    assert (0x92432, 0xBF7FFD3E) in msgs and (0x9219E, 0x04000000) in msgs
    dropped = {0x92416, 0x92420, 0x9240A, 0x9244C, 0x92473, 0x92100}
    assert not dropped & {a for a, _ in msgs}
    assert sum(1 for a, _ in msgs if a == 0x92414) == 1
    assert index == 0


def test_default_index_without_word0_message() -> None:
    _, index = s.voice0_messages([row(0x921A6, 1)])
    assert index == s.DEFAULT_INDEX
    assert s.ENTRY_BASE + index == 0x93580


def test_coefficients_default_to_identity() -> None:
    assert s.coefficients([]) == (0.0, -1.0)
    c0 = struct.unpack("<I", struct.pack("<f", 0.5))[0]
    c1 = struct.unpack("<I", struct.pack("<f", -0.25))[0]
    assert s.coefficients([row(0x92420, c0), row(0x92421, c1)]) == (0.5, -0.25)


def test_ring_signal_scale_and_one_pole() -> None:
    out = s.ring_signal([(1.0, 2.0), (0.0, 0.0)], 0.5, -0.25)
    assert out[0] == pytest.approx((0.25 * s.RING_SCALE, 0.5 * s.RING_SCALE))
    assert out[1] == pytest.approx((0.5 * 0.25 * s.RING_SCALE, 0.5 * 0.5 * s.RING_SCALE))
    ident = s.ring_signal([(3.0, -3.0)], 0.0, -1.0)
    assert ident[0] == pytest.approx((3.0 * s.RING_SCALE, -3.0 * s.RING_SCALE))


def test_pcm_round_trip(tmp_path: Path) -> None:
    frames = [(1.0, -1.0), (0.5, 0.25)]
    s.write_pcm(tmp_path / "x.f32", frames)
    assert s.read_pcm(tmp_path / "x.f32") == frames


def test_pro_scenario_and_dump(tmp_path: Path) -> None:
    src = tmp_path / "a.scn"
    src.write_text("# c\nmeta rate 48000\n\npoke 0x1 0x2\nirq 13\nmsg 0 0\n", encoding="utf-8")
    pro = s.prologue(src)
    assert pro == ["meta rate 48000", "poke 0x1 0x2", "irq 13"]
    text = s.build_pcm_scenario(pro, Path("r.f32"), [(0x92414, s.STATE_PREPARE)], 0x93580, 790, 5)
    lines = text.splitlines()
    assert lines[0] == "meta rate 43402" and "pcm 0 path=r.f32" in lines
    assert lines.index("msg 0x92414 0x1c07cd") < lines.index("irq 17 400000")
    assert "poke 0x93580 790" in lines and "loop 5" in lines and lines[-1] == "end"
    st = s.build_stream_scenario(pro, Path("s.bin"), [(0x92414, 1)], 0x93580, 790, 3)
    assert "stream 0 path=s.bin" in st and st.splitlines()[-1].startswith("capture ringL")
    dump = tmp_path / "d.csv"
    dump.write_text("# h\nindex,L,R\n0,1e0,2e0\n1,3e0,4e0\n2,5e0,\n", encoding="utf-8")
    d = s.read_dump(dump)
    assert d["L"] == [1.0, 3.0, 5.0] and d["R"] == [2.0, 4.0]
