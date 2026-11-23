"""Unit tests for tools/arm_smp_flash.py (synthetic `.npno` tables, no firmware, no subprocess)."""

from __future__ import annotations

import math
import struct

import arm_smp_flash as sf


def synthetic_npno(channels: int, strokes: list[tuple[int, int, int]]) -> bytes:
    """Header tables only: `(offset_from_cnsp_base, frames, npages)` per stroke."""
    data = bytearray(sf.STROKE_TABLE + len(strokes) * sf.STROKE_SIZE)
    struct.pack_into(">H", data, sf.CHANNELS_AT, channels)
    struct.pack_into(">H", data, sf.NSTROKES_AT, len(strokes))
    for k, (off, frames, npages) in enumerate(strokes):
        at = sf.STROKE_TABLE + k * sf.STROKE_SIZE
        struct.pack_into(">I", data, at + 8, off)
        struct.pack_into(">I", data, at + 0x0E, frames)
        struct.pack_into(">H", data, at + 0x12, npages)
    return bytes(data)


class FakeLink:
    def __init__(self) -> None:
        self.sent: list[str] = []

    def command(self, line: str) -> str:
        self.sent.append(line)
        return "F ok"


# the two Astoria Grand strokes of the run (file offset, frames, pages) and their rec[-10]
ASTORIA = [(82_933_064, 162_532, 99, 0x9E7E), (80_929_944, 153_593, 93, 0x9AAA)]


def test_parse_strokes_reads_offset_frames_and_pages() -> None:
    npno = synthetic_npno(2, [(a, b, c) for a, b, c, _ in ASTORIA])
    got = sf.parse_strokes(npno)
    assert [(s.offset, s.frames, s.npages, s.channels) for s in got] == [
        (a + 0x2C, b, c, 2) for a, b, c, _ in ASTORIA
    ]


def test_the_page_law_reproduces_the_observed_descriptors() -> None:
    strokes = sf.parse_strokes(synthetic_npno(2, [(a, b, c) for a, b, c, _ in ASTORIA]))
    assert [sf.stroke_page(s) for s in strokes] == [v for *_, v in ASTORIA]
    # the file byte of page v is 2044 * v - 148 (stereo)
    assert [2044 * sf.stroke_page(s) - 148 for s in strokes] == [s.offset for s in strokes]


def test_a_stroke_off_the_grid_has_no_page() -> None:
    npno = synthetic_npno(2, [(10, 5, 1)])
    assert sf.stroke_page(sf.parse_strokes(npno)[0]) is None


def test_mono_pages_use_the_1022_byte_grid() -> None:
    off = 1022 * 7 - 148 - sf.CNSP_BASE
    s = sf.parse_strokes(synthetic_npno(1, [(off, 9, 1)]))[0]
    assert sf.stroke_page(s) == 7


def test_a_descriptor_in_the_message_stream_places_the_file_once() -> None:
    npno = synthetic_npno(2, [(a, b, c) for a, b, c, _ in ASTORIA])
    link = FakeLink()
    feed = sf.SmpFlashFeed(sf.parse_strokes(npno), link)
    assert feed.observe(0x9391D, 0x6E6F) is None
    assert feed.observe(0x9391E, 0x9E7E) is None
    det = feed.observe(0x9391F, 162_532)
    assert det and det.stroke == 0 and det.law_ok and det.rec11 == 0x6E6F and det.placed == "F ok"
    # the second stroke of the same file: same directory index, no second F
    feed.observe(0x93953, 0x6E6F)
    feed.observe(0x93954, 0x9AAA)
    det2 = feed.observe(0x93955, 153_593)
    assert det2 and det2.stroke == 1 and det2.law_ok and det2.placed == ""
    assert link.sent == [f"F {0x6E6F}"]
    assert feed.summary()["descriptors"] == 2 and feed.summary()["law_ok"] == 2


def test_a_frame_count_with_a_wrong_page_is_reported_not_placed() -> None:
    npno = synthetic_npno(2, [(a, b, c) for a, b, c, _ in ASTORIA])
    link = FakeLink()
    feed = sf.SmpFlashFeed(sf.parse_strokes(npno), link)
    feed.observe(0x100, 5)
    feed.observe(0x101, 12345)
    det = feed.observe(0x102, 162_532)
    assert det and not det.law_ok and link.sent == []


