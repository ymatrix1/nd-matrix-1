"""Tests of `arm_dsp_route` (D144): chip specs, routing, watched words, replay logs."""

import struct
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import arm_dsp_route as r  # noqa: E402


class FakeLink:
    def __init__(self) -> None:
        self.sent: list[tuple[int, int]] = []
        self.poked: list[tuple[int, int]] = []

    def send(self, w: int, v: int) -> None:
        self.sent.append((w, v))

    def poke(self, w: int, v: int) -> None:
        self.poked.append((w, v))


def test_chip_spec_parses_all_fields_and_builds_flags() -> None:
    s = r.parse_chip_spec("PNO:2=org=a/b.bin=0xC1F83:4:2=0x1256AD,0x121E2B=0xC1174,0xC1180")
    assert (s.name, s.comm, s.preset, s.window) == ("PNO2", 1, "org", "0xC1F83:4:2")
    assert s.probes == ["0x1256AD", "0x121E2B"] and s.watch == [0xC1174, 0xC1180]
    assert s.flags() == [
        "--org", "--epdma", "--window", "0xC1F83:4:2",
        "--probe", "0x1256AD", "--probe", "0x121E2B",
    ]  # fmt: skip
    short = r.parse_chip_spec("ORG:2=org=i.bin")
    assert short.window == "" and short.probes == [] and short.flags() == ["--org", "--epdma"]


@pytest.mark.parametrize("bad", ["x", "FOO:1=org=i", "PNO:3=org=i", "PNO:1=sy=i", "PNO=org=i"])
def test_bad_chip_specs_are_refused(bad: str) -> None:
    with pytest.raises(ValueError):
        r.parse_chip_spec(bad)


def test_router_sends_counts_and_skips() -> None:
    org, pno2 = FakeLink(), FakeLink()
    rt = r.Router()
    rt.add(r.parse_chip_spec("ORG:2=org=i"), org)
    rt.add(r.parse_chip_spec("PNO:2=org=i"), pno2)
    assert rt.dispatch(2, 2, 2, 0xC1DCC, 7) == "sent"
    assert rt.dispatch(2, 1, 2, 0xC1DCC, 9) == "sent"
    assert rt.dispatch(2, 1, 1, 5, 1) == "counted"  # PNO core 1: no chip wired
    assert rt.dispatch(1, 3, 0, 1, 1) == "counted"  # SY path 1
    assert rt.dispatch(2, 2, 2, 0x200000, 1) == "skipped"  # outside the 1 M word window
    assert org.sent == [(0xC1DCC, 7)] and pno2.sent == [(0xC1DCC, 9)]
    assert rt.seen == {"ORG2p2": 2, "PNO2p2": 1, "PNO1p2": 1, "SY0p1": 1}
    assert rt.skipped == {"ORG2p2": 1}


def test_l2_stores_reach_the_core_2_chip_of_the_same_comm_as_data_words() -> None:
    # `arm_emu.Message` files a store to the L2 byte window under core 1 with a negative data word
    pno1, pno2 = FakeLink(), FakeLink()
    rt = r.Router()
    rt.add(r.parse_chip_spec("PNO:1=pno=i"), pno1)
    rt.add(r.parse_chip_spec("PNO:2=org=i"), pno2)
    dm = (0x200C0000 - 0x28000000) // 4  # first byte of the wavetable segment
    assert rt.dispatch(2, 1, 1, dm, 0xAB) == "skipped"  # core 1 keeps its old behaviour
    assert rt.dispatch(2, 1, 1, dm + 1, 0xCD) == "skipped"
    assert pno2.poked == [(0x08030000, 0xAB), (0x08030001, 0xCD)] and pno2.sent == []
    assert pno1.sent == [] and rt.l2 == {"PNO2": 2} and rt.skipped == {"PNO1p2": 2}
    # no chip of that comm on core 2 (ORG here), an address below L2 or a core 2 store: no forward
    assert rt.dispatch(2, 2, 1, dm, 1) == "counted" and rt.dispatch(2, 1, 2, 5, 1) == "sent"
    assert rt.forward_l2(1, 2, 0x1FFF_FFFC, 1) is False and rt.l2 == {"PNO2": 2}
    assert pno2.sent == [(5, 1)] and len(pno2.poked) == 2


