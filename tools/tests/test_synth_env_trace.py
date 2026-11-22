"""Unit tests for tools/synth_env_trace.py (knob words, rate index inversion, chain facts)."""

from __future__ import annotations

import numpy as np
import pytest
import synth_env_trace as t


def test_rate_word_is_the_index_in_the_top_byte() -> None:
    assert t.rate_word(64) == 0
    assert t.rate_word(0) == 0xC0000000  # index -64
    assert t.rate_word(22) == (-42 & 0xFF) << 24
    assert t.rate_word(127) == 63 << 24
    assert t.rate_word(500) == 63 << 24  # knob clipped to 127


def test_cutoff_word_follows_the_knob_laws_table() -> None:
    assert t.cutoff_word(0) == (-52 & 0xFF) << 24 == 0xCC000000  # csv: knob 0 -> 0xcc000000
    assert t.cutoff_word(57) == 5 << 24
    assert t.cutoff_word(117) == 65 << 24
    assert t.cutoff_word(118) == 0x42B33300
    assert t.cutoff_word(127) == 0x6A000000


def test_f32_bits_and_program_words_cover_adr_cutoff_and_amount() -> None:
    assert t.f32_bits(1.0) == 0x3F800000
    w = t.program_words({"amp": [22, 127, 57], "filter": [29, 120, 50]}, 57, 127)
    assert w[t.ADR_BASE] == t.rate_word(22)
    assert w[t.ADR_BASE + 2] == t.rate_word(57)
    assert w[t.ADR_BASE + 3] == t.rate_word(29)
    assert w[t.ADR_BASE + 5] == t.rate_word(50)
    assert t.ADR_BASE + 6 not in w  # oscillator ADR keeps the image value
    assert w[t.COMMON_CUTOFF] == 5 << 24
    assert w[t.COMMON_FLT_ENV_AMOUNT] == 0x3F800000


def test_inject_script_is_sorted_flat_lines() -> None:
    s = t.inject_script({0xE001B: 0x1, 0xE001A: 0xFFFFFFFF})
    assert s == "0 0xe001a 0xffffffff\n0 0xe001b 0x00000001\n"
    assert t.inject_script({0x10: 2}, at_pass=7) == "7 0x10 0x00000002\n"


@pytest.fixture(scope="module")
def tables() -> tuple[dict[int, float], dict[int, float]]:
    return t.load_rate_table()


def test_rate_table_spans_minus_64_to_64(tables) -> None:
    att, dec = tables
    assert min(att) == -64 and max(att) == 64
    assert att[-64] == pytest.approx(0.092161648)
    assert dec[-64] == pytest.approx(0.931707144)
    assert att[-64] > att[0] > att[63]


def test_attack_index_inverts_the_table(tables) -> None:
    att, _ = tables
    assert t.attack_index(att[0], att) is not None
    for idx in (-64, -30, 0, 30, 63):
        got = t.attack_index(att[idx], att)
        assert att[got] == att[idx]  # equal value: the table repeats over plateaus
    assert t.attack_index(0.0, att) is None


def test_release_index_inverts_the_table(tables) -> None:
    _, dec = tables
    for idx in (-64, -10, 20, 63):
        got = t.release_index(dec[idx], dec)
        assert dec[got] == dec[idx]
    assert t.release_index(1.0, dec) is None
    assert t.release_index(0.0, dec) is None


def synthetic_chain(att: dict, dec: dict, a_idx: int, r_idx: int) -> np.ndarray:
    """Linear ramp to 1 at `att[a_idx]` per pass from pass 300, hold, release at 600."""
    y = np.zeros(900)
    for p in range(301, 900):
        if p < 600:
            y[p] = min(1.0, y[p - 1] + att[a_idx])
        else:
            y[p] = y[p - 1] * dec[r_idx]
    return y


def test_chain_facts_recovers_attack_and_release_index(tables) -> None:
    att, dec = tables
    y = synthetic_chain(att, dec, -55, -50)
    f = t.chain_facts(y, 300, 600, att, dec)
    assert f["first_rise_pass"] == 301
    assert att[f["attack_index"]] == att[-55]
    assert dec[f["release_index"]] == dec[-50]
    assert f["peak"] == pytest.approx(1.0)
    assert f["level_at_off"] == pytest.approx(1.0)


def test_chain_facts_of_a_silent_chain_has_no_rise(tables) -> None:
    att, dec = tables
    f = t.chain_facts(np.zeros(900), 300, 600, att, dec)
    assert f["first_rise_pass"] is None
    assert f["peak"] == 0.0
    assert f["release_index"] is None


def test_r_points_and_compare_native() -> None:
    r = np.concatenate([np.full(300, 0.1), np.linspace(0.1, 0.5, 300), np.full(300, 0.2)])
    p = t.r_points(r, 300, 600, t.SY_FRAME_RATE_HZ)
    assert p["before"] == pytest.approx(0.1)
    assert p["max"] == pytest.approx(0.5)
    assert p["max_pass"] == 599
    assert p["released"] == pytest.approx(0.2)
    assert p["max_hz"] == pytest.approx(t.r_to_hz(0.5, t.SY_FRAME_RATE_HZ))
    row = t.compare_native(
        {"attack_index": -32, "release_index": None, "level_at_off": 0.9, "peak": 1.0},
        {"attack_index": -42, "release_index": -3, "level_at_off": 1.0, "peak": 1.0},
    )
    assert row["attack_index"] == 10
    assert row["release_index"] is None
    assert row["level_at_off"] == pytest.approx(-0.1)


def hz_block(before: float, mx: float, hold: float, rel: float) -> dict:
    return {
        "before_hz": before, "max_hz": mx, "end_of_hold_hz": hold, "released_hz": rel,
    }  # fmt: skip


