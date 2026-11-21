#!/usr/bin/env python3
"""Piano timbre (`Edt_Pno_kEq`) per-note gain law of the host firmware, from `bank0_CODE.bin`.

The `kEq` handler (`0xC011EC2C`) maps the edit value, by sound family, to a timbre mode and stores
it in the
sound settings object (`[[core + 0x390] + 0x5C]`). The voice-frame builder `0xC00AA520` turns modes
`0x10..=0x1A` into a per-note gain: with `r2 = mode - 0x10`, `key` the played key and `sl = r8 >> 3`
(`r8` = the internal velocity of the note, `docs/research/piano-touch-curves.md`):

    idx = key + 128 r2
    n   = ((S[sl + 128 r2] * T1[idx] + T2[idx]) * 10) >> 16        (floor, n < 0 reads 0)
    amp = G[n] >> 2                      for n <= 60
    amp = G[n - 60 k] >> (k + 2)         for n > 60 (k subtractions of 60 until n <= 60)

`G[i] = round(2^23 * 2^(-i / 60))` (`0xC02A1A08`) and `amp` is the Q23 gain of the voice,
`0x200000` (0.25
as a float word) when no timbre is set. This tool reads the four ROM tables, prints them as Rust,
and
checks the model against the ARM oracle cases:

    python tools/piano_timbre_gain.py check  target/fw_unpacked/bank0_CODE.bin
    python tools/piano_timbre_gain.py rust   target/fw_unpacked/bank0_CODE.bin > \
        crates/nord-core/src/piano/timbre_gain_tables.rs

Cases come from `tools/arm_piano_timbre.py` (`docs/research/data/piano-timbre-gain-cases.json`).
Only
derived numbers are committed; the firmware image stays local. Evidence:
`docs/research/piano-timbre-gain.md`.
"""

from __future__ import annotations

import json
import struct
import sys
from pathlib import Path

LOAD_BASE = 0xC0000000
# VERIFIED (disassembly `0xC00AA754..0xC00AA7CC`, ARM run `arm_piano_timbre.py`): per-mode tables,
# 128 words
# per mode: `S` is read at `sl + 128 r2`, `T1` and `T2` at `key + 128 r2`. 11 modes (`r2` = 0..=10).
S_TABLE = 0xC02A1D94
T1_TABLE = 0xC02A3394
T2_TABLE = 0xC02A4994
# VERIFIED (`piano-dsp-path.md` s7, `level_q23`): `G[i] = round(2^23 2^(-i/60))`, 61 words.
G_TABLE = 0xC02A1A08
MODE_COUNT = 11
FIRST_MODE = 0x10
UNITY_Q23 = 0x200000
# VERIFIED (`docs/research/piano-touch-curves.md`, `arm_touch_curves.py`): `r8 = s16[0xC02B3128 +
# 0x18 + 2 (v + 128 * 6)]`,
# curve 6 of the 8 internal velocity curves is in force at the default program.
R8_TABLE = 0xC02B3128
R8_OFFSET = 0x18
R8_CURVE = 6
CASES = Path(__file__).resolve().parent.parent / "docs/research/data/piano-timbre-gain-cases.json"


def read_words(code: bytes, address: int, count: int) -> list[int]:
    """`count` signed little-endian words at ROM address `address`."""
    return list(struct.unpack_from(f"<{count}i", code, address - LOAD_BASE))


def load_tables(code: bytes) -> dict[str, list[int]]:
    """The four ROM tables as flat lists (`S`, `T1`, `T2`: `MODE_COUNT * 128` words, `G`: 61)."""
    size = MODE_COUNT * 128
    return {
        "S": read_words(code, S_TABLE, size),
        "T1": read_words(code, T1_TABLE, size),
        "T2": read_words(code, T2_TABLE, size),
        "G": read_words(code, G_TABLE, 61),
    }


def r8_table(code: bytes) -> list[int]:
    """`r8` for the velocities 1..=127 (index `v - 1`)."""
    base = R8_TABLE + R8_OFFSET + 2 * 128 * R8_CURVE - LOAD_BASE
    return [struct.unpack_from("<h", code, base + 2 * v)[0] for v in range(1, 128)]


def sl_of_r8(r8: int) -> int:
    """Velocity argument of the frame builder: `r8 >> 3` (VERIFIED, ARM run, 8 of 8 velocities)."""
    return r8 >> 3


