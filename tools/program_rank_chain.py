#!/usr/bin/env python3
r"""Chain census of every factory program: one held key through the live ARM + SHARC link.

For each `.ns4p` the ARM firmware harness boots with the program recalled, key 60 (velocity 100) is
held 1,500 ticks and released (`tools/arm_dsp_live.py`, D138, D144, D153). Four chips run on the one
ARM stream: PNO core 1 (lane `2:0`, the organ / B3 lane), PNO core 2 and ORG core 2 (voice window
`0xC1F89`, the synth voice), SY (D153, path 1: sample voices). A second run with `--return-path`
(SAMPLE programs only) adds the SY audio peak (D150 / D154). Output: one JSON line per program in
`--out/chain.jsonl` (numbers only: levels, message and voice counts; no audio, firmware or sample
content). Finished programs are skipped on a rerun.

    uv run --with unicorn --with capstone python tools/program_rank_chain.py \
        --programs factory_programs/Program --out target/rank/chain --workers 8 \
        --fw-dir target/fw_unpacked --eeprom target/oled/eeprom.bin --backup B.ns4b \
        --pno1 target/PNO/core1_sram.bin --pno2 target/PNO/core2_sram.bin \
        --sy target/oracle/sy.bin --live target/release/sharc-live \
        --audio-for target/rank/sample_entries.txt
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

import program_rank

# Voice record, 8 words, stride 1 (D175): the lanes are interleaved (even words left, odd words
# right) and an arp step with pan R sounds on the odd lane only; the old window `0xC1F89:4:2`
# (D144) read the left lane.
WINDOW = "0xC1F89:8:1"
# EMPIRICAL (D164, `silent-programs.md`): the first arp step of Gravity Pattern reaches the DSP
# about 2,900 ARM ticks after the key, so an arp program is held far longer than the 1,500 ticks of
# the plain census. 15,000 ticks = 0.625 s = 1.25 beats at 120 bpm.
ARP_KEY_TICKS = 15_000
CHAINS = ("PNO1", "PNO2", "ORG2", "SY0")
SMP_FIELDS = ("kind", "peak_dbfs", "fundamental_cycles", "words")
SY_FIELDS = ("prepare", "go", "peak", "audio_ticks", "first_audio_tick", "voice0_playing_tick")


def entry_of(program: Path) -> str:
    """Backup entry name `Program/Bank A/Name.ns4p` of a program file."""
    return f"Program/{program.parent.name}/{program.stem}.ns4p"


def chips_for(engines: list[str], smp: bool = False) -> list[str]:
    """Chips a program needs: the tap of each engine in scene I (`none` and piano: PNO1 only).

    With `smp` (the `--smp` image is given) a piano program also runs the SMP chip: its post
    stage ring is the piano chain tap (D205).

    PNO core 1 carries the organ lane, PNO core 2 the synth voice window, SY the sample voices
    (ORG core 2 never receives a key word, D144, so it is not run). Fewer chips cost less CPU.
    """
    chips = []
    if "organ" in engines or not set(engines) & {"synth", "sample"}:
        chips.append("PNO1")
    if "synth" in engines or "sample" in engines:
        chips.append("PNO2")
    if "sample" in engines:
        chips.append("SY0")
    if smp and "piano" in engines:
        chips.append("SMP0")
    return chips


def chip_args(ns: argparse.Namespace, chips: list[str]) -> list[str]:
    """`--chip` arguments for the chip names."""
    spec = {
        "PNO1": f"PNO:1=pno={ns.pno1}",
        "PNO2": f"PNO:2=org={ns.pno2}={WINDOW}",
        "SY0": f"SY:0=sy={ns.sy}",
        "SMP0": f"SMP:0=smp={getattr(ns, 'smp', None)}",
    }
    return [a for c in chips for a in ("--chip", spec[c])]


def live_command(
    ns: argparse.Namespace,
    program: Path,
    out: Path,
    audio: bool,
    chips: list[str] | None = None,
    key_ticks: int | None = None,
) -> list[str]:
    """`arm_dsp_live.py` command line of one run (census, or with the SY return path)."""
    fw = ns.fw_dir
    cmd = [
        sys.executable, str(Path(__file__).resolve().with_name("arm_dsp_live.py")),
        str(fw / "bank0_CODE.bin"), str(fw / "bank0_SRAM.bin"), "--eeprom", str(ns.eeprom),
        "--program", str(program),
        "--live", str(ns.live), "--poll-advance", "64",
        "--ns4b", str(ns.backup), "--out", str(out),
    ]  # fmt: skip
    chips = chips or ["PNO1", "PNO2", "SY0"]
    cmd += chip_args(ns, chips)
    if "SY0" in chips:
        cmd.append("--sy-timeline")
    if "SMP0" in chips:  # the program's piano sound is served as the SMP flash (D191, D205)
        cmd += ["--piano-bind", str(ns.backup)]
    if key_ticks:
        cmd += ["--key-ticks", str(key_ticks)]
    return [*cmd, "--return-path"] if audio else cmd


def extract(result: dict) -> dict:
    """Compact numbers of one live run: per-chip levels and counts, SY voice timeline."""
    chips = {}
    for name in CHAINS:
        c = result.get("final", {}).get(name)
        if c is None:
            continue
        chips[name] = {
            "peak": c["lane_peak"], "rms": c["lane_rms"], "messages": c["messages_sent"],
            "passes": c["passes"], "halted": c["halted"], "errors": len(c["errors"]),
        }  # fmt: skip
    sy = result.get("sy_timeline") or {}
    tap = (result.get("smp_flash") or {}).get("tap") or {}
    return {
        "chips": chips,
        "smp": {k: tap.get(k) for k in SMP_FIELDS} if tap else None,
        "sy": {k: sy.get(k) for k in SY_FIELDS},
        "flagged_layers": len(result.get("flagged_layers") or []),
        "arm_messages": result.get("arm_messages", {}),
        "key_arm_messages": result.get("phases", {}).get("key", {}).get("arm_messages", {}),
        "ticks": result.get("ticks"),
    }


def read_records(path: Path) -> list[dict]:
    """Records of the output file (empty when it does not exist)."""
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text("utf-8").splitlines() if line]


def is_complete(rec: dict, need_audio: set[str]) -> bool:
    """Complete: the census ran and, for an `--audio-for` entry, the return-path run too."""
    return bool(rec.get("census")) and (rec["entry"] not in need_audio or bool(rec.get("audio")))


def finished(path: Path, need_audio: set[str] | None = None) -> set[str]:
    """Entries with a complete record in the output file (resume).

    A failed entry (`census` is `null`, or a missing return-path run of an `--audio-for` entry: the
    harness crashed or timed out) is not finished: a rerun runs it again (D170; the earlier code
    skipped it and kept the failure for good).
    """
    return {r["entry"] for r in read_records(path) if is_complete(r, need_audio or set())}


def drop_failed(path: Path, need_audio: set[str] | None = None) -> int:
    """Rewrites the output file without the incomplete records, so a retry leaves one line per
    entry; returns how many were dropped."""
    records = read_records(path)
    kept = [r for r in records if is_complete(r, need_audio or set())]
    if len(kept) != len(records):
        text = "".join(json.dumps(r, sort_keys=True) + chr(10) for r in kept)
        path.write_text(text, encoding="utf-8")
    return len(records) - len(kept)


def run_live(
    ns: argparse.Namespace, program: Path, audio: bool, chips: list[str], key_ticks: int | None
) -> dict | None:  # pragma: no cover
    """One live run in a temp file; `None` when the harness failed."""
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "run.json"
        r = subprocess.run(  # noqa: S603
            live_command(ns, program, out, audio, chips, key_ticks),
            capture_output=True,
            text=True,
            check=False,
        )
        if r.returncode != 0 or not out.exists():
            return None
        return extract(json.loads(out.read_text("utf-8")))


def record(
    ns: argparse.Namespace, program: Path, audio: bool, chips: list[str], key_ticks: int | None
) -> dict:  # pragma: no cover
    """Census record, plus the return-path run for the programs listed in `--audio-for`."""
    rec = {"entry": entry_of(program), "census": run_live(ns, program, False, chips, key_ticks)}
    rec["audio"] = run_live(ns, program, True, chips, key_ticks) if audio else None
    rec["chips"] = chips
    rec["key_ticks"] = key_ticks
    return rec


def audio_entries(path: Path) -> set[str]:
    """Backup entry names of `--audio-for` (one per line; CRLF and blank lines tolerated)."""
    return {line.strip() for line in path.read_text("utf-8").splitlines() if line.strip()}


def key_ticks_for(expect: dict | None) -> int | None:
    """Hold length of a program: `ARP_KEY_TICKS` when a scene I synth layer has its arpeggiator on
    (line of `tools/program_expect.py`), else the census default (`None`)."""
    return ARP_KEY_TICKS if expect and expect.get("arp") else None


def programs_of(root: Path, only: str | None, every: int, limit: int) -> list[Path]:
    """Program files in bank and name order, filtered."""
    files = sorted(root.glob("Bank */*.ns4p"))
    files = [f for f in files if only is None or only in f.name][::every]
    return files[:limit]


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - needs the local firmware
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--programs", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    for name in ("fw-dir", "eeprom", "backup", "pno1", "pno2", "sy", "live"):
        ap.add_argument(f"--{name}", type=Path, required=True)
    ap.add_argument("--smp", type=Path, help="SMP image: piano programs also run the SMP chip")
    ap.add_argument("--audio-for", type=Path, help="file with one backup entry name per line")
    ap.add_argument("--native", type=Path, required=True, help="program_rank lines (engines)")
    ap.add_argument("--expect", type=Path, help="program_expect.py lines (arp: longer hold)")
    ap.add_argument("--workers", type=int, default=6)
    ap.add_argument("--only")
    ap.add_argument("--entries", type=Path, help="run only the backup entries listed in this file")
    ap.add_argument("--every", type=int, default=1)
    ap.add_argument("--limit", type=int, default=10**9)
    ns = ap.parse_args(argv)
    ns.out.mkdir(parents=True, exist_ok=True)
    target = ns.out / "chain.jsonl"
    audio = audio_entries(ns.audio_for) if ns.audio_for else set()
    dropped = drop_failed(target, audio)
    if dropped:
        print(f"retrying {dropped} failed entries", flush=True)
    engine_of = {r["entry"]: program_rank.engines(r) for r in program_rank.load_jsonl(ns.native)}
    only_entries = audio_entries(ns.entries) if ns.entries else None
    expect = {r["entry"]: r for r in program_rank.load_jsonl(ns.expect)} if ns.expect else {}
    done = finished(target, audio)
    todo = [
        p for p in programs_of(ns.programs, ns.only, ns.every, ns.limit)
        if entry_of(p) not in done and (only_entries is None or entry_of(p) in only_entries)
    ]  # fmt: skip
    with ThreadPoolExecutor(ns.workers) as pool, target.open("a", encoding="utf-8") as fh:
        jobs = [
            pool.submit(
                record,
                ns,
                p,
                entry_of(p) in audio,
                chips_for(engine_of[entry_of(p)], ns.smp is not None),
                key_ticks_for(expect.get(entry_of(p))),
            )
            for p in todo
        ]
        for job in as_completed(jobs):
            rec = job.result()
            fh.write(json.dumps(rec, sort_keys=True) + "\n")
            fh.flush()
            print(rec["entry"], "ok" if rec["census"] else "FAILED", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
