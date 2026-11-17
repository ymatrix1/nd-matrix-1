#!/usr/bin/env python3
"""Live link between the ARM harness and one emulated DSP chip (D138).

Pure parts (no firmware, no unicorn): the clock coupling, the incremental port grouper and the
line-protocol client of `sharc-live` (`crates/nord-sharc/src/runner/live.rs`). The firmware driver
is `tools/arm_dsp_live.py`; design, evidence and limits: `docs/research/arm-dsp-live-link.md`.

Clock coupling. One ARM timer tick is 1/24,000 s (D130, 24 kHz). The DSP pass period is two audio
frames, `2 / F` with `F = 43,402.34375 Hz = 1,388,875 / 32` (`dsp-frame-rate.md`), so passes per
second are `1,388,875 / 64` and passes per tick `1,388,875 / 1,536,000` = 0.9042. The number of
passes owed after `t` ticks is the integer `floor(t * 1,388,875 / 1,536,000)`: no float, so two
runs of the same ARM instruction stream give the same pass schedule.
"""

from __future__ import annotations

import queue
import subprocess
import threading
from collections.abc import Iterable
from dataclasses import dataclass, field

# F = 43402.34375 Hz is the audio frame rate of `docs/research/dsp-frame-rate.md`; only the identity
# F = 1,388,875 / 32 (exact) is arithmetic here, the value of F keeps that note's tag. Passes/s =
# F / 2 (two frames per pass), ticks/s = 24,000 (D130).
PASS_NUM, PASS_DEN = 1_388_875, 1_536_000


def passes_due(ticks: int) -> int:
    """Total DSP passes that have elapsed after `ticks` ARM timer ticks (24 kHz), integer exact."""
    return ticks * PASS_NUM // PASS_DEN


class Clock:
    """Counts ARM timer ticks and says how many DSP passes to run now."""

    def __init__(self) -> None:
        self.ticks = 0
        self.passes = 0

    def tick(self, count: int = 1) -> int:
        """Advance `count` ticks; returns the passes to run to catch up."""
        self.ticks += count
        owed = passes_due(self.ticks) - self.passes
        self.passes += owed
        return owed


class PortGrouper:
    """Incremental `arm_emu.group_port_writes`: partial groups wait for the next feed.

    `ports` maps a port address to `(path, comm, need)`; stores to other addresses are ignored.
    Feeding the writes in any split gives the same messages as grouping them in one call.
    """

    def __init__(self, ports: dict[int, tuple[int, int, int]]) -> None:
        self.ports = ports
        self.pending: dict[int, list[tuple[int, int]]] = {}

    def feed(self, writes: Iterable[tuple[int, int, int]]) -> list[tuple[int, int, int, int, int]]:
        """`(port, value, pc)` stores to complete messages `(path, comm, word1, word2, pc)`."""
        out = []
        for port, value, pc in writes:
            spec = self.ports.get(port)
            if spec is None:
                continue
            path, comm, need = spec
            buf = self.pending.setdefault(port, [])
            buf.append((value, pc))
            if len(buf) == need:
                out.append((path, comm, buf[0][0], buf[-1][0], buf[-1][1]))
                buf.clear()
        return out


def port_table(
    port2_base: int, port1_base: int, comm_count: int, pad: Iterable[int]
) -> dict[int, tuple[int, int, int]]:
    """The `PortGrouper` table for the fake port layout of `arm_emu` (`pad[c]` = 1: 3 stores)."""
    table = {}
    for c, p in zip(range(comm_count), pad, strict=False):
        table[port2_base + 0x10 * c] = (2, c, 3 if p else 2)
        table[port1_base + 0x10 * c] = (1, c, 2)
    return table


@dataclass
class PassReport:
    """One `P` reply of `sharc-live`."""

    req: int = 0
    idle: int = 0
    steps: int = 0
    last: int = 0
    pass_: int = 0
    halted: bool = False
    cnt: int = 0
    sumsq: float = 0.0
    peak: float = 0.0
    hits: dict[int, int] = field(default_factory=dict)
    error: str = ""

    @staticmethod
    def parse(line: str) -> PassReport:
        """Parse `P req=.. idle=.. ...`; raises `ValueError` on any other line."""
        parts = line.split()
        if not parts or parts[0] != "P":
            raise ValueError(f"not a pass reply: {line!r}")
        kv = dict(p.split("=", 1) for p in parts[1:])
        hits = {}
        if kv.get("hits"):
            for item in kv["hits"].split(","):
                pc, n = item.split(":")
                hits[int(pc, 16)] = int(n)
        return PassReport(
            req=int(kv["req"]),
            idle=int(kv["idle"]),
            steps=int(kv["steps"]),
            last=int(kv["last"]),
            pass_=int(kv["pass"]),
            halted=kv["halted"] == "1",
            cnt=int(kv["cnt"]),
            sumsq=float(kv["sumsq"]),
            peak=float(kv["peak"]),
            hits=hits,
            error=kv.get("error", ""),
        )


