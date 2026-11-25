#!/usr/bin/env python3
"""What the firmware SMP voice does at the end of a held piano stroke (issue #640).

Two commands on an `arm_dsp_live --piano-bind --log DIR` run (`SMP0.script`, `smp_flash.npno`):

    python tools/smp_stream_end.py loop-words DIR            # the stream words the ARM writes
    python tools/smp_stream_end.py replay DIR smp.bin sharc-live.exe --voice 0 --passes 260000

* `loop-words`: for each voice start in the script, the record words `F-9 .. F-4` (stream end,
  loop end, loop length, page words) and the rate words `F+6` / `F+7`, next to the `.npno`
  stroke record fields they come from (`u32` at `0x24` loop start, `0x28` loop length, `0x0E`
  total frames, `i16` at `0x38` loop decay).
* `replay`: replays the note-on messages of the script (the note-off and everything after pass
  `HOLD_PASSES` are dropped: a held note) into the SMP chip of `sharc-live` and prints the voice
  record every `--step` passes: counter `F-30`, active `F+8`, level `F15`, rate `F17`. The post
  stage ring gate is not involved (D235), the voice record is read directly.

Findings: `docs/research/piano-sample-end.md`.
"""

from __future__ import annotations

import argparse
import struct
from collections import defaultdict
from pathlib import Path

VOICE0 = 0x93928  # VERIFIED (run): record base of voice 0, 0x36 words per voice
VOICE_STRIDE = 0x36
# VERIFIED (run, SMP0.script pass 814): the voice-start request blocks are 12 words apart from
# `0x9251A` (voice pointer word); word +5 is the in-loop rate `F+6`, word +6 the rate `F+7` the
# SMP handler copies into the voice record (replay: record `F+6` = 0.99997795, `F+7` = 0.99998820).
REQUEST0 = 0x9251A
REQUEST_STRIDE = 12
HOLD_PASSES = 2000  # messages of later passes (key-up and after) are dropped by `replay`
STROKE_TABLE = 0x756
STROKE_RECORD = 0x76
TWO_POW_24 = 16_777_216.0


def read_script(path: Path) -> dict[int, list[tuple[int, int]]]:
    """`PASS ADDR VALUE` lines of `arm_dsp_live --log` as messages per pass."""
    out: dict[int, list[tuple[int, int]]] = defaultdict(list)
    for line in path.read_text("utf-8").splitlines():
        p, a, v = line.split()
        out[int(p)].append((int(a, 16), int(v, 16)))
    return out


def f32(word: int) -> float:
    """Float value of a 32 bit DM word."""
    return struct.unpack("<f", struct.pack("<I", word & 0xFFFFFFFF))[0]


def rate_count(word: int) -> int:
    """`d` of a rate word `1 - d / 2^24` (`F17`, `F+6`, `F+7`)."""
    return round((1.0 - f32(word)) * TWO_POW_24)


def voice_base(voice: int) -> int:
    """Record base (`F`) of voice `voice`."""
    return VOICE0 + VOICE_STRIDE * voice


def record_words(msgs: dict[int, list[tuple[int, int]]], voice: int) -> dict[int, int]:
    """Last word the script writes to each record offset `-12 .. +9` of `voice`."""
    base = voice_base(voice)
    words: dict[int, int] = {}
    for p in sorted(msgs):
        for addr, value in msgs[p]:
            if -12 <= addr - base <= 9:
                words[addr - base] = value
    return words


def request_rates(msgs: dict[int, list[tuple[int, int]]], voice: int) -> tuple[int, int] | None:
    """`(F+6, F+7)` rate words the script first writes to the request block of `voice`."""
    base = REQUEST0 + REQUEST_STRIDE * voice
    last: dict[int, int] = {}
    for p in sorted(msgs):
        for addr, value in msgs[p]:
            if base + 5 <= addr <= base + 6:
                last.setdefault(addr - base, value)  # the note-on values: first write
    return (last[5], last[6]) if len(last) == 2 else None


def loop_words(words: dict[int, int]) -> dict[str, int]:
    """Stream words of a voice record: end `F-9`, loop end `F-8`, loop length `F-7`, loop start."""
    end, loop_end, length = words[-9], words[-8], words[-7]
    return {"end": end, "loop_end": loop_end, "loop_len": length, "loop_start": loop_end - length}


def npno_loop_fields(record: bytes) -> dict[str, int]:
    """The `.npno` stroke record fields behind the loop words (`NpnoStroke`, `parse_loop`)."""
    start = struct.unpack_from(">I", record, 0x24)[0]
    length = struct.unpack_from(">I", record, 0x28)[0]
    return {
        "loop_start": start,
        "loop_len": length,
        "loop_end": start + length,
        "end": struct.unpack_from(">I", record, 0x0E)[0],
        "loop_decay": struct.unpack_from(">h", record, 0x38)[0],
    }


def stroke_record(npno: bytes, index: int) -> bytes:
    """Descriptor `index` of an `.npno` file."""
    off = STROKE_TABLE + index * STROKE_RECORD
    return npno[off : off + STROKE_RECORD]


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local firmware
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    lw = sub.add_parser("loop-words")
    lw.add_argument("log", type=Path)
    lw.add_argument("--voices", type=int, default=2)
    rp = sub.add_parser("replay")
    rp.add_argument("log", type=Path)
    rp.add_argument("image", type=Path, help="SMP flat image (target/oracle/smp.bin)")
    rp.add_argument("live", type=Path, help="sharc-live executable")
    rp.add_argument("--voice", type=int, default=0)
    rp.add_argument("--passes", type=int, default=260_000)
    rp.add_argument("--step", type=int, default=8_000)
    ns = ap.parse_args(argv)
    msgs = read_script(ns.log / "SMP0.script")
    if ns.cmd == "loop-words":
        for v in range(ns.voices):
            w = record_words(msgs, v)
            if -9 in w:
                rates = request_rates(msgs, v)
                d = tuple(rate_count(x) for x in rates) if rates else None
                print(f"voice {v}: {loop_words(w)}  rate counts (F+6, F+7) = {d}")
        return 0
    import arm_dsp_link as dl
    import arm_smp_flash as sf

    npno = (ns.log / "smp_flash.npno").read_bytes()
    link = dl.Link.spawn(str(ns.live), str(ns.image), "--smp", "--fifo-pace", "1")
    sf.attach(link, npno, ns.log / "replay_flash.npno")
    base, queue = voice_base(ns.voice), []
    for p in range(ns.passes):
        if p < HOLD_PASSES:
            queue += msgs.get(p, [])
        take, queue = queue[:2], queue[2:]
        for a, v in take:
            link.send(a, v)
        link.run(1)
        if p % ns.step == 0:
            w = link.dump(base - 30, 55)
            at = {off: w[off + 30] for off in (-30, 8, 15, 17, 6, 7)}
            print(
                f"pass {p} counter {at[-30]} active {at[8]} F15 {f32(at[15]):.6f} "
                f"F17 d={rate_count(at[17])} (F+6 d={rate_count(at[6])}, F+7 d={rate_count(at[7])})"
            )
    link.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
