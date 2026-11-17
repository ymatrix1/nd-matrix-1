#!/usr/bin/env python3
"""Per-tick timeline of the live SY chip beside the ARM message stream (D153).

Pure parts (no firmware, no unicorn), tested in `tools/tests/test_arm_sy_timeline.py`; the driver is
`tools/arm_dsp_live.py --sy-timeline`. One sample per ARM tick holds the SY words the model needs
(voice 0 idle mask bit, release mask bit, Stage A state, step word, stream read pointer) and the
level of the frames the passes of that tick produced. `summary` turns the samples and the ARM
messages into the numbers of `docs/research/sy-live-chip.md`: key-down timing, voice busy and free
ticks, whether the voice reached the playing state, how many source frames it consumed (the ring
is 128 frames, `sy-smp-audio.md` s5) and whether the audio stays non-silent past the first ring.
"""

from __future__ import annotations

from dataclasses import dataclass, field

import arm_dsp_return as rtn
import arm_dsp_return_run as rr

# VERIFIED (`sharc-ref` scripts, `sy-smp-audio.md` s3): voice 0 record `0x92400`, state `+0x14`.
REC = 0x92400
STATE, POS, STEP, FILLED, READ = REC + 0x14, REC + 0x2B, REC + 0x2D, REC + 0x10, REC + 0x1C
STATE_WAITING, STATE_RELEASE, STATE_PLAYING = 0x1C0811, 0x1C083C, 0x1C0851
# VERIFIED (`sy-smp-audio.md` s4): `pos` advances by `2 * step` per audio frame and one source frame
# is `2^25` in `pos`, so one audio frame consumes `step / 2^24` source frames (0.625 = 0xA00000).
STEP_ONE = 1 << 24
RING_FRAMES = 128
ISRS_PER_PASS = 2  # `sharc-live --sy` default (D153)


@dataclass
class Sample:
    """What one ARM tick of the live SY chip showed."""

    tick: int
    idle: int  # DM(0x92196) low word
    release: int  # DM(0x9219A) low word
    state: int
    step: int
    read: int
    passes: int  # passes run in this tick
    cnt: int = 0
    peak: float = 0.0
    sumsq: float = 0.0


@dataclass
class SyTimeline:
    """ARM messages (as `arm_dsp_return_run` rows) and the per-tick SY samples."""

    rows: list[tuple] = field(default_factory=list)
    samples: list[Sample] = field(default_factory=list)

    def message(self, tick: int, chip: str, path: int, w1: int, w2: int) -> None:
        self.rows.append((tick, chip, path, w1, w2))

    def sample(self, s: Sample) -> None:
        self.samples.append(s)

    def consumed(self) -> list[float]:
        """Source frames voice 0 had consumed after each sample: audio frames in the playing
        state x `step / 2^24` (ratio of the pitch table entry)."""
        total, out = 0.0, []
        for s in self.samples:
            if s.state == STATE_PLAYING:
                total += s.passes * ISRS_PER_PASS * s.step / STEP_ONE
            out.append(total)
        return out

    def first(self, pred) -> int | None:  # noqa: ANN001
        """Tick of the first sample for which `pred(sample)` holds."""
        return next((s.tick for s in self.samples if pred(s)), None)

    def transitions(self, limit: int = 60) -> list[list[object]]:
        """`[tick, idle bit 0, release bit 0, state, step]` at every change (first `limit`)."""
        out: list[list[object]] = []
        last = None
        for s in self.samples:
            key = (s.idle & 1, s.release & 1, s.state, s.step)
            if key != last and len(out) < limit:
                out.append([s.tick, key[0], key[1], hex(s.state), hex(s.step)])
            last = key
        return out

    def state_messages(self, limit: int = 40) -> list[list[object]]:
        """`[tick, state word address, state value]` of the SY state stores (first `limit`)."""
        out = [
            [t, hex(w1), hex(w2)]
            for t, chip, path, w1, w2 in self.rows
            if chip == "SY" and path == 1 and rtn.voice_of(w1) is not None
        ]
        return out[:limit]

    def summary(self, note_ticks: list[int], off_tick: int) -> dict[str, object]:
        """The numbers of one run (`note_ticks`: note-on ticks, `off_tick`: the release tick)."""
        out = rr.summarize(self.rows, note_ticks, off_tick)
        # first busy sample after an idle one: the sample before the first pass reads a cold chip
        seen_idle = [False]

        def went_busy(s: Sample) -> bool:
            seen_idle[0] = seen_idle[0] or bool(s.idle & 1)
            return seen_idle[0] and not s.idle & 1

        busy = self.first(went_busy)
        after = [s for s in self.samples if s.tick >= off_tick]
        free = next((s.tick for s in after if s.idle & 1), None)
        loud = [s for s in self.samples if s.peak > 0.0]
        cum = self.consumed()
        frames = cum[-1] if cum else 0.0
        later = [s for s, c in zip(self.samples, cum) if c > RING_FRAMES and s.peak > 0.0]
        out.update(
            voice0_busy_tick=busy,
            voice0_playing_tick=self.first(lambda s: s.state == STATE_PLAYING),
            voice0_release_tick=self.first(lambda s: s.state == STATE_RELEASE),
            voice0_free_after_off_ticks=None if free is None else free - off_tick,
            samples=len(self.samples),
            first_audio_tick=loud[0].tick if loud else None,
            audio_ticks=len(loud),
            peak=max((s.peak for s in self.samples), default=0.0),
            source_frames=round(frames, 1),
            past_first_ring=frames > RING_FRAMES,
            audio_ticks_after_first_ring=len(later),
            transitions=self.transitions(),
            state_messages=self.state_messages(),
        )
        return out


@dataclass
class SyProbe:
    """Takes one `Sample` per ARM tick: from the live SY chip (`link`), else from the message
    driven mask of the return source (`source`, the stub of D147: no state, step or audio)."""

    timeline: SyTimeline
    link: object | None = None  # `arm_dsp_link.Link` of the SY chip
    source: object | None = None  # `arm_dsp_return.ReturnSource`

    def take(self, tick: int, passes: int, report) -> Sample:  # noqa: ANN001
        if self.link is not None:
            word = lambda a: self.link.dump(a, 1)[0]  # noqa: E731
            s = Sample(tick, word(rtn.IDLE_MASK), word(rtn.RELEASE_MASK), word(STATE),
                       word(STEP), word(READ), passes)  # fmt: skip
        else:
            idle = self.source.sy.idle & 0xFFFFFFFF if self.source else 0
            s = Sample(tick, idle, 0, 0, 0, 0, passes)
        if report is not None:
            s.cnt, s.peak, s.sumsq = report.cnt, report.peak, report.sumsq
        self.timeline.sample(s)
        return s
