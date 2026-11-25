"""Tests of `smp_stream_end`: record words of a voice start, loop fields of an `.npno` record."""

import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import smp_stream_end as se  # noqa: E402

# VERIFIED (Astoria Grand run, SMP0.script pass 814, stroke 513 on voice 0): the ARM writes the
# stream words 0x9391D .. 0x93924 = F-11 .. F-4 and the rate words F+6 / F+7 from the request.
F = se.voice_base(0)
MSGS = {
    814: [
        (F - 11, 0x6E6F),
        (F - 10, 0x9E7E),
        (F - 9, 0x27AE4),
        (F - 8, 0x27AC9),
        (F - 7, 0xD385),
        (F - 6, 0x1A28C),
        (F + 0x40, 7),  # another record: ignored
        (se.REQUEST0 + 5, 0x3F7FFE8E),  # request block of voice 0: F+6
        (se.REQUEST0 + 6, 0x3F7FFF3A),  # F+7
        (se.REQUEST0 + 12 + 5, 0x3F7FFE8E),  # voice 1
        (se.REQUEST0 + 12 + 6, 0x3F7FFF36),
    ]
}


def test_voice_base_and_record_words() -> None:
    assert F == 0x93928
    assert se.voice_base(1) == 0x9395E
    w = se.record_words(MSGS, 0)
    assert w[-9] == 0x27AE4 and 8 not in w and 0x40 not in w
    assert se.record_words(MSGS, 1) == {}


def test_loop_words_of_stroke_513() -> None:
    lw = se.loop_words(se.record_words(MSGS, 0))
    assert lw == {"end": 162532, "loop_end": 162505, "loop_len": 54149, "loop_start": 108356}


def test_rate_words_are_counts_of_two_to_the_24() -> None:
    d6, d7 = (se.rate_count(x) for x in se.request_rates(MSGS, 0))
    assert (d6, d7) == (370, 198)  # in the loop, outside
    assert se.rate_count(se.request_rates(MSGS, 1)[1]) == 202
    assert se.request_rates(MSGS, 2) is None
    assert abs(se.f32(0x3F800000) - 1.0) < 1e-12


def test_npno_record_fields_equal_the_voice_record_words() -> None:
    rec = bytearray(se.STROKE_RECORD)
    struct.pack_into(">I", rec, 0x0E, 162532)
    struct.pack_into(">I", rec, 0x24, 108356)
    struct.pack_into(">I", rec, 0x28, 54149)
    struct.pack_into(">h", rec, 0x38, -185)
    f = se.npno_loop_fields(bytes(rec))
    lw = se.loop_words(se.record_words(MSGS, 0))
    assert (f["end"], f["loop_end"], f["loop_len"], f["loop_start"]) == (
        lw["end"],
        lw["loop_end"],
        lw["loop_len"],
        lw["loop_start"],
    )
    assert 2 * -f["loop_decay"] == se.rate_count(se.request_rates(MSGS, 0)[0])


def test_stroke_record_slices_the_table() -> None:
    npno = bytes(se.STROKE_TABLE) + bytes(range(se.STROKE_RECORD)) * 2
    assert se.stroke_record(npno, 1) == bytes(range(se.STROKE_RECORD))


def test_read_script(tmp_path: Path) -> None:
    f = tmp_path / "SMP0.script"
    f.write_text("814 0x93928 0x00000001\n814 0x93929 0xffffffff\n9 0x1 0x2\n", "utf-8")
    assert se.read_script(f)[814] == [(0x93928, 1), (0x93929, 0xFFFFFFFF)]
