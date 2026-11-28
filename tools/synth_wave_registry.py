#!/usr/bin/env python3
"""Reads the WAVE bank table registry of the OS image (`bank0_CODE.bin`) and prints a census.

The registry (`docs/research/synth-stage-models-13.md` s2) is an array of 28-byte records in
rodata at `0xC02FD4B0`: `name_ptr, rows_end, 8, rows_ptr, 512, data_ptr, count` (little endian
words; `rows_end = rows_ptr + 0x800`, `data_ptr` points at `count + 1` int16 samples). The first 46
records are the WAVE waves in table-id order (`Second` = 0 .. `Tubular Bells` = 45), the next nine
are vowel tables. Only addresses, counts and names are derived here; the samples stay in the
firmware image and are never written out.

    python tools/synth_wave_registry.py target/fw_unpacked/bank0_CODE.bin
    python tools/synth_wave_registry.py target/fw_unpacked/bank0_CODE.bin --csv out.csv
    python tools/synth_wave_registry.py --check          # committed CSV vs the documented totals
    python tools/synth_wave_registry.py CODE --boot-check  # rebuilt entry 0 vs the boot table

The DSP layout of a table (`build_l2`, VERIFIED run against the boot capture: 1970 of 1970 samples
and 128 of 128 band rows of entry 0 equal) re-lays the unique bands of the rodata table in
ascending source order with a stride of `count + 12` and appends the first 12 samples of each band
cyclically; the row words `(offset, count, spare, spare)` get the new offset.
"""

from __future__ import annotations

import argparse
import csv
import struct
import sys
from dataclasses import dataclass
from pathlib import Path

LOAD_BASE = 0xC0000000
# VERIFIED (run of this tool on OS v1.66 `bank0_CODE.bin`): first record of the registry.
REGISTRY = 0xC02FD4B0
RECORD_BYTES = 28
ROWS_WORDS = 0x200
WAVE_COUNT = 46  # table ids 0..45 (docs/research/data/synth-wave-categories.csv)
CSV_PATH = Path(__file__).resolve().parent.parent / "docs/research/data/synth-wave-registry.csv"
# VERIFIED (run): totals over the 55 records, used by `--check`.
TOTAL_RECORDS = 55
TOTAL_SAMPLES = 605_164
GUARD = 12  # cyclic samples appended to every band in the DSP layout
BANDS = 128
TAIL_TRIM = 2  # entry 0: 1,970 samples, two short of the last band's full guard


@dataclass(frozen=True)
class Entry:
    """One registry record: names and addresses only."""

    index: int
    name: str
    rows_ptr: int
    data_ptr: int
    count: int  # last sample index; the table holds `count + 1` int16 samples


def _word(code: bytes, addr: int) -> int:
    return struct.unpack_from("<I", code, addr - LOAD_BASE)[0]


def _cstr(code: bytes, addr: int) -> str:
    off = addr - LOAD_BASE
    return code[off : code.index(b"\0", off)].decode("latin1")


def parse(code: bytes, start: int = REGISTRY) -> list[Entry]:
    """Records from `start` while the fixed fields (`8`, `512`, `rows_end`) hold."""
    out: list[Entry] = []
    p = start
    while p - LOAD_BASE + RECORD_BYTES <= len(code):
        name, end, eight, rows, n, data, count = (_word(code, p + 4 * j) for j in range(7))
        if eight != 8 or n != ROWS_WORDS or end != rows + 4 * ROWS_WORDS:
            break
        out.append(Entry(len(out), _cstr(code, name), rows, data, count))
        p += RECORD_BYTES
    return out


def find_registry(code: bytes, minimum: int = WAVE_COUNT) -> int:
    """Address of record 0: `REGISTRY` (OS v1.66) when `minimum` records hold there, else the
    first 4-aligned address that does (OS v1.68 has it at `0xC02FD850`); `REGISTRY` if none."""
    if len(parse(code, REGISTRY)) >= minimum:
        return REGISTRY
    n = len(code)
    for off in range(0, n - RECORD_BYTES * minimum, 4):
        # cheap reject on the two fixed words before the full parse
        if (
            struct.unpack_from("<2I", code, off + 8)[0] != 8
            or struct.unpack_from("<I", code, off + 16)[0] != ROWS_WORDS
        ):
            continue
        try:
            if len(parse(code, LOAD_BASE + off)) >= minimum:
                return LOAD_BASE + off
        except (ValueError, struct.error):
            continue
    return REGISTRY


def build_l2(code: bytes, entry: Entry) -> tuple[list[int], list[int]]:
    """`(rows, samples)` of the DSP layout of one table (PM rows and L2 samples, int16 values)."""
    rows_in = [_word(code, entry.rows_ptr + 4 * i) for i in range(ROWS_WORDS)]
    off = entry.data_ptr - LOAD_BASE
    src = list(struct.unpack_from(f"<{entry.count + 1}H", code, off))
    bands = sorted({(rows_in[4 * b], rows_in[4 * b + 1]) for b in range(BANDS)})
    new_off: dict[tuple[int, int], int] = {}
    out: list[int] = []
    for o, c in bands:
        new_off[(o, c)] = len(out)
        band = src[o : o + c]
        out += [band[i % c] for i in range(c + GUARD)]
    rows = list(rows_in)
    for b in range(BANDS):
        rows[4 * b] = new_off[(rows_in[4 * b], rows_in[4 * b + 1])]
    return rows, out[: len(out) - TAIL_TRIM]


def to_csv(entries: list[Entry]) -> str:
    lines = ["index,name,samples,data_addr,rows_addr"]
    for e in entries:
        lines.append(f"{e.index},{e.name},{e.count + 1},{e.data_ptr:#x},{e.rows_ptr:#x}")
    return "\n".join(lines) + "\n"


def load_csv(path: Path = CSV_PATH) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8") as fh:
        return list(csv.DictReader(fh))


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - thin CLI
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("code", nargs="?", type=Path)
    ap.add_argument("--csv", type=Path)
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--boot-check", action="store_true")
    ns = ap.parse_args(argv)
    if ns.check:
        rows = load_csv()
        total = sum(int(r["samples"]) for r in rows)
        print(len(rows), total)
        return 0 if (len(rows), total) == (TOTAL_RECORDS, TOTAL_SAMPLES) else 1
    if ns.code is None:
        ap.error("code image required")
    code = ns.code.read_bytes()
    entries = parse(code, find_registry(code))
    if ns.boot_check:
        import synth_super24_tables as boot

        tb = boot.load()
        rows, l2 = build_l2(code, entries[0])
        want = [h for w in tb["wave"] for h in (w & 0xFFFF, w >> 16)]
        ok = rows == tb["rows"][:ROWS_WORDS] and l2 == want
        print(f"entry 0 rebuilt: rows {len(rows)}, samples {len(l2)}, equal {ok}")
        return 0 if ok else 1
    text = to_csv(entries)
    if ns.csv:
        ns.csv.write_text(text, encoding="utf-8", newline="\n")
    print(f"{len(entries)} records, {sum(e.count + 1 for e in entries)} samples")
    return 0


if __name__ == "__main__":
    sys.exit(main())
