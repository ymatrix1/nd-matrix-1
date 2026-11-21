"""Pieces of the live ARM + SHARC driver (`tools/arm_dsp_live.py`): progress, snapshots, the
message feeder, the return-path wiring and the chip set. Split out to keep the driver small
(`.agents/rules/code-structure.md`); `arm_dsp_live` re-exports every name, tests in
`tools/tests/test_arm_dsp_live.py` and `test_arm_dsp_live_parts.py`.
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

import arm_dsp_link as dl
import arm_dsp_return as rtn
import arm_dsp_route as rt


def progress(text: str) -> None:
    """Progress line on stderr (stdout carries the JSON result only)."""
    print(f"[{time.strftime('%H:%M:%S')}] {text}", file=sys.stderr, flush=True)


def snapshot(link: dl.Link, exe_t0: float) -> dict[str, object]:
    """Cumulative numbers of `link.stats` (JSON friendly)."""
    s = link.stats
    return {
        "messages_sent": s.messages,
        "distinct_words": len(s.words),
        "passes": link.pass_index,
        "idle_passes": s.idle,
        "halted": s.halted,
        "lane_peak": s.peak,
        "lane_rms": s.rms,
        "probe_hits": {f"{pc:#x}": n for pc, n in sorted(s.hits.items())},
        "probe_first_pass": {f"{pc:#x}": n for pc, n in sorted(s.first_hit.items())},
        "errors": s.errors[:3],
        "wall_s": round(time.perf_counter() - exe_t0, 1),
    }


def phase_delta(before: dict[str, object], after: dict[str, object]) -> dict[str, object]:
    """Difference of two cumulative snapshots for the additive numbers."""
    keys = ("messages_sent", "distinct_words", "passes", "idle_passes")
    out = {k: after[k] - before[k] for k in keys}  # type: ignore[operator]
    hits_a, hits_b = after["probe_hits"], before["probe_hits"]
    out["probe_hits"] = {k: v - hits_b.get(k, 0) for k, v in hits_a.items()}  # type: ignore[union-attr]
    return out


def seen_delta(before: dict[str, int], after: dict[str, int]) -> dict[str, int]:
    """Messages per `COMMcorepPATH` name that arrived between two `Router.seen` copies."""
    return {k: v - before.get(k, 0) for k, v in sorted(after.items()) if v != before.get(k, 0)}


class Feeder:
    """Moves the new port stores of the emulator through the grouper to the router."""

    def __init__(self, emu, router: rt.Router, observer=None, timeline=None) -> None:  # noqa: ANN001
        import arm_emu as ae

        self.emu, self.router, self.ae = emu, router, ae
        self.timeline, self.tick = timeline, -1  # tick -1: the boot, before the clock runs (D153)
        self.observer = observer  # called with every grouped message (return path shadow, D147)
        self.grouper = dl.PortGrouper(
            dl.port_table(ae.PORT2_BASE, ae.PORT1_BASE, ae.COMM_COUNT, emu.pad)
        )

    def feed(self) -> None:
        writes = list(self.emu.port_writes)
        self.emu.port_writes.clear()
        for path, comm, w1, w2, pc in self.grouper.feed(writes):
            m = self.ae.Message(path, comm, w1, w2, pc)
            if self.observer:
                self.observer(m)
            if self.timeline:
                self.timeline.message(self.tick, m.chip, m.path, m.word1, m.word2)
            self.router.dispatch(path, comm, m.core, m.dm_word, w2)


def return_observer(source):  # noqa: ANN001, ANN201
    """Feeder callback that mirrors every grouped ARM message into the return source."""
    if source is None:
        return None
    return lambda m: source.observe(m.comm, m.core, m.path, m.word1, m.word2)


def return_source(  # noqa: ANN001
    items, images: Path | None, poll: int = 0, before: list | None = None
) -> rtn.ReturnSource:
    """Return source with every live chip registered as a live layer (D147); `poll` > 0 runs one
    SY pass per `poll` reads of the same status word (`rtn.poll_advance`), after the callables of
    `before` (the message feed: the ARM sends a message and polls its effect inside one tick)."""
    source = rtn.ReturnSource(images=rtn.load_images(images, {1: "PNO", 2: "ORG"}))
    for spec, link in items:
        every = poll if spec.chip == "SY" else 0
        step = lambda link=link: ([f() for f in before or []], link.run(1))  # noqa: E731
        source.live[(spec.comm, spec.core)] = rtn.poll_advance(link.dump, step, every)
    return source


def return_report(source: rtn.ReturnSource, log: rtn.ReadLog) -> dict[str, object]:
    """Numbers of the return path: reads per chip and address, answering layers."""
    reads = {
        f"{rtn.CHIPS[c]}:{core}:{dm:#x}": n for (c, core, dm), n in sorted(source.reads.items())
    }
    return {
        "reads": reads,
        "answered_by": dict(source.layer),
        "calls": len(log.calls),
        "callers": {f"{k:#x}:{lr:#x}:{a:#x}": n for (k, lr, a), n in sorted(log.callers.items())},
        "wait_tests": {f"idx{i}:mask{m:#x}:{a:#x}": row for (i, m, a), row in log.waits.items()},
        "wait_ticks": {f"{a:#x}": span for a, span in log.wait_ticks.items()},
        "busy_voices": rtn.VOICES - bin(source.sy.idle).count("1"),
    }


class Chips:
    """The live chips of one run: spec, link and watched-word statistics."""

    def __init__(self, items: list[tuple[rt.ChipSpec, dl.Link]]) -> None:
        self.items = items
        self.watch = {s.name: rt.WatchStats() for s, _ in items}

    @property
    def lead(self) -> dl.Link:
        return self.items[0][1]

    def run(self, owed: int) -> dict[str, object]:
        """Run `owed` passes on every chip, then sample the watched words; reports by chip name."""
        reports = {}
        for spec, link in self.items:
            reports[spec.name] = link.run(owed)
            if spec.watch:
                words = {a: link.dump(a, 1)[0] for a in spec.watch}
                self.watch[spec.name].add(link.pass_index, words)
        return reports

    def sy_link(self):  # noqa: ANN201
        """Link of the live SY chip, `None` when SY is not live."""
        return next((link for s, link in self.items if s.chip == "SY"), None)
