# ruff: noqa: E501
"""Scenario builder for SAMPLE voices on the SY1SY2 oracle (D150).

Turns the ARM to SY messages of one played key (`arm_dsp_capture` rows) into a `sharc-ref` scenario
that starts voice 0 through the real command sequence and feeds the sample either through the
real packed stream (`stream`, the unpacker runs) or through decoded PCM written into the source
ring (`pcm`, the unpacker is bypassed). Derived numbers only: no sample data is written here
except the local scratch file of the decoded ring signal (never committed).

Provenance tags follow `.agents/rules/provenance-in-code.md`.
"""

from __future__ import annotations

import struct
from collections.abc import Iterable, Sequence
from pathlib import Path

# VERIFIED (sy-smp-audio s3, listing 0x1C0794..0x1C084C): Stage A states written to `rec + 0x14`.
STATE_PREPARE = 0x1C07CD
STATE_GO = 0x1C084C
REC_BASE = 0x92400
REC_STRIDE = 0x38
STATE_FIELD = 0x14
# VERIFIED (sy-pitch-entry-writer s2): index 0x36 is the DSP reset value, the table starts here.
ENTRY_BASE = 0x9354A
DEFAULT_INDEX = 0x36
WORD0_ADDRS = (0x9219E, 0x9219F)  # pair block word 0 of voice lane x / y
PITCH_WORD_ADDR = 0x921A6  # pair block pitch word of voice 0 (`arm-dsp-capture` s1)
# VERIFIED (D112): the unpacker scales the decoded samples by 2^-6 against `sample_dump` and its
# tail runs the 1-pole `y = -c1 x + c0 y`, c0 / c1 = `rec + 0x20 / 0x21` sent by the ARM.
RING_SCALE = 2.0**-6
COEF_ADDRS = (0x92420, 0x92421)
# Record fields of the note-on that only the unpacker uses (zone pairs, start offset, coefficients).
UNPACKER_FIELDS = frozenset({*range(0x0A, 0x14), 0x16, 0x20, 0x21})


def voice_of(addr: int) -> int | None:
    """Voice number of a record word address, `None` outside the voice records.

    EMPIRICAL: the per voice extras `0x92432..0x9243B` of voice 0 reach into the next record's
    first four words, so a remainder of 0..3 past the first record belongs to the previous voice.
    """
    off = addr - REC_BASE
    if off < 0 or off >= REC_STRIDE * 56:
        return None
    voice, field = divmod(off, REC_STRIDE)
    return voice - 1 if voice > 0 and field <= 3 else voice


def field_of(addr: int) -> int:
    """Field (word offset) of a record address inside its voice record."""
    return (addr - REC_BASE) - REC_STRIDE * (voice_of(addr) or 0)


def prologue(scenario: Path) -> list[str]:
    """Command lines of `sy_isr_stream.scn` up to its first `irq 13` (init script state, ring pointers)."""
    lines = [s.strip() for s in scenario.read_text(encoding="utf-8").splitlines()]
    lines = [s for s in lines if s and not s.startswith("#")]
    return lines[: lines.index("irq 13") + 1]


def voice0_messages(rows: Iterable[dict[str, object]]) -> tuple[list[tuple[int, int]], int]:
    """Messages that start voice 0 in the oracle and the table index of its pitch entry.

    The prepare message goes first (EMPIRICAL: with any message ahead of it, or a second one after
    it, the oracle voice never reaches the resampler, see `sample-stream-audio.md` s4); resets,
    the unpacker fields and other voices are dropped.
    """
    keep: list[tuple[int, int]] = []
    index = DEFAULT_INDEX
    for r in rows:
        if r["chip"] != "SY" or r["path"] != 1:
            continue
        addr, word = int(r["dm"]), int(r["w2"])  # type: ignore[call-overload]
        if addr == WORD0_ADDRS[0]:
            index = word & 0xFF
            keep.append((addr, word))
            continue
        if addr == PITCH_WORD_ADDR:
            keep.append((addr, word))
            continue
        v = voice_of(addr)
        if v != 0 or field_of(addr) in UNPACKER_FIELDS or field_of(addr) == STATE_FIELD:
            continue  # other voices, unpacker fields, state messages (prepare is added first)
        keep.append((addr, word))
    prepare = (REC_BASE + STATE_FIELD, STATE_PREPARE)
    return [prepare, *keep], index


