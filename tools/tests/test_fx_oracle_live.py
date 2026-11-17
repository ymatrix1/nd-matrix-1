"""Tests of tools/fx_oracle_live.py: install scripts, test signals and the comparison metrics.

No firmware image or built binary is needed: the script builders and metrics are pure functions.
"""

import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import fx_oracle_live as fo  # noqa: E402

TYPE_WORDS_RS = """pub(super) const TYPE_WORDS: [[(i8, u32); 39]; 2] = [
    [
        (-18, 0x00000015),
        (-17, 0x3ea48fae),
    ],
    [
        (-18, 0x00000016),
    ],
];
"""


def test_install_writes_the_stage_record_and_master_level():
    words = dict(fo.install("chor4", 0xC13A4))
    assert words[fo.MASTER] == fo.F_ONE
    assert words[fo.REC["fn"]] == fo.ROUTINE["chor4"]
    assert words[fo.REC["input"]] == fo.BUS
    assert words[fo.REC["state"]] == 0xC13A4
    assert words[fo.REC["level"]] == fo.F_HALF


def test_reverb_type_words_are_parsed_per_type():
    assert fo.reverb_type_words(TYPE_WORDS_RS, 0) == [(-18, 0x15), (-17, 0x3EA48FAE)]
    assert fo.reverb_type_words(TYPE_WORDS_RS, 1) == [(-18, 0x16)]
    try:
        fo.reverb_type_words("nothing", 0)
    except ValueError:
        pass
    else:
        raise AssertionError("missing TYPE_WORDS must raise")


def test_reverb_script_clears_buffers_then_sets_type_and_amount_words():
    words = fo.reverb0_words(TYPE_WORDS_RS, send=1.0, dry=0.0)
    cleared = sum(n for _, n in fo.REVERB_REGIONS)
    assert all(w == 0 for _, w in words[:cleared])
    d = dict(words)
    assert d[fo.REVERB_I1 - 18] == 0x15
    assert d[fo.REVERB_I1 + 0x12] == fo.f32(1.0)
    assert d[fo.REVERB_I1 + 0x13] == 0
    assert d[fo.REVERB_I1 + 0x14] == fo.f32(1.0)
    assert d[fo.REC["fn"]] == fo.ROUTINE["reverb0"]


def test_chorus_context_depends_on_amount_and_zeroes_the_ring():
    lo, hi = dict(fo.chor4_words(64, 0)), dict(fo.chor4_words(64, 127))
    assert lo[fo.CHOR_I1 + 9] == 2 and hi[fo.CHOR_I1 + 9] == 2
    assert lo[fo.CHOR_I1 - 3] == 0  # no mix at amount 0
    assert hi[fo.CHOR_I1 - 3] != 0
    assert all(hi[fo.CHOR_RING + k] == 0 for k in range(0x410))
    assert hi[fo.CHOR_I1 - 0x20] == 12800 + 4096 * 64


def test_script_text_is_one_triple_per_line():
    text = fo.script_text([(0xB6518, 0x3F800000), (0x10, 5)])
    assert text == "0 0xB6518 0x3F800000\n0 0x10 0x00000005\n"


def test_signals_are_deterministic_and_shaped():
    n = 20000
    for kind in ("sine", "burst", "noise", "impulse"):
        a, b = fo.signal(kind, n), fo.signal(kind, n)
        assert a.shape == (n, 2) and np.array_equal(a, b)
    assert np.abs(fo.signal("burst", n)[int(0.3 * fo.FRAME_RATE) :]).max() == 0
    assert fo.signal("impulse", 8)[0, 0] == 0.5
    try:
        fo.signal("nope", 4)
    except ValueError:
        pass
    else:
        raise AssertionError("unknown signal must raise")


def test_wav_round_trip(tmp_path):
    x = fo.signal("noise", 64)
    fo.write_wav(tmp_path / "x.wav", x)
    assert np.array_equal(fo.read_wav(tmp_path / "x.wav"), x)


def test_identical_signals_null_deeply_and_a_gain_is_fitted():
    x = fo.signal("noise", 4000)
    same = fo.compare(x, x, fo.FRAME_RATE, 100)
    assert same["lag_frames"] == 0 and same["null_unity_db"] < -120
    scaled = fo.compare(0.25 * x, x, fo.FRAME_RATE, 100)
    assert abs(scaled["fit_gain"] - 0.25) < 1e-6 and scaled["null_fit_db"] < -120
    assert abs(scaled["rms_delta_db"] - fo.db(0.25)) < 1e-3


def test_best_lag_finds_a_shift_and_tail_length_counts_the_decay():
    x = fo.signal("noise", 3000)[:, 0]
    shifted = np.concatenate([np.zeros(5, np.float32), x])[: len(x)]
    assert fo.best_lag(shifted, x) == 5
    env = np.exp(-np.arange(20000) / 2000.0).astype(np.float32) * np.sin(np.arange(20000))
    assert 0.0 < fo.tail_ms(env, 0, fo.FRAME_RATE) < 1000.0
    assert fo.tail_ms(np.zeros(100, np.float32), 0, fo.FRAME_RATE) == 0.0


def test_centroid_of_a_tone_is_its_frequency():
    t = np.arange(8192)
    tone = np.sin(2 * np.pi * 1000 * t / fo.FRAME_RATE)
    assert abs(fo.centroid(tone, fo.FRAME_RATE) - 1000) < 20
    assert fo.centroid(np.zeros(64), fo.FRAME_RATE) == 0.0


def test_null_chain_points_head_link_and_later_headers_at_one_stub_record():
    words = dict(fo.null_chain())
    s = fo.NULL_REC + 0x20
    assert words[fo.HEAD_LINK] == s
    assert all(words[h] == s for h in fo.GROUP_HEADERS[1:])
    assert fo.GROUP_HEADERS[0] not in words
    assert words[fo.NULL_REC] == fo.NULL_STUB
    assert words[fo.NULL_REC + 3] == s  # the stub links to itself


def test_chor5_words_install_the_variation_routine_and_zero_the_ring():
    words = fo.chor5_words()
    d = dict(words)
    assert d[fo.REC["fn"]] == fo.ROUTINE["chor5"]
    assert d[fo.REC["state"]] == fo.CHOR_I1
    assert all(d[fo.CHOR_RING + k] == 0 for k in range(0x410))
    assert d[fo.CHOR_I1 + 6] == fo.CHOR5_CTX[6]
