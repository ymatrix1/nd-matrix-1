"""Install delta and oracle streams of the B3 Bass firmware machine (D128).

`docs/research/organ-farfisa-bass.md` s15. The instruction rows of the generator `0x121BBE` and the
processor `0x122296` and the DM image are no longer tracked: the Rust loader
`crates/nord-core/src/organ/b3_bass_fw/` builds them at run time from the user's OS update file.
What the loader cannot read from the DSP image is the DM state the ARM adds (boot transfers and the
model 5 install): `image` writes it as `docs/research/data/bass-fw-install-delta.csv`.

    python tools/organ_bass_fw_gen.py image target/PNO/core1_sram.bin SCRIPT OUT.csv \
        --code target/fw_unpacked/bank0_CODE.bin --exe target/release/sharc-chain
    python tools/organ_bass_fw_gen.py golden target/PNO/core1_sram.bin SCRIPT OUT.csv --calls 64 \
        --exe target/release/sharc-chain

`golden` writes the oracle output words of the chain.

Delta lines: `R,name,hex` start registers, `M,addr,hex ...` inline words (a `0` clears a loader
word) and `A,addr,count,offset` (hex): `count` words the ARM sends from its constants, which the
user's `bank0_CODE` holds at byte `offset`. The golden stream comes from `sharc-chain` (the oracle);
the Python VM replay (`organ_replay.py`) is the reference for it and is itself checked against it.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import organ_replay as rp

# VERIFIED: DM clusters the machine touches (words of the oracle image merged within 0x40), as
# `KEEP` of `crates/nord-core/src/organ/b3_bass_fw/image.rs` (`ranges()` regenerates the table).
GEN = ("gen", 0x121BBE, 0x121E28)
PROC = ("proc", 0x122296, 0x12261D)
CLUSTER_GAP = 0x40
# Shortest run worth an `A` line (shorter runs stay inline).
MIN_A = 8


def ranges(words: dict[int, int], gap: int = CLUSTER_GAP) -> list[tuple[int, int]]:
    """`(base, count)` clusters of the addresses of `words` (a gap above `gap` starts a new one)."""
    addrs = sorted(words)
    out: list[tuple[int, int]] = []
    start = prev = addrs[0]
    for a in addrs[1:]:
        if a - prev > gap:
            out.append((start, prev - start + 1))
            start = a
        prev = a
    out.append((start, prev - start + 1))
    return out


def static_words(sram: bytes, keep: list[tuple[int, int]]) -> dict[int, int]:
    """Non zero DM words of the `keep` ranges in a flat core 1 SRAM image (`4 * address`)."""
    out = {}
    for base, n in keep:
        for a in range(base, base + n):
            v = int.from_bytes(sram[4 * a : 4 * a + 4], "little")
            if v:
                out[a] = v
    return out


def install_delta(now: dict[int, int], static: dict[int, int]) -> list[tuple[int, int]]:
    """`(address, value)` of the words whose oracle value differs from the loader data.

    A word the install clears appears with value 0.
    """
    return sorted(
        (a, now.get(a, 0)) for a in set(now) | set(static) if now.get(a, 0) != static.get(a, 0)
    )


def _pack(words: list[int]) -> bytes:
    return b"".join(w.to_bytes(4, "little") for w in words)


def _match(code: bytes, words: list[int]) -> tuple[int, int]:
    """`(offset, length)` of the longest prefix of `words` that `code` holds (length 0: none)."""
    best = (0, 0)
    probe = _pack(words[:MIN_A])
    at = code.find(probe)
    while at >= 0:
        n = MIN_A
        while n < len(words) and code[at + 4 * n : at + 4 * n + 4] == _pack(words[n : n + 1]):
            n += 1
        if n > best[1]:
            best = (at, n)
        at = code.find(probe, at + 1)
    return best


def delta_lines(delta: list[tuple[int, int]], code: bytes) -> list[str]:
    """`A` and `M` lines of the delta words: runs the user's `bank0_CODE` holds become `A` lines."""
    lines: list[str] = []
    inline: list[tuple[int, int]] = []

    def flush() -> None:
        lines.extend(mem_runs(inline))
        inline.clear()

    k = 0
    while k < len(delta):
        end = k + 1
        while end < len(delta) and delta[end][0] == delta[end - 1][0] + 1:
            end += 1
        run = delta[k:end]
        p = 0
        while p < len(run):
            off, n = _match(code, [v for _a, v in run[p:]]) if len(run) - p >= MIN_A else (0, 0)
            if n >= MIN_A and all(v for _a, v in run[p : p + n]):
                flush()
                lines.append(f"A,{run[p][0]:x},{n:x},{off:x}")
                p += n
            else:
                inline.append(run[p])
                p += 1
        k = end
    flush()
    return lines


