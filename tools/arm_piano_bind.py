#!/usr/bin/env python3
"""Bind a program's piano sound in the booted ARM harness (OQ41 blocker 1, D181).

The recall stores the piano layer's sound as a 32-bit name hash (`ns4_inspect` layer `name_hash`);
the harness has no sound library, so no piano sound object exists and a key never reaches the SMP
voice start. This module maps the hash to the `.npno` member of the factory backup (the same
MurmurHash2 as `nord_core::cbin::presets::nord_name_hash`), then `arm_piano_load` serves that file
as library entry 1, loads it with the real loader and binds it to core 0 layer 0.

    members = piano_members(backup_namelist)
    member = resolve_member(members, program_piano_hash(program_bytes, "layer_a"))
"""

from __future__ import annotations

from collections.abc import Iterable
from pathlib import PurePosixPath

import ns4_inspect as ni

MURMUR_M = 0x5BD1E995
MASK = 0xFFFFFFFF
CBIN_HEADER = ni.CBIN_HEADER_SIZE  # 48: payload start of a `.ns4p`
PIANO_BLOCK = (180, 327)  # `.ns4p` payload bytes of the piano section (`decode_cbin_payload`)


def nord_name_hash(stem: str) -> int:
    """VERIFIED (mirror of `nord_name_hash` in `crates/nord-core/src/cbin/presets.rs`, ARM0
    `0x0a5a44`): MurmurHash2 with big-endian 4-byte reads, seed = length, 0 maps to 1."""
    data = stem.encode("ascii", "replace")
    h = len(data) & MASK
    whole = len(data) // 4 * 4
    for i in range(0, whole, 4):
        k = int.from_bytes(data[i : i + 4], "big")
        k = k * MURMUR_M & MASK
        k ^= k >> 24
        k = k * MURMUR_M & MASK
        h = h * MURMUR_M & MASK
        h ^= k
    rem = data[whole:]
    if len(rem) == 3:
        h ^= rem[2] << 16
    if len(rem) >= 2:
        h ^= rem[1] << 8
    if rem:
        h ^= rem[0]
        h = h * MURMUR_M & MASK
    h ^= h >> 13
    h = h * MURMUR_M & MASK
    h ^= h >> 15
    return h or 1


def hash_matches(stem: str, target: int) -> bool:
    """Raw or upper-cased stem equals `target` (`nord_name_hash_matches`)."""
    return target != 0 and target in (nord_name_hash(stem), nord_name_hash(stem.upper()))


def piano_members(names: Iterable[str]) -> list[str]:
    """The `.npno` members of a backup name list."""
    return [n for n in names if n.lower().endswith(".npno")]


def resolve_member(members: Iterable[str], target: int) -> str | None:
    """First member whose file stem hashes to `target` (None when no match or `target` is 0)."""
    for name in members:
        if hash_matches(PurePosixPath(name).stem, target):
            return name
    return None


LAYER_BASE = {"layer_a": 104, "layer_b": 200}  # `PianoPresetDecoded::unpack` layer block bases
NAME_HASH_BIT = 28  # bit offset of `name_hash` in a layer block (`unpack_piano_layer_block`)
ENABLE_BIT = {"layer_a": 10, "layer_b": 9}  # `enable_i` is 2 bits at 9, A = its low bit (`unpack`)


def _piano_payload(program: bytes) -> bytes:
    lo, hi = PIANO_BLOCK
    return program[CBIN_HEADER:][lo:hi]


def program_piano_hash(program: bytes, layer: str = "layer_a") -> int:
    """VERIFIED (Astoria Grand: bit 1956 of the file = `0x444EBB81`, `ns4 inspect --json`
    agrees): `name_hash` of piano layer `layer` of a `.ns4p` file (0: no sound stored).

    Read directly: `ns4_inspect._decode_piano_layer` seeks to bit 28 before the `variation` field
    and so reads the hash 2 bits late (stale decoder, not used here)."""
    reader = ni.BitReader(_piano_payload(program))
    reader.set_bit_pos(LAYER_BASE[layer] + NAME_HASH_BIT)
    return reader.read_bits(32)


def active_layer(layers: dict[str, dict]) -> str:
    """The piano layer the loader plays: the first with its enable bit, else layer A.

    VERIFIED (`engine/routing.rs::default_layer_a`, `engine-routing.md` s7): an enabled section with
    no layer enabled plays layer A. Five factory programs (Tines Layer, LoFi Piano, Rising Sun,
    Piano Soft Orch, 70s CP Strings) store both piano enable bits 0 and sound on layer A; the
    census bound nothing for them and read `piano_no_chain_tap` (D216).
    """
    return next((k for k, v in layers.items() if v["enabled"]), "layer_a")


