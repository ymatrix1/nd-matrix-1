"""Unit tests for tools/arm_dsp_link.py (no firmware, no unicorn, no Rust binary)."""

from __future__ import annotations

import io
import random

import arm_dsp_link as dl
import arm_emu as ae
import pytest


def test_passes_per_tick_is_the_exact_ratio_of_the_two_clocks() -> None:
    # F = 43402.34375 Hz, two frames per pass, ARM tick 24 kHz (D130)
    assert dl.PASS_NUM / dl.PASS_DEN == pytest.approx(43402.34375 / 2 / 24000)
    assert 43402.34375 * 32 == 1_388_875
    assert dl.passes_due(0) == 0
    assert dl.passes_due(1536) == 1388  # 1536 ticks = 64 ms = 1388.875 passes of 46.08 us (floor)


def test_clock_catch_up_is_independent_of_the_step_size() -> None:
    a, b = dl.Clock(), dl.Clock()
    one = sum(a.tick() for _ in range(5000))
    chunks = sum(b.tick(c) for c in [7, 993, 1500, 2500])
    assert one == chunks == dl.passes_due(5000) == a.passes == b.passes
    assert a.ticks == b.ticks == 5000


def _ports() -> dict[int, tuple[int, int, int]]:
    return dl.port_table(ae.PORT2_BASE, ae.PORT1_BASE, ae.COMM_COUNT, (1, 1, 1, 0, 0))


def _writes(rng: random.Random, n: int) -> list[tuple[int, int, int]]:
    ports = list(_ports())
    return [
        (rng.choice(ports + [0x1234]), rng.randrange(1 << 32), 0xC0000000 + i) for i in range(n)
    ]


def test_incremental_grouping_equals_the_batch_grouping_for_any_split() -> None:
    rng = random.Random(7)
    writes = _writes(rng, 600)
    whole = ae.group_port_writes(writes, (1, 1, 1, 0, 0))
    want = [(m.path, m.comm, m.word1, m.word2, m.pc) for m in whole]
    g = dl.PortGrouper(_ports())
    got, i = [], 0
    while i < len(writes):
        step = rng.randrange(1, 9)
        got += g.feed(writes[i : i + step])
        i += step
    assert got == want and want, "split feeds must reproduce the batch result"


def test_partial_group_waits_for_the_next_feed() -> None:
    port = ae.PORT2_BASE + 0x10  # comm 1, pad 1: (address, pad, value)
    g = dl.PortGrouper(_ports())
    assert g.feed([(port, 0x28B00000, 1), (port, 0, 2)]) == []
    assert g.feed([(port, 0xAB, 3)]) == [(2, 1, 0x28B00000, 0xAB, 3)]


REPLY = (
    "P req=4 idle=4 steps=100 last=25 pass=4 halted=0 cnt=16 sumsq=2.5e-1 peak=5e-1 "
    "hits=1256ad:3,121e2b:4 error=x"
)


def test_pass_reply_parses_every_field() -> None:
    r = dl.PassReport.parse(REPLY)
    assert (r.req, r.idle, r.steps, r.last, r.pass_, r.halted) == (4, 4, 100, 25, 4, False)
    assert (r.cnt, r.sumsq, r.peak) == (16, 0.25, 0.5)
    assert r.hits == {0x1256AD: 3, 0x121E2B: 4} and r.error == "x"
    with pytest.raises(ValueError):
        dl.PassReport.parse("D b0200 1")


class FakeProc:
    """Stands in for the `sharc-live` process: records stdin, serves canned stdout lines."""

    def __init__(self, replies: list[str]) -> None:
        self.stdin = io.StringIO()
        self.stdout = io.StringIO("READY\n" + "".join(r + "\n" for r in replies))
        self.waited = False

    def wait(self, timeout: int = 0) -> int:  # noqa: ARG002
        self.waited = True
        return 0


