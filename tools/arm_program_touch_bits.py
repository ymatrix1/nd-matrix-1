#!/usr/bin/env python3
"""Program recall to the piano Touch edit value for every factory program (D99 / D117 follow-up).

The decoder reads the per-layer Touch at file bit `layer base + 63` (`cbin/piano_preset.rs`). This
tool compares that file value with what the firmware itself holds after it recalls the program,
for every factory program (no poke, no single-bit flip). Per program it

1. puts the `.ns4p` into the Store record of the I2C EEPROM image (`arm_program_load.build_image`),
2. boots the full machine so the firmware recalls the program (`arm_screens.Session`),
3. reads `Edt_Pno_kKbdTouch` (`0x159`) of every piano layer store through the layer value table
   (`arm_edit_capture.value_cell`, same method as `arm_program_synth_velocity.py`), and
4. hooks the Touch handler store `0xC011AA38` (`[core + 0x1C] = curveId`, 10 / 11 / 13).

Sub-commands (a boot takes about 3 minutes of CPU, so `batch` runs programs in parallel):

    C=target/fw_unpacked
    python tools/arm_program_touch_bits.py recall $C/bank0_CODE.bin $C/bank0_SRAM.bin \
        --eeprom target/oled/eeprom.bin --ns4b "<backup>.ns4b" \
        --program "Program/Bank A/Pearl Upright.ns4p"
    python tools/arm_program_touch_bits.py batch $C/bank0_CODE.bin $C/bank0_SRAM.bin \
        --eeprom target/oled/eeprom.bin --ns4b "<backup>.ns4b" --workers 12 \
        --census docs/research/data/piano-touch-program.csv \
        --out docs/research/data/piano-touch-recall.json
    python tools/arm_program_touch_bits.py check docs/research/data/piano-touch-program.csv \
        docs/research/data/piano-touch-recall.json

`check` needs no firmware: it compares the committed recall results with the decoder census
(`cargo run --release -p nord-tools --bin piano_touch_census -- <backup> --csv`) and prints
`N of N`. Reads the local firmware only; output is derived numbers (never commit the firmware).
"""

from __future__ import annotations

import argparse
import csv
import json
import subprocess
import sys
import zipfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

# VERIFIED (`docs/research/arm-edit-params.csv`, `piano-touch-curves.md` s1): `Edt_Pno_kKbdTouch`.
TOUCH_EDIT_ID = 0x159
# VERIFIED (disassembly `0xC011AA24..0xC011AA38`, `piano-touch-curves.md` s1): the handler stores
# curveId = u32[0xC02BBF98 + 0x678 + 4 * value] (10, 11, 13) into `[core + 0x1C]`.
TOUCH_STORE_PC = 0xC011AA38
CURVE_IDS = (10, 11, 13)
LAYER_NAMES = ("A", "B")
RESULT_PREFIX = "RESULT "


def expected_curve_id(edit_value: int) -> int:
    """`curveId` the handler stores for the edit value `0 / 1 / 2`."""
    return CURVE_IDS[edit_value]


def parse_census(text: str) -> dict[str, dict]:
    """Census CSV rows to `{entry: {"A": touch, "B": touch, ...}}`.

    Columns: `entry,slot,name,touch_a,touch_b,layer_a_on,layer_b_on,section_on`.
    """
    out: dict[str, dict] = {}
    for row in csv.DictReader(text.splitlines()):
        out[row["entry"]] = {
            "A": int(row["touch_a"]),
            "B": int(row["touch_b"]),
            "in_use": row["layer_a_on"] == "1" or row["layer_b_on"] == "1",
        }
    return out


def parse_result_line(line: str) -> dict | None:
    """The JSON of a `RESULT {...}` line, or None for any other line."""
    if not line.startswith(RESULT_PREFIX):
        return None
    return json.loads(line[len(RESULT_PREFIX) :])


def compare_program(file_touch: dict, recall: dict) -> list[str]:
    """Mismatch lines for one program (empty when the recall equals the file bits).

    `recall` is `{"A": edit value, "B": edit value, "curve_ids": [curveId, ...]}`; the handler
    stores layer A first, then layer B.
    """
    problems = []
    for name in LAYER_NAMES:
        if recall.get(name) != file_touch[name]:
            problems.append(f"layer {name}: recalled {recall.get(name)} != file {file_touch[name]}")
    want = [expected_curve_id(file_touch[n]) for n in LAYER_NAMES]
    if recall.get("curve_ids", [])[:2] != want:
        problems.append(f"curveId {recall.get('curve_ids')} != {want}")
    return problems


def check(census: dict[str, dict], results: dict[str, dict]) -> tuple[int, list[str]]:
    """`(matching programs, problem lines)` over every census program."""
    matching, problems = 0, []
    for entry, file_touch in sorted(census.items()):
        if entry not in results:
            problems.append(f"{entry}: no recall result")
            continue
        bad = compare_program(file_touch, results[entry])
        if bad:
            problems.extend(f"{entry}: {p}" for p in bad)
        else:
            matching += 1
    problems.extend(
        f"{e}: recalled but not in the census" for e in sorted(set(results) - set(census))
    )
    return matching, problems


def summary(census: dict[str, dict], matching: int) -> str:
    """`N of N` line with the value mix of the checked layers."""
    mix = {0: 0, 1: 0, 2: 0}
    for t in census.values():
        mix[t["A"]] += 1
        mix[t["B"]] += 1
    return (
        f"{matching} of {len(census)} programs: recalled Touch equals the decoded file bits "
        f"(both layers; values heavy/medium/light over {2 * len(census)} layers: "
        f"{mix[0]}/{mix[1]}/{mix[2]})"
    )