@dataclass
class LinkStats:
    """Totals over every `run` of a link."""

    messages: int = 0
    words: set[int] = field(default_factory=set)
    idle: int = 0
    steps: int = 0
    cnt: int = 0
    sumsq: float = 0.0
    peak: float = 0.0
    hits: dict[int, int] = field(default_factory=dict)
    first_hit: dict[int, int] = field(default_factory=dict)
    halted: bool = False
    errors: list[str] = field(default_factory=list)

    def add(self, r: PassReport, first_pass: int) -> None:
        """Fold one reply in; `first_pass` is the pass index the reply's batch started at."""
        self.idle += r.idle
        self.steps += r.steps
        self.cnt += r.cnt
        self.sumsq += r.sumsq
        self.peak = max(self.peak, r.peak)
        self.halted = self.halted or r.halted
        for pc, n in r.hits.items():
            self.hits[pc] = self.hits.get(pc, 0) + n
            if n and pc not in self.first_hit:
                self.first_hit[pc] = first_pass
        if r.error:
            self.errors.append(r.error)

    @property
    def rms(self) -> float:
        return (self.sumsq / self.cnt) ** 0.5 if self.cnt else 0.0


class Link:
    """Client of one `sharc-live` process (or any object with `stdin` / `stdout` text streams)."""

    def __init__(  # noqa: ANN001
        self, proc, log: list[tuple[int, int, int]] | None = None, timeout: float = 600.0
    ) -> None:
        self.proc = proc
        self.stats = LinkStats()
        self.pass_index = 0
        self.log = log if log is not None else []
        self._buf: list[str] = []
        self.timeout = timeout
        self._lines: queue.Queue[str] = queue.Queue()
        threading.Thread(target=self._pump, daemon=True).start()
        ready = self._readline().strip()
        if ready != "READY":
            raise RuntimeError(f"sharc-live did not start: {ready!r}")

    def _pump(self) -> None:
        """Reader thread: stdout lines to a queue (a blocking pipe has no portable timeout)."""
        for line in iter(self.proc.stdout.readline, ""):
            self._lines.put(line)
        self._lines.put("")  # EOF marker

    def _readline(self) -> str:
        """Next reply line; raises `TimeoutError` when the server stays silent for `timeout` s."""
        try:
            return self._lines.get(timeout=self.timeout)
        except queue.Empty:
            raise TimeoutError(f"sharc-live silent for {self.timeout} s") from None

    @classmethod
    def spawn(cls, exe: str, image: str, *flags: str) -> Link:  # pragma: no cover - subprocess
        proc = subprocess.Popen(  # noqa: S603
            [exe, image, *flags],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        return cls(proc)

    def send(self, dm_word: int, value: int) -> None:
        """Queue one message (data word, value); delivered before the next pass."""
        self._buf.append(f"M {dm_word:#x} {value:#x}\n")
        self.log.append((self.pass_index, dm_word, value))
        self.stats.messages += 1
        self.stats.words.add(dm_word)

    def _flush(self) -> None:
        if self._buf:
            self.proc.stdin.write("".join(self._buf))
            self._buf.clear()

    def run(self, n: int) -> PassReport:
        """Run `n` passes; returns the reply and folds it into `stats`."""
        self._flush()
        self.proc.stdin.write(f"P {n}\n")
        self.proc.stdin.flush()
        line = self._readline().strip()
        if line.startswith("ERR"):
            raise RuntimeError(line)
        report = PassReport.parse(line)
        self.stats.add(report, self.pass_index)
        self.pass_index = report.pass_
        return report

    def dump(self, addr: int, count: int) -> list[int]:
        """Data words `addr..addr + count` (after the queued messages)."""
        self._flush()
        self.proc.stdin.write(f"D {addr:#x} {count}\n")
        self.proc.stdin.flush()
        parts = self._readline().split()
        if parts[:1] == ["ERR"]:
            raise RuntimeError(" ".join(parts))
        return [int(w, 16) for w in parts[2:]]

    def close(self) -> None:
        """Ask the server to end; kill it when it does not (also safe after a failed run)."""
        try:
            self._flush()
            self.proc.stdin.write("Q\n")
            self.proc.stdin.flush()
        except (OSError, ValueError):
            pass
        if hasattr(self.proc, "wait"):
            try:
                self.proc.wait(timeout=30)
            except subprocess.TimeoutExpired:
                self.proc.kill()


def script_text(log: Iterable[tuple[int, int, int]]) -> str:
    """The sent messages as a `sharc-sys --msg-script` file (`PASS W1 W2` per line)."""
    return "".join(f"{p} {w:#x} {v:#010x}\n" for p, w, v in log)
