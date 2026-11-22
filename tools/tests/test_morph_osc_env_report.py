"""Tests for `tools/morph_osc_env_report.py` (synthetic harness output, no firmware)."""

from __future__ import annotations

import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import morph_osc_env_report as rep  # noqa: E402


def word(x: float) -> str:
    return f"{struct.unpack('>I', struct.pack('>f', x))[0]:#010x}"


LOG = "CELL 0x91097560 4 synth Edt_Syn_kOscEnvAmt (21, (2, 0, 0))\n"


def csv_text(rows: list[tuple]) -> str:
    head = "scenario,store,client,kind,id,name,value\n"
    return head + "".join(",".join(map(str, r)) + "\n" for r in rows)


def test_weight_is_v_over_128_and_one_at_127() -> None:
    assert rep.weight(64) == 0.5
    assert rep.weight(127) == 1.0
    assert rep.weight(0) == 0.0


def test_law_sums_clamps_and_keeps_fractions() -> None:
    assert rep.law(21, (2, 0, 0), {"wheel": 64}) == 22.0
    assert rep.law(60, (-45, 0, 0), {"wheel": 64}) == 37.5
    assert rep.law(50, (40, 0, 0), {"wheel": 127}) == 60.0
    assert rep.law(-50, (-40, 0, 0), {"wheel": 127}) == -60.0
    assert rep.law(30, (10, 20, -40), {"wheel": 127, "at": 127, "pedal": 127}) == 20.0


def test_parse_cells_maps_client_to_layer() -> None:
    log = LOG + "CELL 0x910e6ae0 6 synth Edt_Syn_kOscEnvAmt (-21, (-15, 0, 3))\n"
    assert rep.parse_cells(log) == {0: (21, (2, 0, 0)), 2: (-21, (-15, 0, 3))}


def test_rows_check_dispatch_and_dsp_word() -> None:
    table = csv_text(
        [
            ("wheel:64", "0x91097560", 4, "synth", "0x122", rep.NAME, "22.0000"),
            ("wheel:64", "dsp-all", 1, "ORG", "0xe02f4", "", word(22 / 60)),
            ("wheel:127", "0x91097560", 4, "synth", "0x122", rep.NAME, "23.0000"),
            ("wheel:127", "dsp-all", 1, "ORG", "0xe02f4", "", word(0.5)),
        ]
    )
    rows = rep.rows_of("FM Harpies", LOG, table)
    assert [r["law"] for r in rows] == ["22.0000", "23.0000"]
    assert rep.verdict(rows[0]) == (True, True)
    assert rep.verdict(rows[1]) == (True, False)


def test_main_writes_csv_and_counts(tmp_path: Path, capsys) -> None:
    (tmp_path / "X.log").write_text(LOG, encoding="utf-8")
    table = csv_text([("wheel:64", "0x91097560", 4, "synth", "0x122", rep.NAME, "22.0000")])
    (tmp_path / "X.csv").write_text(table, encoding="utf-8")
    out = tmp_path / "out.csv"
    assert rep.main([str(tmp_path), "--csv", str(out)]) == 0
    assert "dispatch = law 1" in capsys.readouterr().out
    assert out.read_text(encoding="utf-8").splitlines()[0].startswith("program,layer")