def read_member(ns4b: Path, member: str) -> bytes:
    """Bytes of an archive member."""
    with zipfile.ZipFile(ns4b) as archive, archive.open(member) as handle:
        return handle.read()


def recall_one(ns: argparse.Namespace) -> dict:  # pragma: no cover - needs the local firmware
    """Boot with the program, return `{"A", "B", "curve_ids", "clients"}` as the firmware holds."""
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import arm_edit_capture as aec
    import arm_program_load as pg
    import arm_screens as sc
    import unicorn.arm_const as uc_arm

    program = read_member(ns.ns4b, ns.program)
    session = sc.Session(ns.image, ns.sram)
    emu = session.emu
    stores: list[tuple[int, int]] = []
    emu.uc.hook_add(
        emu._u.UC_HOOK_CODE,
        lambda uc, *_: stores.append(
            (uc.reg_read(uc_arm.UC_ARM_REG_R2), uc.reg_read(uc_arm.UC_ARM_REG_R3))
        ),
        begin=TOUCH_STORE_PC,
        end=TOUCH_STORE_PC,
    )
    session.load_eeproms(
        pg.build_image(ns.eeprom.read_bytes(), program, Path(ns.program).stem[:23])
    )
    session.boot()
    session.run(ns.ticks)
    # VERIFIED (run, D142 method): each layer client owns two stores with the same values; the
    # piano clients are listed in layer order (A, B).
    by_client: dict[int, set[int]] = {}
    for store in (s for s in aec.find_stores(emu) if s.kind == "piano"):
        cell = aec.value_cell(emu, store.address, TOUCH_EDIT_ID)
        by_client.setdefault(store.layer, set()).add(emu.read32(cell) if cell else -1)
    out: dict = {"clients": sorted(by_client), "curve_ids": [c for _, c in stores]}
    for name, client in zip(LAYER_NAMES, sorted(by_client)):
        vals = by_client[client]
        out[name] = next(iter(vals)) if len(vals) == 1 else sorted(vals)
    return out


def run_worker(ns: argparse.Namespace, entry: str) -> tuple[str, dict]:  # pragma: no cover
    """Recall one program in a child process (a boot is single threaded CPU)."""
    cmd = [
        sys.executable,
        str(Path(__file__).resolve()),
        "recall",
        str(ns.image),
        str(ns.sram),
        "--eeprom",
        str(ns.eeprom),
        "--ns4b",
        str(ns.ns4b),
        "--program",
        entry,
        "--ticks",
        str(ns.ticks),
    ]
    done = subprocess.run(cmd, capture_output=True, text=True, check=False)  # noqa: S603
    for line in done.stdout.splitlines():
        parsed = parse_result_line(line)
        if parsed is not None:
            return entry, parsed
    return entry, {"error": (done.stderr or done.stdout)[-300:]}


def cmd_batch(ns: argparse.Namespace) -> int:  # pragma: no cover - needs the local firmware
    census = parse_census(ns.census.read_text(encoding="utf-8"))
    results: dict[str, dict] = {}
    if ns.out.exists() and ns.resume:
        results = json.loads(ns.out.read_text(encoding="utf-8")).get("programs", {})
    todo = [e for e in sorted(census) if "error" in results.get(e, {"error": 1})]
    if ns.limit:
        todo = todo[: ns.limit]
    print(f"{len(todo)} programs to recall, {ns.workers} workers", flush=True)
    with ThreadPoolExecutor(max_workers=ns.workers) as pool:
        for entry, res in pool.map(lambda e: run_worker(ns, e), todo):
            results[entry] = res
            print(entry, res, flush=True)
            ns.out.write_text(
                json.dumps({"format": FORMAT_NOTE, "programs": results}, indent=1, sort_keys=True),
                encoding="utf-8",
            )
    matching, problems = check({e: census[e] for e in results if e in census}, results)
    for p in problems:
        print("MISMATCH", p)
    print(summary({e: census[e] for e in results if e in census}, matching))
    return int(bool(problems))


FORMAT_NOTE = (
    "per program: A / B = Edt_Pno_kKbdTouch (0x159) held by the firmware after it recalls the "
    "program (tools/arm_program_touch_bits.py, no poke); curve_ids = handler stores "
    "0xC011AA38 in order; derived numbers only"
)


def cmd_check(ns: argparse.Namespace) -> int:
    census = parse_census(ns.census.read_text(encoding="utf-8"))
    results = json.loads(ns.results.read_text(encoding="utf-8"))["programs"]
    matching, problems = check(census, results)
    for p in problems:
        print("MISMATCH", p)
    print(summary(census, matching))
    return int(bool(problems))


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("recall", "batch"):
        p = sub.add_parser(name)
        p.add_argument("image", type=Path)
        p.add_argument("sram", type=Path)
        p.add_argument("--eeprom", type=Path, required=True)
        p.add_argument("--ns4b", type=Path, required=True)
        p.add_argument("--ticks", type=int, default=300)
        if name == "recall":
            p.add_argument("--program", required=True, help="archive member of the .ns4p")
        else:
            p.add_argument("--census", type=Path, required=True)
            p.add_argument("--out", type=Path, required=True)
            p.add_argument("--workers", type=int, default=8)
            p.add_argument("--limit", type=int, default=0)
            p.add_argument("--resume", action="store_true", help="keep results already in --out")
    p = sub.add_parser("check")
    p.add_argument("census", type=Path)
    p.add_argument("results", type=Path)
    ns = ap.parse_args(argv)
    if ns.cmd == "recall":  # pragma: no cover - needs the local firmware
        print(RESULT_PREFIX + json.dumps(recall_one(ns)), flush=True)
        return 0
    if ns.cmd == "batch":  # pragma: no cover - needs the local firmware
        return cmd_batch(ns)
    return cmd_check(ns)


if __name__ == "__main__":
    sys.exit(main())
