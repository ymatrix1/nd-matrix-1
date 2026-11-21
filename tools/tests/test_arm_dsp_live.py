"""Unit tests for tools/arm_dsp_live.py (the pure parts; the firmware run needs local files)."""

from __future__ import annotations

import io
import types
from pathlib import Path

import arm_dsp_link as dl
import arm_dsp_live as live
import arm_dsp_live_parts as parts
import arm_dsp_route as rt
import arm_emu as ae


class FakeEmu:
    """Only what `Feeder` touches: `pad` and the list of port stores."""

    def __init__(self) -> None:
        self.pad = (1, 1, 1, 0, 0)
        self.port_writes: list[tuple[int, int, int]] = []


class Proc:
    def __init__(self) -> None:
        self.stdin = io.StringIO()
        self.stdout = io.StringIO("READY\n")


def test_feeder_sends_only_wired_chips_and_counts_the_rest() -> None:
    emu, link = FakeEmu(), dl.Link(Proc())
    router = rt.Router()
    router.add(rt.parse_chip_spec("PNO:1=pno=i.bin"), link)
    f = live.Feeder(emu, router)
    pno, org = ae.PORT2_BASE + 0x10, ae.PORT2_BASE + 0x20  # comm 1 = PNO, comm 2 = ORG
    # PNO core 1: byte address 0x28000000 + 4 * 0xB0200, pad byte 0, value 0x1234
    emu.port_writes += [(pno, 0x28000000 + 4 * 0xB0200, 1), (pno, 0, 2), (pno, 0x1234, 3)]
    # PNO core 2 (0x28800000 base) and an ORG message: counted, not sent
    emu.port_writes += [(pno, 0x28800000 + 4 * 5, 4), (pno, 0, 5), (pno, 9, 6)]
    emu.port_writes += [(org, 0x28000000, 7), (org, 0, 8), (org, 1, 9)]
    emu.port_writes += [(pno, 0x28000000 + 4 * 6, 10)]  # partial group waits
    f.feed()
    assert link.log == [(0, 0xB0200, 0x1234)]
    assert router.seen == {"PNO1p2": 1, "PNO2p2": 1, "ORG1p2": 1}
    assert emu.port_writes == []
    emu.port_writes += [(pno, 0, 11), (pno, 0x55, 12)]
    f.feed()
    assert link.log[-1] == (0, 6, 0x55) and router.seen["PNO1p2"] == 2


def test_two_chips_get_their_own_messages_and_run_the_same_passes() -> None:
    emu = FakeEmu()
    a, b = dl.Link(Proc()), dl.Link(Proc())
    router = rt.Router()
    sa, sb = rt.parse_chip_spec("ORG:2=org=a.bin=,=,"), rt.parse_chip_spec("PNO:2=org=b.bin")
    router.add(sa, a)
    router.add(sb, b)
    f = live.Feeder(emu, router)
    org, pno = ae.PORT2_BASE + 0x20, ae.PORT2_BASE + 0x10
    emu.port_writes += [(org, 0x28800000 + 4 * 0xC1DCC, 1), (org, 0, 2), (org, 5, 3)]
    emu.port_writes += [(pno, 0x28800000 + 4 * 0xC1DCC, 4), (pno, 0, 5), (pno, 7, 6)]
    f.feed()
    assert a.log == [(0, 0xC1DCC, 5)] and b.log == [(0, 0xC1DCC, 7)]


def test_replay_command_uses_the_lane_for_pno_and_a_tap_for_the_organ_program() -> None:
    pno = rt.parse_chip_spec("PNO:1=pno=p.bin")
    c = live.replay_cmd(pno, Path("sys"), Path("l.script"), 9, Path("t"))
    assert "--pno" in c and c[c.index("--passes") + 1] == "9" and "--tap" not in c
    org = rt.parse_chip_spec("ORG:2=org=o.bin=0xC1F83:4:2")
    c = live.replay_cmd(org, Path("sys"), Path("l.script"), 9, Path("t"))
    assert "--pno" not in c and c[c.index("--tap") + 1] == "0xc1f83:8"


def test_specs_from_args_keeps_the_one_chip_form() -> None:
    ns = types.SimpleNamespace(chip=[], dsp_image="core1.bin", probe=["0x1256AD"])
    (s,) = live.specs_from_args(ns)
    assert (s.name, s.preset, s.probes, s.window) == ("PNO1", "pno", ["0x1256AD"], "")
    ns = types.SimpleNamespace(chip=["ORG:2=org=o.bin"], dsp_image=None, probe=[])
    assert live.specs_from_args(ns)[0].name == "ORG2"


