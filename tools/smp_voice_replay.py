#!/usr/bin/env python3
"""Replay the SMP messages of an `arm_dsp_live --piano-bind --log DIR` run, check the voice decode.

The voice of the live SMP chip decodes one CNSP residual per call with a fixed order 1..3
predictor (`docs/research/smp-voice.md` s3). A wrong start history makes the predictor run away
(a polynomial, so the tap turns into a wrapped sawtooth that `arm_smp_flash.tap_stats` still calls
`audio`). This tool replays `DIR/SMP0.script` with the ARM pacing of `sharc-live --fifo-pace`
(D205), reads the history ring of the first voice and compares it with `decode_page` of the bound
file. Reproduce: see `docs/research/smp-voice-pitch.md` s2.

    python tools/smp_voice_replay.py DIR SMP_IMAGE sharc-live.exe --pace-per-pass 2
"""

from __future__ import annotations

import argparse
import struct
from collections import defaultdict
from pathlib import Path

HIST_BASE = 0xC0500  # VERIFIED (run): history ring of voice 0, 16 words, even slots left, odd right
VOICE0 = 0x93928  # VERIFIED (run): `I3` of voice 0 (ring pointer `rec[-26]` = 0x9B000)
# VERIFIED (run, Astoria Grand, SMP0.script: the second start of the note-on writes the request
# `0x92526 = 0x9395E` and the history `0xC0510..`, the release voice `0x93CBE` = voice 17): the
# voice records are `0x36` words apart and the start history of voice `v` is `0xC0500 + 16 v`.
VOICE_STRIDE = 0x36
HIST_STRIDE = 16
COUNTER = -30  # `rec[-30]`: residual number of the next call
LPC = {
    0: (0, 0, 0, 0),
    1: (0, 0, 0, 1),
    2: (0, 0, -1, 2),
    3: (0, 1, -3, 3),
    4: (-1, 4, -6, 4),  # VERIFIED: stroke 523 voice 17 (piano-release-voice.md)
}  # row = 4 * order