def test_expected_head_is_the_payload_after_the_two_header_bytes() -> None:
    npno = bytearray(synthetic_npno(1, [(0x1000, 4, 1)]))
    at = 0x1000 + sf.CNSP_BASE
    npno += bytes(at + 2 + 16 - len(npno))
    for k in range(8):
        struct.pack_into(">H", npno, at + 2 + 2 * k, 0x100 + k)
    s = sf.parse_strokes(bytes(npno))[0]
    assert sf.expected_head(bytes(npno), s) == [0x100 + k for k in range(8)]


def f32(values: list[float]) -> bytes:
    return struct.pack(f"<{len(values)}f", *values)


def test_tap_stats_tells_silence_a_stuck_sample_and_audio_apart() -> None:
    assert sf.tap_stats(b"")["kind"] == "silent"
    assert sf.tap_stats(f32([0.0] * 100))["kind"] == "silent"
    plateau = sf.tap_stats(f32([0.0431] * 5000))  # the D191 clavinet DC plateau
    assert plateau["kind"] == "plateau" and plateau["distinct_in_loudest_window"] == 1
    tone = [0.2 * math.sin(0.01 * k) for k in range(5000)]
    audio = sf.tap_stats(f32(tone))
    assert audio["kind"] == "audio"
    assert abs(audio["peak"] - 0.2) < 1e-3 and abs(audio["peak_dbfs"] - (-13.98)) < 0.05


def test_tap_fundamental_finds_the_pitch_of_a_harmonic_tone_and_none_for_silence() -> None:
    cycles = 0.006
    tone = []
    for k in range(9000):
        v = sum(math.sin(2 * math.pi * cycles * h * k) / h for h in (1, 2, 3))
        tone += [0.2 * v, 0.2 * v]
    got = sf.tap_fundamental(f32(tone))
    assert got is not None and abs(got - cycles) < 2e-5
    assert sf.tap_fundamental(f32([0.0] * 20000)) is None
    assert sf.tap_fundamental(f32([0.1] * 100)) is None


def _frames(values: list[float]) -> bytes:
    return f32([v for x in values for v in (x, x)])


def test_tap_fundamental_keeps_the_octave_of_a_weak_fundamental() -> None:
    """D230: a harpsichord-like tap (strong even harmonics) read twice its pitch with the harmonic
    sum; the first autocorrelation maximum is the real period."""
    cycles = 0.006
    amps = {1: 0.7, 2: 1.0, 3: 0.2, 4: 0.9, 6: 0.5}
    tone = [
        sum(a * math.sin(2 * math.pi * cycles * h * k + h) for h, a in amps.items())
        for k in range(6000)
    ]
    got = sf.tap_fundamental(_frames(tone))
    assert got is not None and abs(got - cycles) < 1e-4


def test_tap_fundamental_ignores_slow_drift_and_a_late_onset() -> None:
    """D230: a decaying tone after a silent lead-in with a slow drift gave the lowest candidate
    (0.0003) before; the window now starts at the onset and the lag range has a floor."""
    cycles = 0.012
    tone = [0.0] * 1500
    for k in range(4500):
        tone.append(
            0.5 * math.exp(-k / 3000) * math.sin(2 * math.pi * cycles * k)
            + 0.05 * math.sin(2 * math.pi * 0.0004 * k)
        )
    got = sf.tap_fundamental(_frames(tone))
    assert got is not None and abs(got - cycles) < 2e-4


def test_tap_fundamental_follows_the_octave_of_the_tone() -> None:
    """One octave up and down gives 2x and 0.5x (the layer octave of the program, D230)."""
    out = []
    for cycles in (0.003, 0.006, 0.012):
        tone = [
            math.sin(2 * math.pi * cycles * k) + 0.4 * math.sin(4 * math.pi * cycles * k)
            for k in range(6000)
        ]
        got = sf.tap_fundamental(_frames(tone))
        assert got is not None
        out.append(got)
    assert abs(out[1] / out[0] - 2) < 0.03 and abs(out[2] / out[1] - 2) < 0.03
