#!/usr/bin/env python3
"""Morph law of the booted firmware: program recall, wheel / pedal / aftertouch (ns4-kq1o).

Puts a `.ns4p` into the Store record of the I2C EEPROM image (`arm_program_load.build_image`),
boots the full machine so the firmware recalls the program itself, and then sends MIDI on UART2:

* A held key `note:60` (`90 60 64`) for the organ and piano captures.
* Mod Wheel `B0 01 v` (`Fnc_MIDIWheel`), Control Pedal `B0 0B v` (`Fnc_MIDIExpr`), Aftertouch
  `D0 v` (`Fnc_MIDIAftertouch`).

Every morph-assigned edit id is re-dispatched through the layer client dispatch `0xC00CC4F0`
`(store, 0, id, &value)`; the hook logs the Q16.16 value the pointer holds, which is the morphed
edit value the setter receives. A scenario is a comma list of `source:value` events (`wheel:127`,
`wheel:127,at:127,pedal:127`), each sent from the settled snapshot, 100 ticks apart.

    uv run --with unicorn python tools/arm_program_morph.py target/fw_unpacked/bank0_CODE.bin \
        target/fw_unpacked/bank0_SRAM.bin --eeprom target/oled/eeprom.bin \
        --ns4b ".reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b" \
        --program "Program/Bank A/Horizon Whl.ns4p" --cache target/morph.state \
        --scenario wheel:64 wheel:127 --csv docs/research/data/morph-fw-horizon.csv

Findings: `docs/research/morph-fw-vs-native.md`. Only derived numbers are printed (never commit
the firmware or a program file).
"""

from __future__ import annotations

import argparse
import csv
import io
import pickle
import sys
import zipfile
from pathlib import Path

# VERIFIED (run, `docs/research/morph-fw-vs-native.md` s2): MIDI sources of the three morph
# controllers on UART2 channel 1 (status byte, data 1 for the controller change).
SOURCES = {"wheel": (0xB0, 1), "pedal": (0xB0, 11), "at": (0xD0, None)}
# A held key for the organ and piano captures (`note:60` is note-on channel 1, velocity 100).
NOTE_VELOCITY = 100
# VERIFIED (run): the value cells of a morph-capable edit id are `[base, wheel, at, pedal]`
# (`Edt_Syn_kFltFreq` of Horizon Whl layer A reads `[52, 10, 0, 0]`).
CELL_SOURCES = ("wheel", "at", "pedal")
# VERIFIED (disassembly + run, `arm_edit_capture.DISPATCH`): the layer client dispatch.
SETTLE_TICKS = 100
# VERIFIED (`arm_synth_freq_law.py`, `arm_synth_filter_probe.py`): word 1 of the ORG core 2 common
# block of synth layer A / B / C (`DM(0xE02E8 + 32 L + 1)`) is the cutoff word, top byte knob - 52.
WATCH_WORDS = {
    0xE02E9: "synth A cutoff word",
    0xE0309: "synth B cutoff word",
    0xE0329: "synth C cutoff word",
}
TICK_FIELDS = ("scenario", "store", "client", "kind", "id", "name", "value")
Q16 = 65536.0


def event_bytes(source: str, value: int) -> bytes:
    """UART2 bytes of one controller event (`source` in `SOURCES`, `value` 0..127)."""
    if not 0 <= value <= 127:
        raise ValueError(f"value {value} outside 0..127")
    if source == "note":
        return bytes([0x90, value, NOTE_VELOCITY])
    status, controller = SOURCES[source]
    if controller is None:
        return bytes([status, value])
    return bytes([status, controller, value])


def parse_scenario(text: str) -> list[tuple[str, int]]:
    """`"wheel:127,at:64"` to `[("wheel", 127), ("at", 64)]`."""
    events: list[tuple[str, int]] = []
    if text == "none":
        return events
    for item in text.split(","):
        source, _, value = item.partition(":")
        if (source not in SOURCES and source != "note") or not value.lstrip("-").isdigit():
            raise ValueError(
                f"bad event {item!r}, want SOURCE:VALUE with SOURCE in {list(SOURCES)}"
            )
        events.append((source, int(value)))
    return events


def fw_weight(value: int) -> float:
    """Firmware controller weight of a MIDI value: `v / 128`, full scale at 127 (VERIFIED run,
    `morph-fw-law.csv`: the piano A level of a +127 wheel delta reads `v * 127 / 128` for v up to
    126 and exactly 127.0 at v = 127)."""
    return 1.0 if value >= 127 else value / 128.0