def page_residual(words: list[int], n: int, width: int, stride: int, base: int = 0) -> int:
    """Residual `n` of a page: `width` bits at `stride * n - base` of the LSB first word stream."""
    bit = stride * n - base
    value = 0
    for k in range(width):
        b = bit + k
        value |= ((words[b // 16] >> (b % 16)) & 1) << k
    return value - (1 << width) if value >> (width - 1) else value


def decode_page(page: bytes, init: list[int], count: int, channel: int = 0) -> list[int]:
    """First `count` samples of channel `channel` of a CNSP page (2 byte header, big endian words).

    `init` is the history `y(-4) .. y(-1)` of that channel (oldest first), the descriptor words.
    """
    order, width = page[1] >> 5, page[1] & 31
    channels = 2 if len(page) >= 2044 else 1
    words = [struct.unpack_from(">H", page, 2 + 2 * k)[0] for k in range(len(page) // 2 - 1)]
    coef = LPC[order]
    hist, out = list(init), []
    for n in range(count):
        base = width * channel
        r = page_residual(words, n, width, channels * width, -base if channel else 0)
        y = sum(c * h for c, h in zip(coef, hist, strict=True)) + r
        out.append(y)
        hist = [*hist[1:], y]
    return out


def history_samples(ring: list[int], counter: int) -> list[int]:
    """Decoded samples `y(counter-8) .. y(counter-1)` in the stride 2 history ring (left lane).

    `y(n)` sits at slot `2 * ((n + 4) & 7)` (stored at `s + 8`, `s = 2 * (n & 7)`, `hist_off = 0`).
    """

    def s32(x: int) -> int:
        return x - (1 << 32) if x >= 1 << 31 else x

    return [s32(ring[2 * ((n + 4) & 7)]) for n in range(counter - 8, counter)]


def voice_addresses(voice: int) -> tuple[int, int]:
    """`(record I3, history base)` of voice `voice` of the SMP (see `VOICE_STRIDE`)."""
    return VOICE0 + VOICE_STRIDE * voice, HIST_BASE + HIST_STRIDE * voice


def match_counter(ring: list[int], counter: int, decoded: list[int]) -> int | None:
    """Counter value (`counter` or `counter + 1`) at which the ring equals the reference decode.

    EMPIRICAL (run, Astoria Grand voice 1, stroke 502: counter 379, seven of eight words equal and
    the eighth is `y(379)`): a dump taken inside the call can hold sample `counter` already, so the
    ring is one sample ahead of the counter word.
    """
    for c in (counter, counter + 1):
        if c >= 8 and history_samples(ring, c) == decoded[c - 8 : c]:
            return c
    return None


def init_history(
    msgs: dict[int, list[tuple[int, int]]], lane: int = 0, base: int = HIST_BASE
) -> list[int]:
    """Start history `y(-4) .. y(-1)` of `lane` as the ARM wrote it: the first words of the ring.

    The ARM writes `0xC0500 + 2 k + lane` for `k = 0..3` (signed 32 bit, left lane 0, right 1).
    """
    first: dict[int, int] = {}
    for p in sorted(msgs):
        for a, v in msgs[p]:
            first.setdefault(a, v)
    return [
        (lambda v: v - (1 << 32) if v >= 1 << 31 else v)(first[base + 2 * k + lane])
        for k in range(4)
    ]


def read_script(path: Path) -> dict[int, list[tuple[int, int]]]:
    """`PASS ADDR VALUE` lines of `arm_dsp_live --log` as messages per pass."""
    out: dict[int, list[tuple[int, int]]] = defaultdict(list)
    for line in path.read_text("utf-8").splitlines():
        p, a, v = line.split()
        out[int(p)].append((int(a, 16), int(v, 16)))
    return out


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local firmware
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument(
        "log", type=Path, help="DIR of the arm_dsp_live run (SMP0.script, smp_flash.npno)"
    )
    ap.add_argument("image", type=Path, help="SMP flat image (target/oracle/smp.bin)")
    ap.add_argument("live", type=Path, help="sharc-live executable")
    ap.add_argument("--stroke", type=int, required=True)
    ap.add_argument("--voice", type=int, default=0, help="voice index (record 0x36 words apart)")
    ap.add_argument("--passes", type=int, default=900)
    ap.add_argument("--pace-per-pass", type=int, default=2, help="ARM pairs per pass (0 = burst)")
    ns = ap.parse_args(argv)
    import arm_dsp_link as dl
    import arm_smp_flash as sf

    npno = (ns.log / "smp_flash.npno").read_bytes()
    stroke = next(s for s in sf.parse_strokes(npno) if s.index == ns.stroke)
    page = npno[stroke.offset : stroke.offset + 1022 * stroke.channels]
    link = dl.Link.spawn(str(ns.live), str(ns.image), "--smp")
    sf.attach(link, npno, ns.log / "replay_flash.npno")
    msgs, queue = read_script(ns.log / "SMP0.script"), []
    for p in range(ns.passes):
        queue += msgs.get(p, [])
        take, queue = (queue, []) if ns.pace_per_pass == 0 else (queue[:2], queue[2:])
        for a, v in take[: ns.pace_per_pass or None]:
            link.send(a, v)
        link.run(1)
    record, hist = voice_addresses(ns.voice)
    counter = link.dump(record + COUNTER, 1)[0]
    ring = link.dump(hist, 16)
    link.close()
    init = init_history(msgs, base=hist)
    decoded = decode_page(page, init, counter + 1)
    at = match_counter(ring, counter, decoded)
    print(f"counter {counter}")
    print(f"history {history_samples(ring, counter)}")
    print(f"reference {decoded[counter - 8 : counter]}")
    print("MATCH" if at is not None else "DIFFERENT")
    return 0 if at is not None else 1


if __name__ == "__main__":
    raise SystemExit(main())
