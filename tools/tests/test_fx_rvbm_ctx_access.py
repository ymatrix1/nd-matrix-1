"""Tests for tools/fx_rvbm_ctx_access.py: context words the multi-tap reverb module touches."""

import csv
from pathlib import Path

import fx_rvbm_ctx_access as ca

DATA = Path(__file__).resolve().parents[2] / "docs/research/data/fx-rvbm-ctx-access.csv"

LISTING = """\
0x241b18 15b   9208 01e0       R3 = DM(-32, I1)
0x241b20 4a    663f 0913 0553  R2 = DM(-31, I3), F5 = F5 * F3
0x241b2e 15b   9708 02e3       DM(-29, I3) = R5
0x241b58 15b   9808 12e2       M5 = DM(-30, I4)
0x242390 15b   9208 0164       R2 = DM(-28, I1)
0x300000 15b   9208 0164       R2 = DM(-28, I1)
"""


def test_scan_reads_context_loads_and_stores_only():
    rows = ca.scan(LISTING)
    assert [(r["routine"], r["kind"], r["base"], r["offset"]) for r in rows] == [
        ("core_0x120d8a", "load", "I1", -32),
        ("core_0x120d8a", "load", "I3", -31),
        ("core_0x120d8a", "store", "I3", -29),
        ("second_0x1211c1", "load", "I1", -28),
    ]  # I4 rows and lines outside the three routines are skipped


def test_scan_reports_pcs_as_instruction_addresses():
    assert ca.scan(LISTING)[0]["pc"] == "0x120d8c"


def test_routine_ranges_are_contiguous():
    ends = [hi for _n, _lo, hi in ca.ROUTINES[:-1]]
    starts = [lo for _n, lo, _hi in ca.ROUTINES[1:]]
    assert ends == starts


def _table() -> list[dict[str, object]]:
    rows = list(csv.DictReader(DATA.open()))
    for r in rows:
        r["offset"] = int(r["offset"])
    return rows


def test_the_send_target_is_read_and_the_dry_word_is_read_by_nobody():
    rows = _table()
    assert -31 in ca.offsets(rows, "core_0x120d8a", "load")  # send target, smoothed into c[-29]
    assert -29 in ca.offsets(rows, "core_0x120d8a", "store")
    for name, _lo, _hi in ca.ROUTINES:
        assert -30 not in ca.offsets(rows, name, "load")  # kRvbAmt dry word


def test_wrapper_and_second_routine_read_the_same_three_words():
    rows = _table()
    assert ca.offsets(rows, "wrapper_0x120fdb", "load") == {-28, 42, 43}
    assert ca.offsets(rows, "second_0x1211c1", "load") == {-28, 42, 43}
    assert not ca.offsets(rows, "wrapper_0x120fdb", "store")
