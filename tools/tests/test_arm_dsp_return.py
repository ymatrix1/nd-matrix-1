"""Unit tests for tools/arm_dsp_return.py (pure parts and the stubs on a fake emulator)."""

from __future__ import annotations

import struct

import arm_dsp_return as rt
import arm_emu as ae


class FakeEmu:
    """Registers and word memory, what the stubs touch."""

    def __init__(self) -> None:
        self.regs = [0] * 16
        self.mem: dict[int, int] = {}

    def reg(self, n: int) -> int:
        return self.regs[n]

    def read32(self, a: int) -> int:
        return self.mem.get(a, 0)

    def write32(self, a: int, v: int) -> None:
        self.mem[a] = v


def test_constants_match_the_harness() -> None:
    assert rt.COMM_TABLE == ae.COMM_TABLE
    assert rt.CHIPS == ae.COMM_NAMES


def test_resolve_alias_core_and_word() -> None:
    assert rt.resolve(1, 0x28307770) == (1, 0xC1DDC)
    assert rt.resolve(1, 0x28B07770) == (2, 0xC1DDC)
    assert rt.resolve(2, 0x28B07770) == (2, 0xC1DDC)
    assert rt.resolve(3, 0x92196) == (0, 0x92196)  # SY: plain word
    assert rt.resolve(3, 0x28307770) == (0, 0x28307770)  # alias only for PNO and ORG


def test_chip_of_comm_scans_the_table() -> None:
    mem = {rt.COMM_TABLE + 4 + 4 * c: 0x9000_0000 + 0x100 * c for c in range(5)}
    assert rt.chip_of_comm(lambda a: mem.get(a, 0), 0x9000_0300) == 3
    assert rt.chip_of_comm(lambda a: mem.get(a, 0), 0x1234) == -1


def test_voice_of_state_message_address() -> None:
    assert rt.voice_of(0x92414) == 0
    assert rt.voice_of(0x92414 + 0x38 * 55) == 55
    assert rt.voice_of(0x92414 + 0x38 * 56) is None
    assert rt.voice_of(0x92415) is None
    assert rt.voice_of(0x92400) is None


def test_sy_mask_follows_state_messages() -> None:
    m = rt.SyVoiceMask()
    assert m.word(rt.IDLE_MASK) == 0xFFFFFFFF and m.word(rt.IDLE_MASK + 1) == 0x00FFFFFF
    m.observe(0x92414, rt.STATE_PREPARE)
    assert m.word(rt.IDLE_MASK) == 0xFFFFFFFE  # the oracle dump after the prepare state
    m.observe(0x92414 + 0x38 * 32, rt.STATE_GO)
    assert m.word(rt.IDLE_MASK + 1) == 0x00FFFFFE
    m.observe(0x92414, rt.STATE_RESET)
    assert m.word(rt.IDLE_MASK) == 0xFFFFFFFF
    m.observe(0x92414 + 0x38 * 32, 0x1234)  # unknown state: unchanged
    assert m.word(rt.IDLE_MASK + 1) == 0x00FFFFFE
    assert m.word(rt.RELEASE_MASK) == 0
    assert m.word(0x1000) is None


def test_image_word_bounds_and_endianness() -> None:
    img = struct.pack("<II", 0x11223344, 0xAABBCCDD)
    assert rt.image_word(img, 0) == 0x11223344
    assert rt.image_word(img, 1) == 0xAABBCCDD
    assert rt.image_word(img, 2) == 0
    assert rt.image_word(None, 0) == 0


def test_source_layer_order() -> None:
    img = struct.pack("<II", 7, 8)
    src = rt.ReturnSource(images={(1, 2): img}, live={(1, 1): lambda dm, n: [0x55] * n})
    assert src.word(1, 2, 0) == 7  # image
    src.observe(1, 2, 2, 0x28800000, 9)  # path 2 write to core 2 word 0
    assert src.word(1, 2, 0) == 9  # shadow beats image
    assert src.word(1, 1, 3) == 0x55  # live beats everything for its chip and core
    assert src.word(2, 1, 5) == 0  # nothing knows it
    assert src.layer == {"image": 1, "shadow": 1, "live": 1, "zero": 1}