def program_piano_layers(program: bytes) -> dict[str, dict]:
    """`{layer: {enabled, name_hash}}` of both piano layers."""
    reader = ni.BitReader(_piano_payload(program))
    out = {}
    for layer, bit in ENABLE_BIT.items():
        reader.set_bit_pos(bit)
        out[layer] = {"enabled": bool(reader.read_bits(1)), "name_hash": 0}
    for layer in out:
        out[layer]["name_hash"] = program_piano_hash(program, layer)
    return out


# VERIFIED (run, `piano-sound-load.md` s2): the core object keeps the sound pool pointer at +0x14
# and the bound layer objects at +4 + 4 * layer.
CORE_POOL_OFFSET = 0x14
# EMPIRICAL (settled state of `arm_piano_load`: managers `0x90BF53D0` / `0x90BF53E0`, core 0
# `0x90BF5400`; the full machine of `arm_dsp_live` puts all three at the same relative offsets).
CORE0_AFTER_MANAGER = 0x30
# EMPIRICAL (run, D230, `piano-smp-pitch-outliers.md` s3): the four piano core objects of the
# pool sit 95,936 bytes apart (`0x176C0`, candidates of `find_cores`). Simple Tune (piano layer B
# only) bound to core 0, 1 or 3 starts no voice (381 key messages, 0 stroke descriptors); bound to
# core 2 it starts three voices (3 of 3 descriptors, `law_ok`). Layer A is core 0 (every other
# census program). Layer B on core 2 is one program; core 1 and 3 are not identified.
CORE_STRIDE = 0x176C0
LAYER_CORE = {"layer_a": 0, "layer_b": 2}


def find_cores(emu, pool: int, **scan) -> list[int]:  # noqa: ANN001
    """Candidate core objects: addresses whose word at +0x14 is the pool pointer `pool`
    (`scan`: `lo`, `hi`, `step` of `arm_piano_load.find_words`)."""
    import arm_piano_load as pl

    return [a - CORE_POOL_OFFSET for a in pl.find_words(emu, pool, **scan)]


def pool_of(emu, manager: int) -> int:  # noqa: ANN001
    """The sound pool pointer of a library manager (`[mgr + 4]`, `piano-sound-load.md` s1)."""
    return emu.read32(manager + 4)


def core0_of(manager: int, candidates: list[int]) -> list[int]:
    """Core 0 among the pool-pointer candidates: the one `CORE0_AFTER_MANAGER` after manager 0
    (empty when it is not a candidate: nothing is bound rather than a guess)."""
    return core_of(manager, candidates, "layer_a")


def core_of(manager: int, candidates: list[int], layer: str) -> list[int]:
    """The core object that plays piano `layer` (`LAYER_CORE`): `CORE0_AFTER_MANAGER` after
    manager 0 plus `LAYER_CORE[layer]` strides (empty when it is not a candidate)."""
    expected = manager + CORE0_AFTER_MANAGER + LAYER_CORE[layer] * CORE_STRIDE
    return [expected] if expected in candidates else []


def bind_piano(  # noqa: ANN001  # pragma: no cover
    emu, npno: bytes, entry: int = 1, layer: str = "layer_a"
) -> dict:
    """Serve `npno` as library entry `entry`, load it with the firmware loader and bind it to
    core 0 layer 0 (`arm_piano_load`), then model the DSP link as always ready.

    ASSUMED: entry/volume id 1 is free (the harness library is empty). The object goes to slot 0 of
    the core that plays `layer` (`LAYER_CORE`, D230: layer B is core 2); only one layer is bound.
    Returns the loader status and the bound pool object (0: not bound)."""
    import arm_piano_load as pl

    pl.LibraryStub({entry: pl.device_image(npno, "Library.npno")}).install(emu)
    managers = pl.managers(emu)
    if not managers:
        return {"status": "no manager", "bound": 0}
    status = pl.load_sound(emu, managers[0], entry)
    pool = pool_of(emu, managers[0])
    cores = find_cores(emu, pool)
    cores = core_of(managers[0], cores, layer)
    info = {"status": status, "managers": managers, "pool": pool, "cores": cores}
    if not cores:
        return {**info, "bound": 0}
    pl.select_sound(emu, entry, core=cores[0])
    pl.install_dsp_ready(emu)
    return {**info, "bound": pl.bound_sound(emu, cores[0])}
