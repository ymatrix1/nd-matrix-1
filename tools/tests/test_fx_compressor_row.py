"""Tests of tools/fx_compressor_row.py: row printing and the ARM write scan (synthetic files)."""

import json
import struct
import sys
from argparse import Namespace
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import fx_compressor_row as cr  # noqa: E402


def make_image(tmp: Path) -> Path:
    words = {cr.ROWS[0] - 0x17: 0x3F400000, cr.ROWS[0] - 0x16: 0x3F800000,
             cr.ROWS[0] - 0x15: 0x3C2AAAAA, cr.ROWS[0] - 0x14: 0x37800000,
             cr.ROWS[0] - 0x0B: 0x40C00000, cr.ROWS[0] - 0x09: 0x3F7DA3C2}  # fmt: skip
    img = bytearray(4 * (cr.ROWS[-1] + 4))
    for a, w in words.items():
        img[4 * a : 4 * a + 4] = struct.pack("<I", w)
    p = tmp / "core1.bin"
    p.write_bytes(bytes(img))
    return p


def test_f32_and_image_word_round_trip():
    assert cr.f32(0x3F400000) == 0.75 and cr.f32(0x40C00000) == 6.0
    img = struct.pack("<II", 1, 0x3F800000)
    assert cr.image_word(img, 1) == 0x3F800000


def test_row_prints_the_gain_law_words_of_the_image(tmp_path, capsys):
    assert cr.cmd_row(Namespace(image=str(make_image(tmp_path)))) == 0
    out = capsys.readouterr().out
    assert "[0.75, 1.0]" in out and "[6.0]" in out and "0.5" in out


def log_line(phase, core, dm, w2):
    return json.dumps({"phase": phase, "chip": "FX", "core": core, "dm": hex(dm),
                       "w1": hex(dm), "w2": w2})  # fmt: skip


def test_scan_reports_no_write_at_the_gain_law_offsets(tmp_path, capsys):
    i1 = cr.ROWS[0]
    lines = [log_line("boot", 1, i1 - 0x20, "0x3dd10ac8"), log_line("boot", 1, i1 - 0x1B, "0x1")]
    p = tmp_path / "log.jsonl"
    p.write_text("\n".join(lines))
    assert cr.cmd_scan(Namespace(log=str(p))) == 0
    assert "none" in capsys.readouterr().out
    lines.append(log_line("settle 2", 1, i1 - 0x15, "0x3f000000"))
    p.write_text("\n".join(lines))
    assert cr.cmd_scan(Namespace(log=str(p))) == 1