def native_weight(value: int) -> float:
    """Weight the native engine uses (`MorphControllers`: MIDI value over 127)."""
    return value / 127.0


def signed32(word: int) -> int:
    """Two's complement of a 32-bit cell word."""
    return word - (1 << 32) if word & 0x80000000 else word


def morph_sum(
    base: float,
    deltas: tuple[float, float, float],
    weights: tuple[float, float, float],
    lo: float,
    hi: float,
) -> float:
    """`clamp(base + sum(delta_i * w_i), lo, hi)`: the combination the firmware applies
    (VERIFIED run: wheel 127 + aftertouch 127 + pedal 127 on 64 / +50 / +100 / -100 reads 114,
    the clamp acts on the sum, not per source)."""
    total = base + sum(d * w for d, w in zip(deltas, weights, strict=True))
    return max(lo, min(hi, total))


def cell_rows(cells: list[int]) -> tuple[int, tuple[int, int, int]]:
    """`(base, (wheel, at, pedal))` of the four words of a value cell array."""
    return signed32(cells[0]), (signed32(cells[1]), signed32(cells[2]), signed32(cells[3]))


def last_values(log: list[tuple[int, int, int | None]]) -> dict[tuple[int, int], float]:
    """Final Q16.16 value (as a number) per `(store, id)` of a dispatch log."""
    out: dict[tuple[int, int], float] = {}
    for store, edit_id, raw in log:
        if raw is not None:
            out[(store, edit_id)] = signed32(raw) / Q16
    return out


def rows_csv(rows: list[tuple]) -> str:
    """CSV text of `TICK_FIELDS` rows."""
    buf = io.StringIO()
    writer = csv.writer(buf, lineterminator="\n")
    writer.writerow(TICK_FIELDS)
    for row in rows:
        writer.writerow(row)
    return buf.getvalue()


def changed_rows(rows: list[tuple]) -> list[tuple]:
    """Rows whose value differs from the same `(store, id)` row of the `none` scenario (all rows
    of `none` itself are dropped; with no `none` scenario every row is kept)."""
    base = {(r[1], r[4]): r[6] for r in rows if r[0] == "none"}
    return [r for r in rows if r[0] != "none" and base.get((r[1], r[4])) != r[6]]


def read_member(ns4b: Path, member: str) -> bytes:
    """Bytes of an archive member."""
    with zipfile.ZipFile(ns4b) as archive, archive.open(member) as handle:
        return handle.read()


def id_names(csv_path: Path) -> dict[int, str]:
    """`{edit id: name}` of `docs/research/arm-edit-params.csv`."""
    with csv_path.open(encoding="utf-8") as fh:
        return {int(r["id"], 0): r["name"] for r in csv.DictReader(fh)}


