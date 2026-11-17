"""Unit tests for tools/arm_voice_gate.py (gate predicate, row and summary helpers)."""

from __future__ import annotations

import json
from pathlib import Path

import arm_voice_gate as avg
import pytest


@pytest.mark.parametrize(
    ("word", "magnitude", "defer"),
    [
        (0x00000000, 0, False),
        (0x00010001, 1, False),
        (0x00000003, 3, True),
        (0x00030003, 3, True),
        (0x0000003F, 63, True),
        (0xFFFFFFFF, 1, False),  # byte 0xFF is -1: magnitude 1, no defer
        (0x80000000, 0, False),
        (0x000000FD, 3, True),  # -3
    ],
)
def test_entry_magnitude_and_defer(word: int, magnitude: int, defer: bool) -> None:
    assert avg.entry_magnitude(word) == magnitude
    assert avg.defers(word) is defer


def run(updates: int, spi: int | None) -> dict:
    return avg.mode_row(updates, 17 * updates, spi)


def test_program_row_fixed_and_prediction() -> None:
    stopped = avg.program_row("A", run(0, 3), run(1, 63))
    assert stopped["stopped"] and stopped["fixed"] and stopped["predicted_stop"] is True
    voiced = avg.program_row("B", run(1, 1), run(1, 1))
    assert not voiced["stopped"] and not voiced["fixed"] and voiced["predicted_stop"] is False
    silent = avg.program_row("C", run(0, None), run(0, None))
    assert silent["stopped"] and not silent["fixed"] and silent["predicted_stop"] is None


def test_summarize_counts(tmp_path: Path) -> None:
    rows = [
        avg.program_row("A", run(0, 3), run(1, 0)),
        avg.program_row("B", run(2, 1), run(2, 1)),
        avg.program_row("C", run(0, None), run(0, None)),
    ]
    for r in rows:
        (tmp_path / f"{r['program']}.json").write_text(json.dumps(r))
    out = avg.summarize(avg.read_rows(tmp_path))
    assert out["programs"] == 3
    assert out["gated_voiced"] == 1
    assert out["gated_stopped"] == 2
    assert out["neutral_voiced"] == 2
    assert out["fixed"] == 1
    assert out["prediction_agrees"] == 2  # C has no prediction, so None != True
    assert out["spi_words"]["0x3"] == 1


def test_read_rows_accepts_a_single_list_file(tmp_path: Path) -> None:
    rows = [avg.program_row("A", run(0, 3), run(1, 0)), avg.program_row("B", run(1, 1), run(1, 1))]
    path = tmp_path / "rows.json"
    path.write_text(json.dumps(rows))
    assert avg.read_rows(path) == rows
    assert avg.summarize(avg.read_rows(path))["fixed"] == 1
