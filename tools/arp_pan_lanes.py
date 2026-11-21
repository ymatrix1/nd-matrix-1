#!/usr/bin/env python3
"""Arp step pan is the lane select of the synth key word (OQ40, D175).

The firmware sends one key word per arp step to PNO core 2 (`DM(0xC1DCC + n)`, see
`arm_key_words.py`). Its bank bits 1..0 are **not** voice banks: they select the output lane of the
voice record `DM(0xC1F89 ..)`, which interleaves the lanes word by word.

| Pattern pan (`ns4_arp_patterns` `PAN`) | ARM pan code (`r3` of `0xC01E81DC`) | Bank bits | Lane |
|---|---|---|---|
| `C` centre | 0 | `0x03` | both |
| `L` left | 1 | `0x02` | even words (`0xC1F89`, `+2`, ..) |
| `R` right | 2 | `0x01` | odd words (`0xC1F8A`, `+2`, ..) |

so the bank bits are `3 - code`, and the census window `0xC1F89:4:2` (even words) saw left and
centre steps only. Evidence: [arp-pan-lanes](../docs/research/arp-pan-lanes.md).

Run as a script it replays a recorded PNO core 2 message log (`arm_key_words.py` output of a
`--chip PNO:2=org=...` run) with one key word of chosen flags into `sharc-sys` and prints the peak
of each lane:

    python tools/arp_pan_lanes.py key_words.json CORE2_SRAM sharc-sys.exe

Only numbers are printed. Needs the local firmware run and a built `sharc-sys`.
"""

from __future__ import annotations

import array
import json
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import arm_dsp_route as rt  # noqa: E402

PAN_CODE_OF_CHAR = {"C": 0, "L": 1, "R": 2}
VOICE_RECORD = 0xC1F89  # even words: left lane, odd words: right lane (EMPIRICAL, this tool)
KEY_WORD_BASE = 0xC1DCC
KEY_PASS = 300
PASSES = 1500


def banks_of_pan(char: str) -> int:
    """Bank bits of the key word for a pattern pan character `C`, `L` or `R`."""
    return 3 - PAN_CODE_OF_CHAR[char]


def pan_of_banks(banks: int) -> str | None:
    """Pattern pan character of the bank bits of an on word, `None` for an off word."""
    return {3: "C", 2: "L", 1: "R"}.get(banks & 3)


def key_word(flags: int, velocity_byte: int = 0xE2, note_offset: int = 0) -> int:
    """A key word with the given bank bits and no keyboard flag (arp step), `0x0?00E200` at 60."""
    return (flags & 0xFF) << 24 | (note_offset & 0xFF) << 16 | velocity_byte << 8


def lane_peaks(raw: bytes, width: int = 8) -> tuple[float, float]:
    """`(left, right)` peak of a `sharc-sys --tap-out` file of `width` words from `VOICE_RECORD`.

    Even words of the window are the left lane, odd words the right lane; NaN and huge words are
    skipped.
    """
    words = array.array("f")
    words.frombytes(raw[: len(raw) // 4 * 4])
    peaks = [0.0, 0.0]
    for base in range(0, len(words) - width + 1, width):
        for k in range(width):
            x = words[base + k]
            if x == x and abs(x) < 1e6:
                peaks[k % 2] = max(peaks[k % 2], abs(x))
    return peaks[0], peaks[1]


def replay(
    log: list[list[int]], core2_image: Path, sharc_sys: Path, flags: int
) -> tuple[float, float]:  # pragma: no cover - needs local files
    """Lane peaks after the recorded load stream of PNO core 2 and one key word with `flags`."""
    base = [(0, row[4], row[5]) for row in log if row[2] == 2 and row[3] == 2 and row[0] < 4000]
    base = [r for r in base if not KEY_WORD_BASE <= r[1] < KEY_WORD_BASE + 16 or r[2] == 0x04000000]
    script = rt.merged_script([*base, (KEY_PASS, KEY_WORD_BASE, key_word(flags))])
    with tempfile.TemporaryDirectory() as tmp:
        script_path, tap = Path(tmp, "script.txt"), Path(tmp, "tap.bin")
        script_path.write_text(script, encoding="utf-8")
        cmd = [str(sharc_sys), str(core2_image), "--epdma", "--warmup", "0"]
        cmd += ["--passes", str(PASSES), "--msg-script", str(script_path)]
        cmd += ["--tap", f"{VOICE_RECORD:#x}:8", "--tap-out", str(tap)]
        subprocess.run(cmd, capture_output=True, text=True, check=True)
        return lane_peaks(tap.read_bytes())


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - thin CLI
    args = sys.argv[1:] if argv is None else argv
    log = json.loads(Path(args[0]).read_text(encoding="utf-8"))
    image, exe = Path(args[1]), Path(args[2])
    for char in "CLR":
        left, right = replay(log, image, exe, banks_of_pan(char))
        flags = banks_of_pan(char)
        print(f"pan {char} (flags {flags:#x}): left peak {left:.4f}, right peak {right:.4f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