def morph_ids(csv_path: Path) -> list[int]:
    """Edit ids with four value cells (the registry width column starts with `4;`)."""
    with csv_path.open(encoding="utf-8") as fh:
        return [int(r["id"], 0) for r in csv.DictReader(fh) if r["w70;w74;w78"].startswith("4;")]


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local firmware
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("image", type=Path)
    ap.add_argument("sram", type=Path)
    ap.add_argument("--eeprom", type=Path, required=True)
    src = ap.add_mutually_exclusive_group(required=True)
    src.add_argument("--program-file", type=Path, help="a .ns4p file")
    src.add_argument("--ns4b", type=Path, help="backup archive (with --program)")
    ap.add_argument("--program", help="archive member of the .ns4p (with --ns4b)")
    ap.add_argument("--cache", type=Path, help="booted state cache (skips the boot)")
    ap.add_argument("--scenario", nargs="+", default=["wheel:127"])
    ap.add_argument("--cells", action="store_true", help="print the base and delta cells")
    ap.add_argument(
        "--dsp-all",
        action="store_true",
        help="log the last word of every DSP message per (chip, core, DM word) of a scenario",
    )
    ap.add_argument("--settle", type=int, default=SETTLE_TICKS, help="ticks after each event")
    ap.add_argument(
        "--history",
        metavar="DM_WORD",
        help="print every dispatch of the ids named by --history-name and every DSP write of this "
        "DM word (hex), in order, per scenario (the CSV keeps only the last value)",
    )
    ap.add_argument("--history-name", default="Edt_Syn_kOscEnvAmt", help="edit name for --history")
    ap.add_argument("--changed-only", action="store_true", help="drop rows equal to `none`")
    ap.add_argument("--csv", type=Path)
    ns = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import arm_edit_capture as aec
    import arm_keydown as kd
    import arm_program_load as pg
    import arm_screens as sc
    import arm_steady as ast
    import unicorn.arm_const as uc_arm

    program = ns.program_file.read_bytes() if ns.program_file else read_member(ns.ns4b, ns.program)
    name = (ns.program_file.stem if ns.program_file else Path(ns.program).stem)[:23]
    params = Path(__file__).resolve().parent.parent / "docs/research/arm-edit-params.csv"
    names, ids = id_names(params), morph_ids(params)
    session = sc.Session(ns.image, ns.sram)
    emu = session.emu
    kd.install_board_present(emu)
    if ns.cache is not None and ns.cache.exists():
        session.load_state(pickle.loads(ns.cache.read_bytes()))  # noqa: S301 - our own cache
    else:
        session.load_eeproms(pg.build_image(ns.eeprom.read_bytes(), program, name))
        session.boot()
        session.run(300)
        if ns.cache is not None:
            ns.cache.write_bytes(pickle.dumps(session.state_bytes()))
    emu.install_dsp_ports()
    rig = kd.Rig(emu, kd.Uart2().install(emu))
    rig.tick(20)
    blob = ast.snapshot(emu)
    stores = {s.address: s for s in aec.find_stores(emu)}
    if ns.cells:
        for s in stores.values():
            for eid in ids:
                cell = aec.value_cell(emu, s.address, eid)
                if cell:
                    words = [emu.read32(cell + 4 * k) for k in range(4)]
                    if any(words[1:]):
                        print("CELL", hex(s.address), s.layer, s.kind, names[eid], cell_rows(words))
    log: list[tuple[int, int, int | None]] = []

    def hook(uc, _a, _s, _u) -> None:  # noqa: ANN001
        r3 = uc.reg_read(uc_arm.UC_ARM_REG_R3)
        try:
            val: int | None = int.from_bytes(uc.mem_read(r3, 4), "little")
        except Exception:  # noqa: BLE001 - unmapped pointer
            val = None
        log.append((uc.reg_read(uc_arm.UC_ARM_REG_R0), uc.reg_read(uc_arm.UC_ARM_REG_R2), val))

    emu.uc.hook_add(emu._u.UC_HOOK_CODE, hook, begin=aec.DISPATCH, end=aec.DISPATCH)
    rows: list[tuple] = []
    for text in ns.scenario:
        ast.restore(emu, blob)
        emu.install_dsp_ports()
        ast.enable_real_timer(emu)
        log.clear()
        words: dict[int, int] = {}
        every: dict[tuple[str, int, int], int] = {}
        for source, value in parse_scenario(text):
            rig.send(event_bytes(source, value))
            for _t, msg in rig.tick(ns.settle):
                if ns.history and msg.dm_word == int(ns.history, 16) and msg.chip == "ORG":
                    print("HISTORY dsp", text, _t, msg.core, hex(msg.dm_word), f"{msg.word2:#010x}")
                if msg.chip == "ORG" and msg.core == 2 and msg.dm_word in WATCH_WORDS:
                    words[msg.dm_word] = msg.word2
                if ns.dsp_all:
                    every[(msg.chip, msg.core, msg.dm_word)] = msg.word2
        if ns.history:
            for store, eid, raw in log:
                if names.get(eid) == ns.history_name and raw is not None:
                    print("HISTORY dispatch", text, hex(store), signed32(raw) / Q16)
        for dm_word, word in sorted(words.items()):
            rows.append(
                (text, "dsp", 2, "ORG", hex(dm_word), WATCH_WORDS[dm_word], f"{word:#010x}")
            )
        for (chip, core, dm_word), word in sorted(every.items()):
            rows.append((text, "dsp-all", core, chip, hex(dm_word), "", f"{word:#010x}"))
        for (store, eid), value in sorted(last_values(log).items()):
            st = stores.get(store)
            rows.append(
                (
                    text,
                    hex(store),
                    st.layer if st else -1,
                    st.kind if st else "?",
                    hex(eid),
                    names.get(eid, "?"),
                    f"{value:.4f}",
                )
            )
    if ns.changed_only:
        rows = changed_rows(rows)
    text = rows_csv(rows)
    if ns.csv:
        ns.csv.write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