def test_watch_stats_tracks_first_change_range_and_final() -> None:
    w = r.WatchStats()
    one = struct.unpack("<I", struct.pack("<f", 1.0))[0]
    half = struct.unpack("<I", struct.pack("<f", 0.5))[0]
    w.add(0, {0x10: 0})
    w.add(5, {0x10: 0})
    w.add(9, {0x10: one})
    w.add(12, {0x10: half})
    s = w.summary()["0x10"]
    assert s["first_change_pass"] == 9 and s["final"] == f"{half:#010x}"
    assert (s["float_min"], s["float_max"]) == (0.0, 1.0)


def test_merged_script_sorts_by_pass_and_keeps_log_order() -> None:
    text = r.merged_script([(5, 0x10, 1), (0, 0x11, 2)], [(0, 0x12, 3)])
    assert text.splitlines() == ["0 0x11 0x00000002", "0 0x12 0x00000003", "5 0x10 0x00000001"]


def test_tap_levels_take_every_other_word_of_the_window() -> None:
    rec = [0.5, 9.0, -0.25, 9.0, 123.0]  # width 5: 4-word window then one more word
    raw = struct.pack("<10f", *rec, *rec)
    peak, rms, n = r.tap_levels(raw, 5, 4)
    assert (peak, n) == (0.5, 4)
    assert rms == pytest.approx(((0.25 + 0.0625) / 2) ** 0.5)


def test_sy_chip_spec_takes_path_1_and_the_sy_flag() -> None:
    s = r.parse_chip_spec("SY:0=sy=target/oracle/sy.bin")
    assert (s.name, s.comm, s.core, s.path) == ("SY0", 3, 0, 1)
    assert s.flags() == ["--sy"]  # no epdma, no level window
    assert r.parse_chip_spec("SY:0=sy=i==").path == 1 and r.parse_chip_spec("ORG:2=org=i").path == 2
    assert r.parse_chip_spec("SY:0=sy=i=epchain").flags() == ["--sy", "--epchain"]
    # D206: opt-in ARM pacing and the FIFO index bias (R3 = 0x80000 in the SY ISR too)
    paced = ["--fifo-pace", "1", "--index-bias", "0x80000"]
    assert r.parse_chip_spec("SY:0=sy=i=epchain+paced").flags() == ["--sy", "--epchain", *paced]
    assert r.parse_chip_spec("SY:0=sy=i=paced").flags() == ["--sy", *paced]


def test_smp_chip_spec_takes_path_1_and_the_smp_flag():
    s = r.parse_chip_spec("SMP:0=smp=target/DSP/dsp_image_4_SMP.bin")
    assert (s.name, s.comm, s.core, s.path, s.flags()) == (
        "SMP0",
        4,
        0,
        1,
        ["--smp", "--fifo-pace", "1"],
    )


@pytest.mark.parametrize(
    "bad", ["SMP:0=sy=i", "SY:0=smp=i", "SMP:1=smp=i", "PNO:1=smp=i", "ORG:0=smp=i"]
)
def test_smp_chip_spec_rejects_mismatched_chip_and_preset(bad):
    with pytest.raises(ValueError):
        r.parse_chip_spec(bad)


@pytest.mark.parametrize("bad", ["SY:0=org=i", "SY:1=sy=i", "ORG:0=org=i", "PNO:0=sy=i"])
def test_sy_preset_and_core_zero_only_go_together(bad: str) -> None:
    with pytest.raises(ValueError):
        r.parse_chip_spec(bad)


def test_router_sends_sy_path_1_messages_to_the_sy_chip_only() -> None:
    sy, org = FakeLink(), FakeLink()
    rt = r.Router()
    rt.add(r.parse_chip_spec("SY:0=sy=i"), sy)
    rt.add(r.parse_chip_spec("ORG:2=org=i"), org)
    assert rt.dispatch(1, 3, 0, 0x92414, 0x1C07CD) == "sent"
    assert rt.dispatch(2, 3, 2, 0x92414, 1) == "counted"  # SY path 2: nobody owns it
    assert rt.dispatch(1, 4, 0, 0x92414, 1) == "counted"  # SMP path 1
    assert sy.sent == [(0x92414, 0x1C07CD)] and org.sent == []