def coefficients(rows: Iterable[dict[str, object]]) -> tuple[float, float]:
    """`(c0, c1)` the ARM sent for voice 0, `(0, -1)` (identity filter) when absent."""
    vals = {int(r["dm"]): int(r["w2"]) for r in rows if r["chip"] == "SY" and r["path"] == 1}  # type: ignore[call-overload]

    def f(addr: int, default: float) -> float:
        return struct.unpack("<f", struct.pack("<I", vals[addr]))[0] if addr in vals else default

    return f(COEF_ADDRS[0], 0.0), f(COEF_ADDRS[1], -1.0)


def ring_signal(
    frames: Sequence[tuple[float, float]], c0: float, c1: float
) -> list[tuple[float, float]]:
    """Ring content the unpacker would deliver: `2^-6 * x` through `y = -c1 x + c0 y` per channel."""
    out: list[tuple[float, float]] = []
    yl = yr = 0.0
    for lf, r in frames:
        yl = -c1 * RING_SCALE * lf + c0 * yl
        yr = -c1 * RING_SCALE * r + c0 * yr
        out.append((yl, yr))
    return out


def read_pcm(path: Path) -> list[tuple[float, float]]:
    """Interleaved little-endian float32 stereo frames of a `smp_stream` PCM file."""
    data = path.read_bytes()
    vals = struct.unpack(f"<{len(data) // 4}f", data)
    return list(zip(vals[0::2], vals[1::2], strict=True))


def write_pcm(path: Path, frames: Sequence[tuple[float, float]]) -> None:
    """Write stereo frames as interleaved little-endian float32."""
    flat = [v for lr in frames for v in lr]
    path.write_bytes(struct.pack(f"<{len(flat)}f", *flat))


def _msgs(msgs: Iterable[tuple[int, int]]) -> list[str]:
    return [f"msg {a:#x} {w:#x}" for a, w in msgs]


def build_pcm_scenario(
    pro: Sequence[str],
    pcm: Path,
    msgs: Sequence[tuple[int, int]],
    entry_addr: int,
    entry: int,
    frames: int,
) -> str:
    """Scenario: ring fed with decoded PCM, voice 0 prepared, entry poked, `go`, `frames` audio ISRs."""
    body = [
        # `credit=on`: the model adds the frames it writes to the unpacker credit `rec + 0x10`
        # (VERIFIED listing `0x20A5`, D154); it replaces the per loop `poke 0x92410 0x200` of D150
        # (bit identical output over 20,000 frames, `docs/research/sample-gaps.md` s3).
        f"pcm 0 path={pcm.as_posix()} credit=on",
        *_msgs(msgs),
        "irq 17 400000",
        "irq 13",
        f"poke {entry_addr:#x} {entry}",
        f"msg {REC_BASE + STATE_FIELD:#x} {STATE_GO:#x}",
        f"loop {frames}",
        "irq 17 400000",
        "capture L 0x932ca 1",
        "capture R 0x932cb 1",
        "capture step 0x9242d 1",
        "end",
    ]
    return "\n".join(["meta rate 43402", *pro, *body]) + "\n"


def build_stream_scenario(
    pro: Sequence[str],
    stream: Path,
    arm_msgs: Sequence[tuple[int, int]],
    entry_addr: int,
    entry: int,
    frames: int,
) -> str:
    """Scenario with the real packed stream (unpacker runs) and the ARM messages in ARM order."""
    body = [
        f"stream 0 path={stream.as_posix()}",
        *_msgs(arm_msgs),
        "irq 17 400000",
        "irq 13",
        f"poke {entry_addr:#x} {entry}",
        f"msg {REC_BASE + STATE_FIELD:#x} {STATE_GO:#x}",
        f"loop {frames}",
        "poke 0x92410 0x200",
        "irq 17 400000",
        "capture L 0x932ca 1",
        "capture R 0x932cb 1",
        "end",
        "capture ringL 0xC0000 128 stride=2",
    ]
    return "\n".join(["meta rate 43402", *pro, *body]) + "\n"


def read_dump(path: Path) -> dict[str, list[float]]:
    """Series of a `sharc-ref` CSV dump by name (`#` header lines skipped, empty cells dropped)."""
    lines = path.read_text(encoding="utf-8").splitlines()
    start = next(i for i, s in enumerate(lines) if s.startswith("index,"))
    names = lines[start].split(",")[1:]
    series: dict[str, list[float]] = {n: [] for n in names}
    for line in lines[start + 1 :]:
        for name, cell in zip(names, line.split(",")[1:], strict=False):
            if cell != "":
                series[name].append(float(cell))
    return series