def test_source_path1_shadow_and_sy_mask() -> None:
    src = rt.ReturnSource()
    src.observe(0, 0, 1, 0x5000, 0xAB)
    assert src.word(0, 0, 0x5000) == 0xAB
    src.observe(3, 0, 1, 0x92414, rt.STATE_PREPARE)
    assert src.read(3, rt.IDLE_MASK, 2) == [0xFFFFFFFE, 0x00FFFFFF]


def test_read_stub_four_words_through_the_peek_routine() -> None:
    src = rt.ReturnSource(images={(2, 2): struct.pack("<4I", 1, 2, 3, 4)})
    emu, log = FakeEmu(), rt.ReadLog()
    emu.regs[1], emu.regs[2], emu.regs[3] = 2, 0x28800000, 0x9000_0000
    assert rt.make_stub(src, log, rt.SPI_READ4)(emu) == 0
    assert [emu.mem[0x9000_0000 + 4 * k] for k in range(4)] == [1, 2, 3, 4]
    assert log.calls == [(rt.SPI_READ4, 2, 0x28800000, 4, 0)]


def test_read_stub_two_words_and_the_comm_variant() -> None:
    src = rt.ReturnSource()
    emu, log = FakeEmu(), rt.ReadLog()
    emu.regs[1], emu.regs[2], emu.regs[3] = 3, rt.IDLE_MASK, 0x9100_0000
    rt.make_stub(src, log, rt.SPI_READ2)(emu)
    assert emu.mem[0x9100_0000] == 0xFFFFFFFF and emu.mem[0x9100_0004] == 0x00FFFFFF
    # comm variant: r0 comm object of chip 3, r2 address, r3 count, destination on the stack
    emu.mem[rt.COMM_TABLE + 4 + 4 * 3] = 0x9002_0000
    emu.regs[0], emu.regs[2], emu.regs[3], emu.regs[13] = 0x9002_0000, rt.IDLE_MASK, 2, 0x9300_0000
    emu.mem[0x9300_0000] = 0x9200_0000
    rt.make_stub(src, log, rt.SPI_READN)(emu)
    assert emu.mem[0x9200_0000] & 1 == 1  # voice 0 idle bit: the ARM's wait succeeds
    assert log.polls[(3, rt.IDLE_MASK)] == 2


def test_readn_falls_back_for_unknown_chip_address_or_count() -> None:
    src, log, emu = rt.ReturnSource(), rt.ReadLog(), FakeEmu()
    stub = rt.make_stub(src, log, rt.SPI_READN)
    emu.mem[rt.COMM_TABLE + 4 + 4 * 3] = 0x9002_0000
    emu.mem[0x9300_0000] = 0x9200_0000
    emu.mem[0x9200_0000] = 0x1234
    emu.regs[13] = 0x9300_0000
    # unknown comm object, unknown address, count over the cap: return 0, destination untouched
    for comm, address, count in (
        (0x1, rt.IDLE_MASK, 2),
        (0x9002_0000, 0x5000, 2),
        (0x9002_0000, rt.IDLE_MASK, rt.READN_MAX_COUNT + 1),
    ):
        emu.regs[0], emu.regs[2], emu.regs[3] = comm, address, count
        assert stub(emu) == 0
    assert emu.mem[0x9200_0000] == 0x1234
    assert log.fallbacks == 3 and not log.calls


def test_shadow_key_uses_the_core_of_the_address() -> None:
    src = rt.ReturnSource()
    # a message tagged core 1 that addresses core 2 (bit 23) lands where `read` looks
    src.observe(1, 1, 2, 0x28B07770, 0x77)
    assert src.read(1, 0x28B07770, 1) == [0x77]
    assert src.read(1, 0x28307770, 1) == [0]


def test_busy_voice_is_seen_by_the_status_read() -> None:
    src = rt.ReturnSource()
    src.observe(3, 0, 1, 0x92414, rt.STATE_PREPARE)
    emu = FakeEmu()
    emu.regs[1], emu.regs[2], emu.regs[3] = 3, rt.IDLE_MASK, 0x9100_0000
    rt.make_stub(src, rt.ReadLog(), rt.SPI_READ2)(emu)
    assert emu.mem[0x9100_0000] & 1 == 0  # the wait on bit 0 would not succeed