def test_link_batches_messages_before_the_pass_request_and_logs_the_pass_index() -> None:
    proc = FakeProc(
        [
            REPLY,
            "P req=2 idle=2 steps=9 last=4 pass=6 halted=0 cnt=8 sumsq=1e0 "
            "peak=9e-1 hits=1256ad:0,121e2b:2",
            "D b0200 1234 0",
        ]
    )
    link = dl.Link(proc)
    link.send(0xB0200, 0x1234)
    link.send(0x95471, 0x3F800000)
    r1 = link.run(4)
    link.send(0xB2F25, 7)
    r2 = link.run(2)
    assert proc.stdin.getvalue() == (
        "M 0xb0200 0x1234\nM 0x95471 0x3f800000\nP 4\nM 0xb2f25 0x7\nP 2\n"
    )
    assert (r1.pass_, r2.pass_) == (4, 6) and link.pass_index == 6
    assert link.log == [(0, 0xB0200, 0x1234), (0, 0x95471, 0x3F800000), (4, 0xB2F25, 7)]
    s = link.stats
    assert (s.messages, len(s.words), s.idle, s.cnt) == (3, 3, 6, 24)
    assert s.hits == {0x1256AD: 3, 0x121E2B: 6}
    assert s.first_hit == {0x1256AD: 0, 0x121E2B: 0}
    assert s.rms == pytest.approx((1.25 / 24) ** 0.5) and s.peak == 0.9
    assert link.dump(0xB0200, 2) == [0x1234, 0]
    link.close()
    assert proc.stdin.getvalue().endswith("Q\n") and proc.waited


def test_first_hit_records_the_pass_a_probe_first_ran_in() -> None:
    s = dl.LinkStats()
    s.add(dl.PassReport(hits={5: 0}), 0)
    s.add(dl.PassReport(hits={5: 2}), 10)
    s.add(dl.PassReport(hits={5: 1}), 20)
    assert s.hits[5] == 3 and s.first_hit[5] == 10


def test_link_refuses_a_server_that_does_not_say_ready() -> None:
    proc = FakeProc([])
    proc.stdout = io.StringIO("boom\n")
    with pytest.raises(RuntimeError):
        dl.Link(proc)


def test_error_lines_raise_for_p_and_d() -> None:
    proc = FakeProc(["ERR bad command", "ERR bad command"])
    link = dl.Link(proc)
    with pytest.raises(RuntimeError):
        link.run(1)
    with pytest.raises(RuntimeError):
        link.dump(0, 1)


def test_a_silent_server_times_out_instead_of_hanging() -> None:
    import os

    r, w = os.pipe()  # a pipe that never closes: readline blocks like a wedged server
    os.write(w, b"READY" + bytes([10]))

    class Silent:
        stdin = io.StringIO()
        stdout = os.fdopen(r, "r")

    link = dl.Link(Silent(), timeout=0.3)
    with pytest.raises(TimeoutError):
        link.run(1)  # no reply line follows READY
    os.close(w)


def test_close_survives_a_dead_pipe_and_kills_a_server_that_will_not_end() -> None:
    import subprocess

    class Dead(FakeProc):
        def __init__(self) -> None:
            super().__init__([])
            self.killed = False
            self.stdin = None  # writing raises AttributeError-free? use a closed stream instead

        def wait(self, timeout: int = 0) -> int:
            raise subprocess.TimeoutExpired("x", timeout)

        def kill(self) -> None:
            self.killed = True

    proc = Dead()
    proc.stdin = io.StringIO()
    proc.stdin.close()  # ValueError on write
    link = dl.Link(proc)
    link.close()
    assert proc.killed


def test_script_text_is_a_msg_script_in_send_order() -> None:
    assert dl.script_text([(0, 0xB0200, 0x12), (3, 0x95471, 0x3F800000)]) == (
        "0 0xb0200 0x00000012\n3 0x95471 0x3f800000\n"
    )