def attenuation_index(tab: dict[str, list[int]], mode: int, key: int, sl: int) -> int:
    """`n` of the model above (steps of `6.02 / 60` dB), clamped at 0 like the firmware `bic`."""
    r2 = mode - FIRST_MODE
    if not 0 <= r2 < MODE_COUNT:
        raise ValueError(f"mode {mode:#x} has no gain tables")
    s = tab["S"][sl + 128 * r2]
    idx = key + 128 * r2
    return max(0, ((s * tab["T1"][idx] + tab["T2"][idx]) * 10) >> 16)


def amp_q23(tab: dict[str, list[int]], mode: int, key: int, sl: int) -> int:
    """Q23 voice gain of `mode` at `key` and `sl`; `UNITY_Q23` outside modes `0x10..=0x1A`."""
    if not FIRST_MODE <= mode < FIRST_MODE + MODE_COUNT:
        return UNITY_Q23
    n = attenuation_index(tab, mode, key, sl)
    if n <= 60:
        return tab["G"][n] >> 2
    k = 0
    while n > 60:
        n -= 60
        k += 1
    return tab["G"][n] >> (k + 2)


def float_word(amp: int) -> int:
    """The float word the ARM writes to the SMP voice record: `amp / 2^23` as `f32` bits."""
    return struct.unpack("<I", struct.pack("<f", amp / 8388608.0))[0]


def check(code: bytes, cases: list[dict]) -> list[str]:
    """Differences between the model and the recorded ARM words (empty when all agree)."""
    tab = load_tables(code)
    bad = []
    for c in cases:
        got = float_word(amp_q23(tab, c["mode"], c["key"], sl_of_r8(c["r8"])))
        if got != c["word"]:
            bad.append(
                f"mode {c['mode']:#x} key {c['key']} r8 {c['r8']}: "
                f"model {got:#010x} ARM {c['word']:#010x}"
            )
    return bad


RUST_TESTS = """#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_have_the_firmware_shape() {
        assert_eq!(S.len(), 1408);
        assert_eq!(T1.len(), 1408);
        assert_eq!(T2.len(), 1408);
        assert_eq!(R8.len(), 127);
        assert!(R8.windows(2).all(|w| w[0] < w[1]), "velocity curve rises");
        assert_eq!((R8[0], R8[63], R8[126]), (17, 203, 656));
    }
}
"""


def rust_tables(code: bytes) -> str:
    """Rust source of the tables (generated; see the module docstring)."""
    tab = load_tables(code)
    out = [
        "//! Generated by `python tools/piano_timbre_gain.py rust` from the host firmware ROM.",
        "//! Do not edit.",
        "",
    ]
    for name, doc, base in (
        ("S", "`S[sl + 128 r2]`, velocity term", S_TABLE),
        ("T1", "`T1[key + 128 r2]`, key slope", T1_TABLE),
        ("T2", "`T2[key + 128 r2]`, key offset", T2_TABLE),
    ):
        out.append(f"/// VERIFIED: {doc}, `0x{base:08X}`, 11 modes x 128.")
        out.append(f"pub const {name}: [i32; {MODE_COUNT * 128}] = [")
        vals = tab[name]
        for i in range(0, len(vals), 12):
            out.append("    " + ", ".join(str(v) for v in vals[i : i + 12]) + ",")
        out.append("];")
        out.append("")
    out.append(
        "/// VERIFIED: internal velocity `r8` for the velocities 1..=127 (curve 6), "
        "`0xC02B3128 + 0x18 + 2 (v + 768)`."
    )
    out.append("pub const R8: [u16; 127] = [")
    vals = r8_table(code)
    for i in range(0, len(vals), 16):
        out.append("    " + ", ".join(str(v) for v in vals[i : i + 16]) + ",")
    out.append("];")
    out.append("")
    out.append(RUST_TESTS)
    return "\n".join(out)


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - thin CLI
    args = sys.argv[1:] if argv is None else argv
    if len(args) != 2 or args[0] not in ("check", "rust"):
        print(__doc__)
        return 2
    code = Path(args[1]).read_bytes()
    if args[0] == "rust":
        print(rust_tables(code))
        return 0
    cases = json.loads(CASES.read_text(encoding="utf-8"))["cases"]
    bad = check(code, cases)
    print(f"{len(cases)} cases, {len(bad)} differences")
    for line in bad:
        print(line)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
