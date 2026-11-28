import os
import sys

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import bench_ab as b  # noqa: E402


def test_metric_takes_the_last_match_by_default_and_the_first_on_request():
    out = "pass mean 10.5 us\nfinal mean 12.25 us\n"
    assert b.parse_metric(out, r"mean ([0-9.]+) us") == 12.25
    assert b.parse_metric(out, r"mean ([0-9.]+) us", "first") == 10.5


def test_metric_that_does_not_match_is_an_error():
    with pytest.raises(ValueError):
        b.parse_metric("nothing here", r"mean ([0-9.]+) us")


def test_disjoint_bands_are_a_real_difference_and_overlapping_bands_are_noise():
    a = b.Side("A", [10.0, 10.5, 11.0])
    fast = b.Side("B", [7.0, 7.5, 8.0])
    slow = b.Side("B", [14.0, 15.0, 16.0])
    close = b.Side("B", [10.8, 11.5, 12.0])
    assert "lower" in b.verdict(a, fast)
    assert "higher" in b.verdict(a, slow)
    assert "noise" in b.verdict(a, close)


def test_rounds_alternate_and_swap_the_order_each_round():
    calls = []

    def run(cmd):
        calls.append(cmd)
        return "mean 1.0 us" if cmd == "A" else "mean 2.0 us"

    a, bb, loads = b.run_ab(run, "A", "B", r"mean ([0-9.]+) us", rounds=4, warmup=1, load=lambda: 0)
    assert calls[:2] == ["A", "B"]  # warm-up
    assert calls[2:] == ["A", "B", "B", "A", "A", "B", "B", "A"]
    assert a.values == [1.0] * 4 and bb.values == [2.0] * 4
    assert loads == [0, 0, 0, 0]


def test_report_names_the_rounds_that_overlapped_a_compiler():
    a = b.Side("A", [1.0, 1.1])
    c = b.Side("B", [2.0, 2.1])
    text = b.report(a, c, [0, 2], "us")
    assert "median 1.05 us" in text
    assert "rounds [2]" in text
    assert "none" in b.report(a, c, [0, 0], "us")


def test_compiler_count_matches_process_names_only():
    listing = "cargo.exe 12\nrustc.exe 33\nrustc.exe 40\nexplorer.exe 5\nmycargo.exe 9\n"
    assert b.compiler_count(lambda: listing) == 3
