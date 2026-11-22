#!/usr/bin/env python3
"""Replay the messages of a SY1SY2 or SMP chip into `sharc-live` and read the FIFO consumer (D206).

The audio ISR of both sample images computes `n = (DM(0x2400) + R3 - I0) >> 1` pairs per interrupt
and caps `n` at a cap word. The SMP image has `R3 = 0x80000` (D199: the model stores `write
address - 0x80000`); the SY1SY2 listing sets `R3 = 0x80000` in the same init block (`b0 = 0x93046`,
`l0 = 0x280`, `r2 = 0x280`, `r3 = 0x80000`, listing offset `0xAA7`) but its model stored the plain
address, so every interrupt saw `n` above the cap and replayed the ring. This tool replays a
`--log` script with `--probe PC` at the `r1 = r1 + r3` and prints the register
snapshots (`S`), the pairs and resets of the model, so the two models can be compared:

    python tools/sy_fifo_probe.py LOG/SY0.script sy.bin sharc-live.exe --probe 0x1C063F \
        --passes 400 --fifo-pace 1 --index-bias 0x80000
"""

from __future__ import annotations

import argparse
import re
from collections import defaultdict
from pathlib import Path

SNAP = re.compile(r"r=([0-9a-f,]+);i=([0-9a-f,]+);l=([0-9a-f,]+)")


def parse_snapshots(reply: str) -> list[dict[str, list[int]]]:
    """`S r=..;i=..;l=..` register snapshots as `{r, i, l}` integer lists."""
    return [
        {k: [int(x, 16) for x in v.split(",")] for k, v in zip("ril", m.groups(), strict=True)}
        for m in SNAP.finditer(reply)
    ]


def parse_report(line: str) -> dict[str, int]:
    """Numbers of the `sy=isrs:N,resets:N,pairs:N,queued:N,...` token of a `P` reply."""
    m = re.search(r"sy=(\S+)", line)
    out: dict[str, int] = {}
    for item in m.group(1).split(",") if m else []:
        key, _, value = item.partition(":")
        if value.isdigit():
            out[key] = int(value)
    return out


def read_script(path: Path) -> dict[int, list[tuple[int, int]]]:
    """`PASS ADDR VALUE` lines of `arm_dsp_live --log` as messages per pass."""
    out: dict[int, list[tuple[int, int]]] = defaultdict(list)
    for line in path.read_text("utf-8").splitlines():
        p, a, v = line.split()
        out[int(p)].append((int(a, 16), int(v, 16)))
    return out


def summarize(snaps: list[dict[str, list[int]]], report: dict[str, int]) -> dict[str, object]:
    """R3 values, consumer entries (probe hits), pairs per reset and the queue left over."""
    r3 = sorted({s["r"][3] for s in snaps})
    resets = report.get("resets", 0)
    return {
        "r3_values": [hex(v) for v in r3],
        "snapshots": len(snaps),
        "resets": resets,
        "pairs": report.get("pairs", 0),
        "pairs_per_reset": report.get("pairs", 0) / resets if resets else 0.0,
        "queued": report.get("queued", 0),
    }


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs sharc-live
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("script", type=Path)
    ap.add_argument("image", type=Path)
    ap.add_argument("live", type=Path)
    ap.add_argument("--chip", default="--sy", choices=["--sy", "--smp"])
    ap.add_argument("--probe", default="0x1C063F")
    ap.add_argument("--passes", type=int, default=400)
    ap.add_argument("--fifo-pace", default="1")
    ap.add_argument("--index-bias", default=None)
    ns = ap.parse_args(argv)
    import arm_dsp_link as dl

    flags = [ns.chip, "--epchain", "--fifo-pace", ns.fifo_pace, "--probe", ns.probe]
    if ns.index_bias is not None:
        flags += ["--index-bias", ns.index_bias]
    link = dl.Link.spawn(str(ns.live), str(ns.image), *flags)
    msgs = read_script(ns.script)
    first = min(msgs) if msgs else 0
    last, snaps = "", []
    for p in range(ns.passes):
        for a, v in msgs.get(p, []):
            link.send(a, v)
        link._flush()  # noqa: SLF001
        link.proc.stdin.write("P 1\n")
        link.proc.stdin.flush()
        last = link._readline().strip()  # noqa: SLF001
        taken = parse_snapshots(link.command("S"))
        snaps += taken if p >= first else []  # before the first message the FIFO is idle
    link.close()
    print(summarize(snaps, parse_report(last)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
