#!/usr/bin/env python3
"""Multi-program ARM trace for the B3 Leslie gate (D155): who sends or loads `0x1256AD`?

For each program the worker boots the full machine with the program in the Store record, recalls
it (`--ticks`), then runs edit groups on every store that owns the edit id (layer stores of
`arm_edit_capture.find_stores` and FX units of `fx_clients.find_units`): the Leslie / rotary ids
(`Edt_OrgCom_*`, `Edt_Sys_kLeslie*`, `Fnc_kRotorSpeed`), the audio routing ids
(`Edt_*AudioRouting*`),
the amp type and the other FX slot types. Every DSP message of every phase (all chips, all cores,
path 1 and 2) is scanned for the literals of `LITERALS` (as value, as address, as code record),
L2 code stores are counted, and the bulk sender, the DSP image download and the SPI block download
are logged. Numbers and addresses only; the firmware stays local, the full message log goes to a
git-ignored gzip file.

    C=target/fw_unpacked
    uv run --with unicorn python tools/arm_leslie_paths.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
        --eeprom target/oled/eeprom.bin --out target/leslie_paths \
        "factory_programs/Program/Bank A/B3 Chords n Solo.ns4p"

Evidence: `docs/research/leslie-paths.md`.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter
from collections.abc import Iterable
from dataclasses import dataclass
from pathlib import Path

# VERIFIED (docs/research/leslie-gate.md): the B3 Leslie stage; its channel record is
# `0x28000000 + 4 * routine`. `0x1257B2` is the unmodelled raw synth filter type 3 stage
# (D121) and `0x1237F8` the Leslie cabinet routine B (D135); they are searched too.
LITERALS = {0x1256AD: "leslie", 0x1257B2: "stage_1257b2", 0x1237F8: "leslie_b"}
# VERIFIED (docs/research/leslie-locate.md s1 rows 4): the slot installer sends the B3 processor
# `0x121E2B` and the Amp module words `0x1250CF`, `0x125371`, `0x1235AA`, `0x1236F8`; they are the
# positive controls of the scan (they must be found, or the scan proves nothing).
CONTROLS = {0x121E2B: "b3_processor", 0x1250CF: "amp_first", 0x125371: "amp_drive",
            0x1235AA: "amp_cab_a", 0x1236F8: "amp_cab_b"}  # fmt: skip
CODE_RECORD_BASE = 0x28000000
# VERIFIED (arm_bulk_tables.py): routines the harness stubs or hooks.
BULK_SENDER, DSP_DOWNLOAD, SPI_DOWNLOAD_BLOCK = 0xC001C6A0, 0xC0241C58, 0xC0240548
HOOKED = {BULK_SENDER: "bulk_sender", DSP_DOWNLOAD: "dsp_image_download",
          SPI_DOWNLOAD_BLOCK: "spi_download_block"}  # fmt: skip
LESLIE_RX = r"Leslie|Rotary|RotorSpeed"
ROUTING_RX = r"AudioRouting"
TYPE_IDS = ("Edt_Effects_kAmpTypeNew", "Edt_Effects_kFx1Type", "Edt_Effects_kFx2Type",
            "Edt_Effects_kRvbType", "Edt_Effects_kDlyFbFxType")  # fmt: skip
LOAD_TICKS, IDLE_TICKS, EDIT_TICKS = 6000, 200, 40
# Type edits run on the FX units of these indices only (organ 3 and 9, one piano, one synth).
TYPE_UNITS = ("fx3organ", "fx9organ", "fx4piano", "fx0synth")


@dataclass(frozen=True)
class Edit:
    """One edit group: the id name, its id and the UI values to apply."""

    group: str
    name: str
    edit_id: int
    values: tuple[int, ...]


def literal_forms(routine: int) -> dict[int, str]:
    """Words that name `routine`: the plain address and its code-record byte address."""
    return {routine: "address", CODE_RECORD_BASE + 4 * routine: "code_record"}


def scan_words(words: Iterable[tuple[str, int]]) -> list[tuple[str, str, str]]:
    """Hits of `LITERALS` among `(field, word)` pairs: `(field, literal name, form)`."""
    forms = {
        w: (name, form) for r, name in LITERALS.items() for w, form in literal_forms(r).items()
    }
    return [(f, forms[w][0], forms[w][1]) for f, w in words if w in forms]


def scan_rows(rows: Iterable[dict], names: dict[int, str] | None = None) -> Counter:
    """Count `(name, form, phase group)` hits in capture rows (`arm_dsp_capture.parse_row`).

    The phase group is the part of the phase before `:` (`load`, `idle`, `edit`, `k60`...). Only the
    value word is scanned (the capture keeps the DM word, not the byte address).
    """
    names = {**LITERALS, **CONTROLS} if names is None else names
    forms = {w: (n, f) for r, n in names.items() for w, f in literal_forms(r).items()}
    hits: Counter = Counter()
    for row in rows:
        hit = forms.get(row["w2"])
        if hit:
            hits[(hit[0], hit[1], str(row["phase"]).split(":")[0])] += 1
    return hits


def scan_message(m) -> list[tuple[str, str, str]]:  # noqa: ANN001 - arm_emu.Message
    """Literal hits in one message: the byte address word and the value word."""
    return scan_words((("word1", m.word1), ("word2", m.word2)))


def is_l2_code_store(m) -> bool:  # noqa: ANN001
    """True for a path 2 store to a DSP L2 byte address (`0x20000000..0x28000000`)."""
    return m.path == 2 and 0x20000000 <= m.word1 < 0x28000000


def census(messages: Iterable) -> Counter:  # noqa: ANN001
    """Messages per `chip core path`."""
    return Counter(f"{m.chip}/c{m.core}/p{m.path}" for m in messages)


def value_set(lo: int, hi: int, dense: int = 12) -> tuple[int, ...]:
    """All values of a small range, else `lo`, quarter points and `hi`."""
    if hi - lo < dense:
        return tuple(range(lo, hi + 1))
    return tuple(sorted({lo, lo + (hi - lo) // 4, lo + (hi - lo) // 2, hi - (hi - lo) // 4, hi}))


def edit_plan(params: list) -> list[Edit]:  # noqa: ANN001 - arm_edit_capture.Param
    """Edit groups: Leslie / rotary ids, audio routing ids, amp and FX slot types."""
    plan = []
    for p in params:
        if re.search(ROUTING_RX, p.name):
            group = "routing"
        elif re.search(LESLIE_RX, p.name):
            group = "leslie"
        elif p.name in TYPE_IDS:
            group = "type"
        else:
            continue
        plan.append(Edit(group, p.name, p.edit_id, value_set(p.minimum, p.maximum)))
    return plan


def summary(hits: list[dict], counts: dict[str, Counter], l2: Counter, calls: Counter) -> dict:
    """Printable summary of one program run."""
    return {
        "hits": hits,
        "hit_count": len(hits),
        "messages": {k: dict(v) for k, v in counts.items()},
        "l2_stores": dict(l2),
        "hooked_calls": dict(calls),
    }


def run_program(  # pragma: no cover - needs the local firmware
    code: Path, sram: Path, eeprom: Path, program: Path, csv_path: Path, ticks: int, edit_ticks: int
) -> tuple[dict, list[str]]:
    """Boot, recall, run the edit groups; returns `(summary, message rows)`."""
    import arm_dsp_capture as cap
    import arm_edit_capture as aec
    import arm_keydown as kd
    import arm_program_load as apl
    import arm_screens as sc
    import fx_clients as fxc

    session = sc.Session(code, sram)
    session.load_eeproms(
        apl.build_image(eeprom.read_bytes(), program.read_bytes(), program.stem[:23])
    )
    emu = session.emu
    kd.install_board_present(emu)
    calls: Counter = Counter()
    phase = ["boot"]
    for pc, name in HOOKED.items():

        def hook(_uc, _a, _s, _u, name=name) -> None:  # noqa: ANN001
            calls[f"{phase[0]}:{name}"] += 1

        emu.uc.hook_add(emu._u.UC_HOOK_CODE, hook, begin=pc, end=pc)
    session.boot()
    kd.Rig(emu, kd.Uart2().install(emu))  # installs the UART2 MIDI model
    emu.install_dsp_ports()
    cap.install_dsp_ack(emu)
    rows: list[str] = []
    counts: dict[str, Counter] = {}
    l2: Counter = Counter()
    hits: list[dict] = []

    def record(label: str, tick: int, messages: list) -> None:
        counts.setdefault(label, Counter()).update(census(messages))
        for m in messages:
            rows.append(cap.format_row(label, tick, m))
            l2[label] += is_l2_code_store(m)
            hits.extend(
                {"phase": label, "tick": tick, "chip": m.chip, "core": m.core, "dm": hex(m.dm_word),
                 "w2": hex(m.word2), "pc": hex(m.pc), "field": f, "literal": name, "form": form}
                for f, name, form in scan_message(m)
            )  # fmt: skip

    def run(label: str, n: int, action=None) -> None:  # noqa: ANN001
        phase[0] = label
        import arm_steady as ast
        import arm_sy_voice as sv
        from arm_org_stream import TICK_INSNS

        for t in range(n):
            if t == 0 and action:
                action()
            emu.clear_messages()
            ast.run_ticking(emu, TICK_INSNS, per_tick=TICK_INSNS)
            sv.pump(emu)
            record(label, t, list(emu.messages))

    run("load", ticks)
    run("idle", IDLE_TICKS)
    params = aec.load_params(csv_path, r".")
    by_name = {p.name: p for p in params}
    markers = {n: by_name[n].edit_id for n in fxc.KIND_MARKERS if n in by_name}
    stores = [
        (f"{s.kind}{s.layer}", s.address) for s in aec.first_store_per_layer(aec.find_stores(emu))
    ]
    stores += [(f"fx{u.index}{u.kind}", u.address) for u in fxc.find_units(emu, markers)]
    owners: dict[str, list[str]] = {}
    for edit in edit_plan(params):
        owned = [(n, a) for n, a in stores if aec.value_cell(emu, a, edit.edit_id) is not None]
        owners[edit.name] = [n for n, _ in owned]
        mine = owned
        if edit.group == "routing":  # no store owns these ids: dispatch to every store
            mine = stores
        elif edit.group == "type":
            mine = [(n, a) for n, a in owned if n in TYPE_UNITS]
        for label, address in mine:
            for value in edit.values:
                word = aec.cell_word(by_name[edit.name], value)
                emu.clear_messages()
                aec.apply_value(emu, address, edit.edit_id, word, window=1)
                record(f"edit:{edit.name}={value}@{label}", 0, list(emu.messages))
                run(f"edit:{edit.name}={value}@{label}", edit_ticks)
    out = summary(hits, counts, l2, calls)
    out["owners"] = owners
    out["ticks"] = {"load": ticks, "idle": IDLE_TICKS, "edit": edit_ticks}
    return out, rows


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local firmware
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("image", type=Path)
    ap.add_argument("sram", type=Path)
    ap.add_argument("--eeprom", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--csv", type=Path, default=Path("docs/research/arm-edit-params.csv"))
    ap.add_argument("--ticks", type=int, default=LOAD_TICKS)
    ap.add_argument("--edit-ticks", type=int, default=EDIT_TICKS)
    ap.add_argument("program", type=Path)
    ns = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import arm_dsp_capture as cap

    result, rows = run_program(
        ns.image, ns.sram, ns.eeprom, ns.program, ns.csv, ns.ticks, ns.edit_ticks
    )
    name = cap.out_name(ns.program)
    ns.out.mkdir(parents=True, exist_ok=True)
    cap.write_rows(ns.out / f"{name}.txt.gz", rows)
    (ns.out / f"{name}.json").write_text(json.dumps(result, indent=1), encoding="utf-8")
    print(f"{ns.program.stem}: {len(rows)} messages, {result['hit_count']} literal hits")
    return 0


if __name__ == "__main__":
    sys.exit(main())