def chain_args(a: argparse.Namespace) -> argparse.Namespace:
    a.gen = "0x121bbe:0x121e28:I0=0xc00f2,I1=0xb52c7"
    a.proc = "0x122296:0x12261d:I0=0xc00ea,I1=0xb472c"
    return a


def parse_image_csv(text: str) -> tuple[list[str], dict[int, int]]:
    """`(R lines, words)` of a full DM image text (`R,name,hex` / `M,addr,hex hex ...`)."""
    regs: list[str] = []
    words: dict[int, int] = {}
    for line in text.splitlines():
        f = line.split(",")
        if f[0] == "R":
            regs.append(line)
        elif f[0] == "M":
            base = int(f[1], 16)
            words.update({base + k: int(w, 16) for k, w in enumerate(f[2].split(" "))})
    return regs, words


def build_delta(
    regs: list[str], now: dict[int, int], sram: bytes, code: bytes
) -> tuple[list[str], list[tuple[int, int]], int]:
    """`(delta lines, keep ranges, delta words)` of a full DM image against the loader data."""
    now = {k: v for k, v in now.items() if v}
    keep = ranges(now)
    delta = install_delta(now, static_words(sram, keep))
    lines = ["# B3 Bass start registers and install words (D128), research s15", *regs]
    return lines + delta_lines(delta, code), keep, len(delta)


def write_delta(lines: list[str], keep: list[tuple[int, int]], words: int, out: str) -> None:
    Path(out).write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"{words} delta words, {len(lines)} lines -> {out}")
    print("KEEP =", ", ".join(f"(0x{b:X}, 0x{n:X})" for b, n in keep))


def cmd_image(a: argparse.Namespace) -> int:  # pragma: no cover - runs the oracle
    chain_args(a)
    mem = rp.memory(a, ())
    row0 = rp.trace_rows(a, 0x121BBE, 0x121E28, (a.gen,))[0]
    _pc, regs = rp.vm.parse_trace_line(row0)
    reg_lines = [f"R,{file}{k},{v:x}" for file in "rsim" for k, v in enumerate(regs[file]) if v]
    lines, keep, n = build_delta(
        reg_lines, mem, Path(a.image).read_bytes(), Path(a.code).read_bytes()
    )
    write_delta(lines, keep, n, a.out)
    return 0


def cmd_convert(a: argparse.Namespace) -> int:
    """Delta of a full DM image text (the former `bass-fw-image.csv`, from git history)."""
    regs, words = parse_image_csv(Path(a.script).read_text(encoding="utf-8"))
    lines, keep, n = build_delta(regs, words, Path(a.image).read_bytes(), Path(a.code).read_bytes())
    write_delta(lines, keep, n, a.out)
    return 0


def mem_runs(words: list[tuple[int, int]]) -> list[str]:
    """`M,addr,hex hex ...` lines of sorted `(addr, value)` pairs (consecutive addresses merged)."""
    lines: list[str] = []
    run: list[int] = []
    start = 0
    for addr, v in words:
        if run and addr != start + len(run):
            lines.append(f"M,{start:x}," + " ".join(f"{w:x}" for w in run))
            run = []
        if not run:
            start = addr
        run.append(v)
    if run:
        lines.append(f"M,{start:x}," + " ".join(f"{w:x}" for w in run))
    return lines


def cmd_delta(a: argparse.Namespace) -> int:  # pragma: no cover - runs the oracle
    """Words that differ between the start memory of `script` and of `script2`."""
    chain_args(a)
    base = rp.memory(a, ())
    a.script = a.script2
    other = rp.memory(a, ())
    words = [(k, other.get(k, 0)) for k in rp.diff(base, other)]
    lines = ["# DM words that differ from the image (key-down of a chord); research s15"]
    Path(a.out).write_text(
        "\n".join(lines + mem_runs(words)) + "\n", encoding="utf-8", newline="\n"
    )
    print(f"{len(words)} words -> {a.out}")
    return 0


def cmd_golden(a: argparse.Namespace) -> int:  # pragma: no cover - runs the oracle
    chain_args(a)
    want, _mem = rp.oracle_calls(a, a.calls)
    lines = ["# call,out0,out1 (hex words of DM 0xC00EA and 0xC00EC after each call; oracle)"]
    lines += [f"{c},{row[0]:x},{row[2]:x}" for c, row in enumerate(want.tolist())]
    Path(a.out).write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("cmd", choices=("image", "convert", "golden", "delta"))
    ap.add_argument("image")
    ap.add_argument("script", nargs="?")
    ap.add_argument("out", nargs="?")
    ap.add_argument("--script2")
    ap.add_argument("--code", default="target/fw_unpacked/bank0_CODE.bin")
    ap.add_argument("--calls", type=int, default=64)
    ap.add_argument("--exe", default="sharc-chain")
    a = ap.parse_args(argv)
    cmds = {"image": cmd_image, "convert": cmd_convert, "golden": cmd_golden, "delta": cmd_delta}
    return cmds[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
