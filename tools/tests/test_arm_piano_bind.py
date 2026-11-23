"""Unit tests for tools/arm_piano_bind.py (hash, member lookup, piano layer decode; no firmware)."""

from __future__ import annotations

import faulthandler
import gc

import arm_piano_bind as pb
import pytest

faulthandler.disable()


@pytest.fixture(autouse=True)
def _release_emulators():
    yield
    gc.collect()


ASTORIA = "Astoria Grand Lrg 6.1"
# VERIFIED: the value `crates/nord-core/src/cbin/presets.rs` pins for the same stem.
ASTORIA_HASH = 0x444EBB81


def program_with_hash(name_hash: int, variation: int = 2) -> bytes:
    """Synthetic `.ns4p` (48 header bytes + 820 payload) whose piano layer A stores `name_hash`."""
    data = bytearray(48 + 820)
    payload = bytearray(147)
    base = 104
    bits = {base + 26: (variation, 2), base + 28: (name_hash, 32), 10: (1, 1)}
    for pos, (value, width) in bits.items():
        for i in range(width):
            bit = (value >> (width - 1 - i)) & 1
            at = pos + i
            payload[at // 8] |= bit << (7 - at % 8)
    lo, hi = pb.PIANO_BLOCK
    data[48 + lo : 48 + hi] = payload
    return bytes(data)


def test_name_hash_matches_the_rust_vector() -> None:
    assert pb.nord_name_hash(ASTORIA) == ASTORIA_HASH
    assert pb.nord_name_hash("Ac Guitar Finger_ST Mart 4.1") == 0xA281B28C


def test_hash_matches_accepts_upper_case_stem_and_rejects_zero() -> None:
    assert pb.hash_matches(ASTORIA, ASTORIA_HASH)
    assert pb.hash_matches(ASTORIA, pb.nord_name_hash(ASTORIA.upper()))
    assert not pb.hash_matches(ASTORIA, 0)


def test_resolve_member_finds_the_npno_by_stem() -> None:
    names = ["Program/Bank A/X.ns4p", f"Piano/Grand/{ASTORIA}.npno", "Piano/Grand/Other 1.npno"]
    members = pb.piano_members(names)
    assert members == names[1:]
    assert pb.resolve_member(members, ASTORIA_HASH) == names[1]
    assert pb.resolve_member(members, 12345) is None


def test_program_piano_hash_reads_layer_a_and_variation_is_independent() -> None:
    program = program_with_hash(ASTORIA_HASH, variation=2)
    assert pb.program_piano_hash(program, "layer_a") == ASTORIA_HASH
    layers = pb.program_piano_layers(program)
    assert layers["layer_a"] == {"enabled": True, "name_hash": ASTORIA_HASH}
    assert layers["layer_b"]["enabled"] is False


def test_active_layer_is_the_enabled_one_else_layer_a() -> None:
    off = {"enabled": False, "name_hash": 1}
    on = {"enabled": True, "name_hash": 2}
    assert pb.active_layer({"layer_a": off, "layer_b": on}) == "layer_b"
    assert pb.active_layer({"layer_a": on, "layer_b": on}) == "layer_a"
    assert pb.active_layer({"layer_a": off, "layer_b": off}) == "layer_a"


def test_core0_is_the_candidate_after_manager_zero_or_nothing() -> None:
    mgr = 0x90BF53D0
    assert pb.core0_of(mgr, [0x1000, mgr + 0x30, mgr + 0x40]) == [mgr + 0x30]
    assert pb.core0_of(mgr, [0x1000, mgr + 0x40]) == []


def test_find_cores_and_pool_read_the_pool_pointer_words() -> None:
    import arm_emu as ae

    pytest.importorskip("unicorn")
    big = 0x600000
    emu = ae.ArmEmu(bytes(big), ram_size=big)
    manager, core, pool = ae.SRAM_BASE + 0x1000, ae.SRAM_BASE + 0x2000, 0xA5A5A5A4
    emu.write32(manager + 4, pool)
    emu.write32(core + pb.CORE_POOL_OFFSET, pool)
    assert pb.pool_of(emu, manager) == pool
    found = pb.find_cores(emu, pool, lo=ae.SRAM_BASE, hi=ae.SRAM_BASE + 0x10000, step=0x10000)
    assert core in found


def test_core_of_follows_the_layer_core_stride() -> None:
    """D230: layer A is core 0, layer B core 2 (four cores `CORE_STRIDE` apart)."""
    mgr = 0x90BF5420
    cores = [mgr + 0x30 + k * pb.CORE_STRIDE for k in range(4)] + [0x1000]
    assert pb.core_of(mgr, cores, "layer_a") == [cores[0]]
    assert pb.core_of(mgr, cores, "layer_b") == [cores[2]]
    assert pb.core_of(mgr, cores[:2], "layer_b") == []
    assert pb.CORE_STRIDE == 95_936