def test_r_to_hz_inverts_the_tan_law() -> None:
    rate = t.ORACLE_STAGE_RATE_HZ
    r = float(np.tan(np.pi * 440.0 / rate))
    assert t.r_to_hz(r, rate) == pytest.approx(440.0)
    assert rate == pytest.approx(86_804.6875)


def test_csv_rows_pair_oracle_and_native_values() -> None:
    chain = {
        "oracle": {"attack_index": -32, "release_index": -5, "peak": 1.0,
                   "level_at_off": 0.954, "release_ratio": 0.99977},
        "native": {"attack_index": -42, "release_index": -10, "peak": 1.0,
                   "level_at_off": 1.0, "release_ratio": 0.99971},
    }  # fmt: skip
    rep = {
        "program": "X",
        "knobs": {"amp": [22, 127, 57], "filter": [29, 120, 50]},
        "chains": {"amp": chain, "filter": chain},
        "cutoff": {
            "cutoff_knob": 57,
            "filter_env_amount_knob": 31.0,
            "oracle_r": hz_block(369.0, 2200.0, 1980.0, 450.0),
            "native_r": hz_block(370.0, 1500.0, 1310.0, 820.0),
        },
    }
    rows, cut = t.csv_rows(rep)
    assert [r["chain"] for r in rows] == ["amp", "filter"]
    assert rows[0]["attack_index_word"] == 22 - 64
    assert rows[0]["attack_index_oracle"] == -32 and rows[0]["attack_index_native"] == -42
    assert rows[1]["release_index_word"] == 50 - 64
    assert cut["hz_before_oracle"] == 369.0 and cut["hz_released_native"] == 820.0
    assert set(rows[0]) == set(t.CSV_FIELDS) and set(cut) == set(t.CUTOFF_FIELDS)


def test_key_byte_script_rewrites_only_note_on_velocity_byte() -> None:
    on = "5 0xe0000 0x0b00a001"
    other = "6 0xe0001 0x01001234"
    out = t.key_byte_script(on + "\n" + other + "\n", 0x10)
    lines = out.splitlines()
    assert lines[0] == "5 0xe0000 0x0b009001"  # byte 1 = 0x80 + 0x10
    assert lines[1] == "6 0xe0001 0x01001234"  # not a note-on word: unchanged
    assert out.endswith("\n")


def test_effective_index_is_exact_on_the_table_and_interpolates_between(tables) -> None:
    att, dec = tables
    for idx in (-60, -20, 30):
        got = t.effective_index(att[idx], att, rising=True)
        assert att[int(round(got))] == pytest.approx(att[idx], rel=1e-2)
    mid = np.sqrt(att[-20] * att[-19])
    got = t.effective_index(mid, att, rising=True)
    assert -20.0 <= got <= -19.0
    d = t.effective_index(dec[10], dec, rising=False)
    assert abs(dec[int(round(d))] - dec[10]) < 1e-3


def test_knob_of_r_uses_the_ramp_for_the_ladder_and_the_exponential_law_otherwise() -> None:
    # oracle taps of cutoff knob 81 (synth-cutoff-terms-oracle.csv): ladder 0.5908, others 0.0536
    assert t.knob_of_r(0.5908, True) == pytest.approx(81.0, abs=0.05)
    assert t.knob_of_r(0.0536, False) == pytest.approx(81.0, abs=0.1)
    assert t.knob_of_hz(13.75) == pytest.approx(0.0)
    assert t.knob_of_hz(440.0) == pytest.approx(60.0)


def test_stage_word_and_key_note_offset_read_the_recall_and_key_script() -> None:
    rows = [
        {"phase": "load", "chip": "ORG", "core": 2, "dm": t.STRIP_WORD1, "w2": 5},
        {"phase": "idle", "chip": "ORG", "core": 2, "dm": t.STRIP_WORD1, "w2": 0x8612549D},
        {"phase": "load", "chip": "ORG", "core": 1, "dm": t.STRIP_WORD1, "w2": 7},
    ]
    assert t.stage_word(rows) == 0x8612549D
    assert t.stage_word([]) is None
    assert t.key_note_offset("2000 0xc1dcc 0x0bf4e200\n") == -12
    assert t.key_note_offset("2000 0xc1dcc 0x0b0ce200\n") == 12
    assert t.key_note_offset("2000 0xc1dcc 0x08f4e200\n") == 0  # note-off word only


def test_index_rows_and_csv_text() -> None:
    pt = {"before": 41.0, "max": 80.0, "end_of_hold": 79.0, "released": 70.0}
    rep = {
        "program": "X",
        "cutoff_index": {
            "ladder": True, "note_offset": -12, "oracle": pt,
            "native_default": {k: v + 1 for k, v in pt.items()},
            "native_fw_law": {k: v - 1 for k, v in pt.items()},
        },
    }  # fmt: skip
    rows = t.index_rows(rep)
    assert [r["point"] for r in rows] == ["before", "max", "end_of_hold", "released"]
    assert rows[0]["stage"] == "ladder" and rows[0]["err_default"] == 1.0
    assert rows[1]["err_fw_law"] == -1.0
    text = t.write_index_csv([rep])
    assert text.splitlines()[0].startswith("program,stage,note_offset")
    assert len(text.splitlines()) == 5


def test_knob_points_picks_before_max_end_and_last() -> None:
    k = np.array([0.0, 1.0, 5.0, 3.0, 2.0, 1.0])
    p = t.knob_points(k, 2, 4)
    assert p == {"before": 1.0, "max": 5.0, "end_of_hold": 3.0, "released": 1.0}
