"""Tests of the WAVE bank stage model (`0x1247B6`) and the table registry reader."""

from __future__ import annotations

import csv
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import synth_stage_super2 as super2  # noqa: E402
import synth_stage_wave as wave_bank  # noqa: E402
import synth_wave_model as m  # noqa: E402
import synth_wave_oracle as o  # noqa: E402
import synth_wave_registry as reg  # noqa: E402

CATEGORIES = Path(__file__).resolve().parents[2] / "docs/research/data/synth-wave-categories.csv"


def _lanes(seed: int = 3) -> list[m.Lane]:
    import random

    rng = random.Random(seed)
    lanes = o.random_lanes(rng, True)
    wave_bank.random_state(seed, lanes)
    for ln in lanes:
        ln.pitch = m.f2u(0.02)
    return lanes


def test_stage_is_deterministic_and_has_four_words_per_lane() -> None:
    tab = super2.load_tables()
    a = wave_bank.stage_wave(_lanes(), tab)
    b = wave_bank.stage_wave(_lanes(), tab)
    assert a == b
    assert [len(w) for w in a[0]] == [4, 4]
    assert [len(p) for p in a[1]] == [2, 2] and [len(s) for s in a[2]] == [6, 6]


def test_phase_advances_by_four_increments_per_pass() -> None:
    lanes = _lanes()
    _, phases, _ = wave_bank.stage_wave(lanes, super2.load_tables())
    for i, ln in enumerate(lanes):
        _, _, ds = wave_bank.lane_setup(ln)
        assert phases[i][0] == (ln.phase1 + 4 * ds[0]) & 0xFFFFFFFF
        assert phases[i][1] == (ln.phase2 + 4 * ds[1]) & 0xFFFFFFFF


def test_pitch_is_clipped_to_half() -> None:
    ln = _lanes()[0]
    ln.pitch = m.f2u(0.9)
    f, _, _ = wave_bank.lane_setup(ln)
    assert float(f) == 0.5


def test_random_state_keeps_the_ratios_near_one() -> None:
    for ln in _lanes():
        for w in (ln.ratio1, ln.ratio2):
            assert 0.7 < float(m.ld(w)) < 1.45
        assert ln.ratios == (ln.ratio1, ln.ratio2)


def test_oracle_registers_the_stage() -> None:
    assert o.STAGE_WAVE == 0x1247B6
    assert o.STAGE_WAVE in o.MODELS and o.STAGE_WAVE in o.UNISON_STAGES


def _registry_image(names: list[str]) -> bytes:
    """A synthetic code image with a registry of `len(names)` records at `REGISTRY`."""
    base = reg.REGISTRY - reg.LOAD_BASE
    strings_at = base + reg.RECORD_BYTES * (len(names) + 1)
    img = bytearray(strings_at + 64)
    pos = strings_at
    for k, name in enumerate(names):
        img[pos : pos + len(name) + 1] = name.encode() + b"\0"
        rows = reg.LOAD_BASE + 0x1000 * (k + 1)
        rec = (reg.LOAD_BASE + pos, rows + 0x800, 8, rows, 0x200, rows - 0x400, 10 + k)
        struct.pack_into("<7I", img, base + reg.RECORD_BYTES * k, *rec)
        pos += len(name) + 1
    return bytes(img)


def test_registry_parse_reads_names_counts_and_stops_at_a_foreign_record() -> None:
    entries = reg.parse(_registry_image(["Second", "Third", "Blues"]))
    assert [e.name for e in entries] == ["Second", "Third", "Blues"]
    assert [e.count for e in entries] == [10, 11, 12]
    assert entries[1].rows_ptr == reg.LOAD_BASE + 0x2000
    assert "Third,12," in reg.to_csv(entries)