def test_load_images_skips_missing(tmp_path) -> None:  # noqa: ANN001
    d = tmp_path / "PNO"
    d.mkdir()
    (d / "core2_sram.bin").write_bytes(b"\x01\x00\x00\x00")
    got = rt.load_images(tmp_path, {1: "PNO", 2: "ORG"})
    assert list(got) == [(1, 2)] and rt.image_word(got[(1, 2)], 0) == 1
    assert rt.load_images(None, {1: "PNO"}) == {}


def test_a_live_sy_chip_answers_before_the_message_driven_mask() -> None:
    src = rt.ReturnSource()
    src.observe(3, 0, 1, rt.STATE_BASE, rt.STATE_PREPARE)  # the model says voice 0 is busy
    assert src.read(3, rt.IDLE_MASK, 1) == [0xFFFFFFFE]
    src.live[(3, 0)] = lambda dm, n: [0xFFFFFFFF] * n  # the chip says it is idle again
    assert src.read(3, rt.IDLE_MASK, 2) == [0xFFFFFFFF, 0xFFFFFFFF]
    assert src.layer["live"] == 2 and src.knows(3, rt.IDLE_MASK)


def test_the_voice_update_wait_records_its_test_and_whether_it_passed() -> None:
    src, log, emu = rt.ReturnSource(), rt.ReadLog(), FakeEmu()
    src.live[(3, 0)] = lambda dm, n: [0x4, 0][:n] if dm == rt.RELEASE_MASK else [0] * n
    emu.mem[rt.COMM_TABLE + 4 + 4 * 3] = 0x9002_0000
    emu.regs[0], emu.regs[2], emu.regs[3], emu.regs[13] = (
        0x9002_0000,
        rt.RELEASE_MASK,
        2,
        0x9300_0000,
    )
    emu.regs[5], emu.regs[14] = 0x9400_0000, rt.WAIT_RETURN
    emu.mem[0x9300_0000] = 0x9200_0000  # destination = cache base
    emu.mem[0x9400_0004], emu.mem[0x9400_0008] = 0, 0x4  # descriptor: word index 0, mask bit 2
    stub = rt.make_stub(src, log, rt.SPI_READN)
    stub(emu)
    emu.mem[0x9400_0008] = 0x1  # a different bit of the same word: the test fails
    stub(emu)
    assert log.waits == {(0, 0x4, rt.RELEASE_MASK): [1, 1], (0, 0x1, rt.RELEASE_MASK): [1, 0]}
    emu.regs[4], emu.regs[14] = (
        0x9500_0000,
        0xC01E26FC,
    )  # the second wait keeps its descriptor in r4
    emu.mem[0x9500_0004], emu.mem[0x9500_0008] = 1, 0x2
    stub(emu)
    assert log.waits[(1, 0x2, rt.RELEASE_MASK)] == [1, 0]
    assert log.wait_ticks[rt.RELEASE_MASK] == [0, 0]
    emu.regs[14] = 0x1234  # any other caller: not a wait test, but counted by its return address
    stub(emu)
    assert sum(r[0] for r in log.waits.values()) == 3
    assert log.callers[(rt.SPI_READN, 0x1234, rt.RELEASE_MASK)] == 1
    assert log.callers[(rt.SPI_READN, rt.WAIT_RETURN, rt.RELEASE_MASK)] == 2


def test_poll_advance_runs_one_pass_per_n_reads_of_the_same_address() -> None:
    steps: list[int] = []
    seen = rt.poll_advance(lambda a, n: [a] * n, lambda: steps.append(1), 3)
    assert [seen(0x9219A, 1)[0] for _ in range(7)] == [0x9219A] * 7
    assert len(steps) == 2  # reads 3 and 6
    seen(0x92196, 1)  # another address has its own count
    assert len(steps) == 2


def test_poll_advance_zero_leaves_the_dump_untouched() -> None:
    dump = lambda a, n: [a] * n  # noqa: E731
    assert rt.poll_advance(dump, lambda: None, 0) is dump
