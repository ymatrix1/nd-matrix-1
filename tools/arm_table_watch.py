#!/usr/bin/env python3
"""Watch ARM reads and writes of a RAM range while the firmware recalls a program.

Answers "which ARM code touches the channel row of routine X": hook `UC_HOOK_MEM_READ` and
`UC_HOOK_MEM_WRITE` over `[start, end)` during the same boot and recall that
`arm_dsp_capture.py` runs (`load` phase), then print one line per distinct
`(kind, pc, lr)` with a count. Numbers and addresses only; the firmware stays local.

    C=target/fw_unpacked
    uv run --with unicorn python tools/arm_table_watch.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
        --eeprom target/oled/eeprom.bin --range 0xC0545128:0xC0545148 \
        "factory_programs/Program/Bank A/B3 Chords n Solo.ns4p"

Evidence: `docs/research/leslie-locate.md` s3.
"""

from __future__ import annotations

import argparse
import sys
from collections import Counter
from pathlib import Path

LOAD_TICKS = 800


def parse_range(text: str) -> tuple[int, int]:
    """`START:END` (hex or decimal, END exclusive) to `(start, end)`."""
    first, _, last = text.partition(":")
    start, end = int(first, 0), int(last, 0)
    if end <= start:
        raise ValueError(f"empty range {text!r}")
    return start, end


def summarise(events: list[tuple[str, int, int, int]]) -> list[tuple[str, int, int, int, int]]:
    """Fold `(kind, pc, lr, address)` events to `(kind, pc, lr, first address, count)`, sorted."""
    counts: Counter[tuple[str, int, int]] = Counter()
    first: dict[tuple[str, int, int], int] = {}
    for kind, pc, lr, address in events:
        key = (kind, pc, lr)
        counts[key] += 1
        first.setdefault(key, address)
    return [(k, pc, lr, first[(k, pc, lr)], n) for (k, pc, lr), n in sorted(counts.items())]


def format_line(row: tuple[str, int, int, int, int]) -> str:
    kind, pc, lr, address, count = row
    return f"{kind:5s} pc {pc:#010x} lr {lr:#010x} first {address:#010x} x{count}"


def parse_edit(spec: str) -> tuple[int, int]:
    """`ID=VALUE` (decimal or `0x` hex) to `(id, value)`."""
    first, sep, last = spec.partition("=")
    if not sep:
        raise ValueError(f"edit {spec!r} is not ID=VALUE")
    return int(first, 0), int(last, 0)


def run_edit(emu, rig, spec: str, ticks: int, events: list, q16: bool = False) -> None:  # noqa: ANN001  # pragma: no cover
    """Dispatch one edit id on the organ layer, run `ticks` ticks, print what the DSPs saw."""
    import arm_edit_capture as aec
    import arm_steady as ast

    edit_id, value = parse_edit(spec)
    if q16:
        value <<= 16  # value cells of ranges of 12 steps or more are Q16.16 (arm_edit_capture)
    import fx_clients as fxc

    units = fxc.find_units(emu, {"Edt_OrgCom_kEffectsLeslieEnable": 0x189})
    owners = [u for u in units if u.kind == "organ"]
    print(f"edit {spec}: {len(units)} fx units, organ units {[u.index for u in owners]}")
    for u in owners:
        rows = fxc.module_rows(emu, u.address)
        print("  unit", u.index, "module rows", {f"{o:#x}": f"{r:#x}" for o, r in rows.items()})
    if not owners:
        return
    store = owners[0].address
    cell = aec.value_cell(emu, store, edit_id)
    if cell is not None:
        emu.write32(cell, value)
    emu.write32(aec.SCRATCH, value)
    events.clear()
    emu.clear_messages()
    ast.call_preserving(emu, aec.DISPATCH, store, 0, edit_id, aec.SCRATCH)
    called = [format_row(m) for m in emu.messages]
    import arm_dsp_capture as cap

    rows = cap.run_phase(rig, ticks, "edit")
    print(f"edit {spec}: cell {cell}, {len(called)} in the call, {len(rows)} in {ticks} ticks")
    for line in called + rows:
        print("  " + line)
    for row in summarise(events):
        print("  " + format_line(row))


def format_row(m) -> str:  # noqa: ANN001  # pragma: no cover
    return f"{m.chip} {m.path} {m.core} {m.dm_word:#x} {m.word2:#010x} {m.pc:#x}"


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local firmware
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("image", type=Path)
    ap.add_argument("sram", type=Path)
    ap.add_argument("--eeprom", type=Path, required=True)
    ap.add_argument("--range", dest="span", required=True, help="START:END, END exclusive")
    ap.add_argument("--ticks", type=int, default=LOAD_TICKS)
    ap.add_argument("--call", type=lambda t: int(t, 0), help="also log r0..r3 and lr at this pc")
    ap.add_argument(
        "--call-r0", type=lambda t: int(t, 0), help="only when [r0] (table pointer) equals this"
    )
    ap.add_argument(
        "--edit",
        action="append",
        default=[],
        help="ID=VALUE (hex ok): after the load phase set the edit id on the organ layer",
    )
    ap.add_argument("--q16", action="store_true", help="shift the --edit values left by 16")
    ap.add_argument("--edit-ticks", type=int, default=120)
    ap.add_argument("program", type=Path)
    ns = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import arm_dsp_capture as cap
    import arm_keydown as kd
    import arm_program_load as apl
    import arm_screens as sc

    start, end = parse_range(ns.span)
    session = sc.Session(ns.image, ns.sram)
    program = ns.program.read_bytes()
    session.load_eeproms(apl.build_image(ns.eeprom.read_bytes(), program, ns.program.stem[:23]))
    kd.install_board_present(session.emu)
    emu = session.emu
    events: list[tuple[str, int, int, int]] = []
    uc = emu.uc

    def watch(kind: str):  # noqa: ANN202
        def hook(_uc, _access, address, _size, _value, _user) -> None:  # noqa: ANN001
            events.append((kind, emu.reg(15), emu.reg(14), address))

        return hook

    uc.hook_add(emu._u.UC_HOOK_MEM_READ, watch("read"), begin=start, end=end - 1)
    uc.hook_add(emu._u.UC_HOOK_MEM_WRITE, watch("write"), begin=start, end=end - 1)
    if ns.call is not None:

        def at_call(_uc, _address, _size, _user) -> None:  # noqa: ANN001
            regs = [emu.reg(n) for n in (0, 1, 2, 3, 14)]
            if ns.call_r0 is None or emu.read32(regs[0]) == ns.call_r0:
                print("call " + " ".join(f"{r:#010x}" for r in regs))

        uc.hook_add(emu._u.UC_HOOK_CODE, at_call, begin=ns.call, end=ns.call)
    session.boot()
    print(f"boot: {len(events)} accesses")
    for row in summarise(events):
        print("  " + format_line(row))
    events.clear()
    rig = kd.Rig(emu, kd.Uart2().install(emu))
    emu.install_dsp_ports()
    cap.install_dsp_ack(emu)
    cap.run_phase(rig, ns.ticks, "load")
    for row in summarise(events):
        print(format_line(row))
    print(f"{len(events)} accesses in {ns.ticks} ticks")
    for spec in ns.edit:
        run_edit(emu, rig, spec, ns.edit_ticks, events, ns.q16)
    return 0


if __name__ == "__main__":
    sys.exit(main())