def test_committed_registry_matches_the_documented_totals_and_wave_ids() -> None:
    rows = reg.load_csv()
    assert len(rows) == reg.TOTAL_RECORDS
    assert sum(int(r["samples"]) for r in rows) == reg.TOTAL_SAMPLES
    with CATEGORIES.open(newline="", encoding="utf-8") as fh:
        waves = [r for r in csv.DictReader(fh) if r["type"] == "Wave"]
    assert len(waves) == reg.WAVE_COUNT
    # the registry order is the table-id order: the squeezed name of wave id k is record k
    squeeze = lambda s: "".join(c for c in s.lower() if c.isalnum())  # noqa: E731
    by_id = {int(r["wave_id"]): squeeze(r["name"]) for r in waves}
    hits = sum(1 for r in rows[: reg.WAVE_COUNT] if squeeze(r["name"]) == by_id[int(r["index"])])
    # the 6 others are abbreviations of the same slot (Full Organ+, Square Bright, Clavinet 1..3,
    # Tubular Bells)
    assert hits == 40


def test_every_record_has_aligned_addresses() -> None:
    for r in reg.load_csv():
        assert int(r["data_addr"], 16) % 4 == 0 and int(r["rows_addr"], 16) % 4 == 0


def _image_with_one_table(rows_in: list[int], samples: list[int]) -> tuple[bytes, reg.Entry]:
    """Code image holding the rows and samples of one table; entry points at them."""
    img = bytearray(0x4000)
    rows_at, data_at = 0x1000, 0x2000
    struct.pack_into(f"<{len(rows_in)}I", img, rows_at, *rows_in)
    struct.pack_into(f"<{len(samples)}H", img, data_at, *samples)
    entry = reg.Entry(0, "t", reg.LOAD_BASE + rows_at, reg.LOAD_BASE + data_at, len(samples) - 1)
    return bytes(img), entry


def test_build_l2_relays_bands_with_a_cyclic_guard() -> None:
    # two bands: 4 samples at source offset 0 and 6 samples at 4; all 128 rows use one or the other
    rows_in = [0, 4, 7, 1, 4, 6, 9, 2] * 64
    samples = [10, 20, 30, 40, 1, 2, 3, 4, 5, 6]
    code, entry = _image_with_one_table(rows_in, samples)
    rows, l2 = reg.build_l2(code, entry)
    first, second = 0, 4 + reg.GUARD
    assert rows[0] == first and rows[4] == second and rows[1] == 4 and rows[5] == 6
    assert rows[2:4] == [7, 1] and rows[6:8] == [9, 2]  # spare words stay
    assert l2[:16] == [10, 20, 30, 40] * 4  # band 0 plus 12 cyclic samples
    assert l2[16:22] == [1, 2, 3, 4, 5, 6] and l2[22:] == ([1, 2, 3, 4, 5, 6] * 2)[:10]
    assert len(l2) == 16 + 6 + reg.GUARD - reg.TAIL_TRIM


def test_build_l2_handles_duplicate_rows_with_one_copy() -> None:
    rows_in = [0, 4, 0, 0] * 128
    code, entry = _image_with_one_table(rows_in, [5, 6, 7, 8])
    rows, l2 = reg.build_l2(code, entry)
    assert all(rows[4 * b] == 0 for b in range(128))
    assert len(l2) == 4 + reg.GUARD - reg.TAIL_TRIM


def test_find_registry_takes_the_v166_address_or_scans_for_the_layout(monkeypatch) -> None:
    names = [f"w{k}" for k in range(reg.WAVE_COUNT)]
    assert reg.find_registry(_registry_image(names)) == reg.REGISTRY
    moved = reg.REGISTRY + 0x3A0
    monkeypatch.setattr(reg, "REGISTRY", moved)
    image = _registry_image(names)
    monkeypatch.setattr(reg, "REGISTRY", moved - 0x3A0)
    assert reg.find_registry(image) == moved
    assert reg.find_registry(bytes(0x2000)) == reg.REGISTRY  # nothing found: the default