def test_phase_delta_subtracts_the_additive_numbers_and_the_probe_hits() -> None:
    a = {"messages_sent": 5, "distinct_words": 4, "passes": 10, "idle_passes": 10,
         "probe_hits": {"0x1256ad": 1}}  # fmt: skip
    b = {"messages_sent": 9, "distinct_words": 6, "passes": 30, "idle_passes": 29,
         "probe_hits": {"0x1256ad": 4, "0x121e2b": 20}}  # fmt: skip
    d = live.phase_delta(a, b)
    assert d == {"messages_sent": 4, "distinct_words": 2, "passes": 20, "idle_passes": 19,
                 "probe_hits": {"0x1256ad": 3, "0x121e2b": 20}}  # fmt: skip


def test_snapshot_reports_link_stats() -> None:
    link = dl.Link(Proc())
    link.stats.add(dl.PassReport(idle=2, cnt=4, sumsq=1.0, peak=0.5, hits={0x1256AD: 1}), 0)
    link.send(3, 4)
    s = live.snapshot(link, 0.0)
    assert s["messages_sent"] == 1 and s["idle_passes"] == 2 and s["lane_peak"] == 0.5
    assert s["probe_hits"] == {"0x1256ad": 1} and s["probe_first_pass"] == {"0x1256ad": 0}
    assert s["lane_rms"] == 0.5
    assert isinstance(types.SimpleNamespace(), object)


def test_feeder_skips_words_outside_the_core_window() -> None:
    emu, link = FakeEmu(), dl.Link(Proc())
    router = rt.Router()
    router.add(rt.parse_chip_spec("PNO:1=pno=i.bin"), link)
    f = live.Feeder(emu, router)
    pno = ae.PORT2_BASE + 0x10
    # byte address below 0x28000000: dm_word would be negative (core 1 by the 0x28800000 split)
    emu.port_writes += [(pno, 0x27000000, 1), (pno, 0, 2), (pno, 5, 3)]
    f.feed()
    assert link.log == [] and router.skipped == {"PNO1p2": 1}


def test_return_observer_mirrors_messages_and_registers_every_live_chip() -> None:
    import arm_dsp_return as rtn

    link_p, link_o = dl.Link(Proc()), dl.Link(Proc())
    items = [
        (rt.parse_chip_spec("PNO:2=pno=i.bin"), link_p),
        (rt.parse_chip_spec("ORG:2=org=i.bin"), link_o),
    ]
    source = live.return_source(items, None)
    assert set(source.live) == {(1, 2), (2, 2)}
    assert live.return_observer(None) is None
    seen = live.return_observer(source)
    seen(ae.Message(1, 3, 0x92414, rtn.STATE_PREPARE, 0))
    assert source.sy.idle & 1 == 0


def test_return_report_counts_reads_and_busy_voices() -> None:
    import arm_dsp_return as rtn

    source, log = rtn.ReturnSource(), rtn.ReadLog()
    source.read(3, rtn.IDLE_MASK, 2)
    rep = live.return_report(source, log)
    assert rep["reads"] == {"SY:0:0x92196": 1, "SY:0:0x92197": 1}
    assert rep["answered_by"] == {"sy_mask": 2} and rep["busy_voices"] == 0


def test_poll_advance_feeds_the_messages_then_runs_one_sy_pass() -> None:
    class Link:
        def __init__(self) -> None:
            self.calls: list[str] = []

        def dump(self, addr: int, count: int) -> list[int]:
            return [addr] * count

        def run(self, n: int) -> None:
            self.calls.append(f"run{n}")

    link, order = Link(), []
    before = [lambda: order.append("feed")]
    items = [(rt.parse_chip_spec("SY:0=sy=i.bin"), link)]
    source = live.return_source(items, None, 2, before)
    live_dump = source.live[(3, 0)]
    link.run = lambda n: order.append(f"run{n}")  # type: ignore[method-assign]
    assert [live_dump(0x9219A, 1)[0] for _ in range(4)] == [0x9219A] * 4
    assert order == ["feed", "run1", "feed", "run1"]  # every 2nd read of the address
    off = live.return_source(items, None, 0, before).live[(3, 0)]
    assert off == link.dump  # poll 0: the plain dump


def test_seen_delta_reports_only_the_names_that_grew() -> None:
    before = {"PNO1p2": 5, "SMP0p1": 7, "FX1p2": 2}
    after = {"PNO1p2": 9, "SMP0p1": 7, "FX1p2": 2, "SY0p1": 3}
    assert live.seen_delta(before, after) == {"PNO1p2": 4, "SY0p1": 3}
    assert live.seen_delta(after, after) == {}


def test_driver_re_exports_the_split_out_parts() -> None:
    for name in ("Chips", "Feeder", "phase_delta", "progress", "seen_delta", "snapshot"):
        assert getattr(live, name) is getattr(parts, name)
