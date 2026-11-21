#!/usr/bin/env python3
r"""Synth envelope and cutoff trajectory: SHARC oracle against native `nord-core` (D195).

The ARM recall stream of a program carries **no** envelope, cutoff, resonance or filter envelope
amount words (`arm_dsp_capture.py`: ORG core 2 gets the oscillator stage, a few common words and
the osc / filter stage ids only), which is why `synth_e2e_oracle.py` could not compare envelopes
(D132). This tool puts them in: it takes the program's stored knobs from the native decoder
(`synth_env_native`), writes the words the ARM edit path writes for those knobs (VERIFIED laws of
`synth-knob-laws.md`: rate index = knob - 64 in the top byte of strip 0 words 26..31, cutoff
position in common word 1, filter envelope amount `knob / 127` in common word 13) at pass 0
of the recall stream, keys the note with the captured key words and taps per pass

* the three envelope chain levels of slot 0 (`F - 20` amp, `F - 8` filter, `F + 6` oscillator),
* the cutoff coefficient word `DM(0xC2925)` (`R`, `docs/research/synth-adr-and-strip-law.md` s4).

and reports, per chain, the effective rate index of the attack ramp and of the release, the
plateau level and the peak, next to the values the native strip chain (`strip_path.rs`) has, and
the cutoff coefficient at the key-on, peak, sustained and released points.

    cargo build --release -p nord-sharc --bin sharc-sys -p nord-tools --bin synth_env_native
    uv run python tools/ns4_dsp_ldr.py target/DSP/dsp_image_2_ORG.bin --sram DIR
    uv run --with unicorn python tools/arm_dsp_capture.py one ... PROGRAM.ns4p   # capture file
    uv run python tools/synth_env_trace.py report CAPTURE.txt.gz PROGRAM.ns4p \
        --sharc-sys target/release/sharc-sys --image DIR/core2_sram.bin \
        --native target/release/synth_env_native --workdir target/synth_env
"""

from __future__ import annotations

import argparse
import csv
import json
import subprocess
import sys
from pathlib import Path

import numpy as np

# VERIFIED (synth-adr-and-strip-law s2): slot 0 envelope frame `F`, chain states F-20, F-8, F+6.
ENV_FRAME = 0xC1188
ENV_TAPS = (ENV_FRAME - 20, ENV_FRAME - 8, ENV_FRAME + 6)
CHAIN_NAMES = ("amp", "filter", "osc")
# VERIFIED (synth-adr-and-strip-law s4): cutoff coefficient of channel 0.
R_TAP = 0xC2925
# VERIFIED (synth-voice-control s3): strip 0 word `w` is `DM(0xE0000 + w)`; ADR words 26..34.
ADR_BASE = 0xE001A
COMMON_CUTOFF = 0xE02E9  # common block 0 word 1 (kFltFreq)
COMMON_FLT_ENV_AMOUNT = 0xE02F5  # common block 0 word 13 (kFltEnvAmt, `knob / 127`)
CUTOFF_LINEAR_LAST = 117
# Same table as `fw_knob_laws.rs::CUTOFF_TOP` (knobs 118..127).
CUTOFF_TOP = (
    0x42B33300, 0x44666680, 0x46199980, 0x47CCCD00, 0x49800000,
    0x4B333300, 0x4CE66680, 0x4E999980, 0x504CCD00, 0x6A000000,
)  # fmt: skip
RATE_INDEX_MAX = 63
# ASSUMED (D121): a pass is 2 SY frames at 43,402.34 Hz.
SY_FRAME_RATE_HZ = 43_402.34375
PASS_S = 2.0 / SY_FRAME_RATE_HZ
# The oracle filter stage runs 4 samples per pass (2 SY frames): 2 x the SY frame rate. The native
# strip chain takes `R = tan(pi fc / F)` at the SY frame rate itself (fw_cutoff_cycles_per_sample).
ORACLE_STAGE_RATE_HZ = 2.0 * SY_FRAME_RATE_HZ
KEY_ON_PASS = 2000  # the strip words settle with tau 256 passes (s4 of synth-adr-and-strip-law)
HOLD_S, TAIL_S = 0.5, 0.25
STRIP_WORD1 = 0xE0001
STAGE_LADDER_ID = 0x12549D  # VERIFIED (synth-filter-type-map s2): Ladder24 reads the ramp table
KNOB_HZ0 = 13.75  # VERIFIED (D200): cutoff knob 0 is A0 / 2
RATE_TABLE = Path(__file__).resolve().parent.parent / "docs/research/data/synth-env-rate-table.csv"


def rate_word(knob: int) -> int:
    """Strip word the ARM writes for a rate knob: index `knob - 64` in the top byte."""
    idx = min(max(int(knob), 0), 127) - 64
    return (min(idx, RATE_INDEX_MAX) << 24) & 0xFFFFFFFF


def cutoff_word(knob: int) -> int:
    """Common word 1 the ARM writes for a `kFltFreq` knob (fw_knob_laws.rs)."""
    k = min(max(int(knob), 0), 127)
    if k <= CUTOFF_LINEAR_LAST:
        return ((k - 52) << 24) & 0xFFFFFFFF
    return CUTOFF_TOP[k - CUTOFF_LINEAR_LAST - 1]


def f32_bits(x: float) -> int:
    return int(np.float32(x).view(np.uint32))


def program_words(knobs: dict, cutoff_knob: int, env_amount_knob: int) -> dict[int, int]:
    """`{DM address: word}` the edit path would write for a program's layer A knobs.

    `knobs` is the `synth_env_native` object: `amp` and `filter` as `[attack, decay, release]`.
    The oscillator ADR words keep the image value (the decoder has no osc envelope knobs).
    """
    words: dict[int, int] = {}
    for i, v in enumerate(list(knobs["amp"]) + list(knobs["filter"])):
        words[ADR_BASE + i] = rate_word(v)
    words[COMMON_CUTOFF] = cutoff_word(cutoff_knob)
    words[COMMON_FLT_ENV_AMOUNT] = f32_bits(int(env_amount_knob) / 127.0)
    return words


def inject_script(words: dict[int, int], at_pass: int = 0) -> str:
    """Flat `pass DM VALUE` lines for `sharc-sys --script-at`."""
    return "".join(f"{at_pass} {a:#x} {v & 0xFFFFFFFF:#010x}\n" for a, v in sorted(words.items()))


def load_rate_table(path: Path = RATE_TABLE) -> tuple[dict[int, float], dict[int, float]]:
    """Attack coefficient and decay / release coefficient per rate index (oracle sweep)."""
    att: dict[int, float] = {}
    dec: dict[int, float] = {}
    with Path(path).open(encoding="utf-8") as f:
        for r in csv.DictReader(f):
            att[int(r["index"])] = float(r["attack_coeff"])
            dec[int(r["index"])] = float(r["decay_release_coeff"])
    return att, dec


def attack_index(slope: float, table: dict[int, float]) -> int | None:
    """Rate index whose attack coefficient is nearest (log scale) to a measured ramp slope.

    The table repeats a value over plateaus; the nearest entry is the first of the smallest
    distance, so the result is exact only to the plateau (`+-3` indexes).
    """
    if slope <= 0:
        return None
    return min(table, key=lambda i: (abs(np.log(table[i]) - np.log(slope)), i))


def release_index(ratio: float, table: dict[int, float]) -> int | None:
    """Rate index whose decay coefficient `a` has `1 - a` nearest to the measured `1 - ratio`."""
    if not 0.0 < ratio < 1.0:
        return None
    return min(table, key=lambda i: (abs(np.log(1 - table[i]) - np.log(1 - ratio)), i))


def chain_facts(y: np.ndarray, on: int, off: int, att: dict, dec: dict) -> dict:
    """Facts of one chain trace `y` (one value per pass): key-on at `on`, key-off at `off`."""
    rise = next((p for p in range(on - 1, off) if y[p] > 0), None)
    out: dict = {"first_rise_pass": rise}
    if rise is not None and rise + 4 < off:
        slope = float((y[rise + 3] - y[rise + 1]) / 2)
        out["attack_slope"] = slope
        out["attack_index"] = attack_index(slope, att)
    out["peak"] = float(y[on:off].max()) if off > on else 0.0
    out["level_at_off"] = float(y[off - 1])
    tail = y[off + 10 : off + 60]
    tail = tail[tail > 1e-6]
    ratio = (tail[-1] / tail[0]) ** (1 / (len(tail) - 1)) if len(tail) > 10 else None
    out["release_ratio"] = float(ratio) if ratio else None
    out["release_index"] = release_index(ratio, dec) if ratio else None
    return out


def r_to_hz(r: float, rate_hz: float) -> float:
    """Cutoff in Hz of a cutoff coefficient `R = tan(pi fc / rate)` (`fc = rate atan(R) / pi`)."""
    return float(rate_hz * np.arctan(r) / np.pi)


def r_points(r: np.ndarray, on: int, off: int, rate_hz: float) -> dict:
    """Cutoff `R` (and Hz at `rate_hz`) before the key, at its maximum, end of hold and released."""
    out: dict = {"max_pass": int(on + np.argmax(r[on:off]))}
    for key, val in (
        ("before", r[on - 1]),
        ("max", r[on:off].max()),
        ("end_of_hold", r[off - 1]),
        ("released", r[-1]),
    ):
        out[key] = float(val)
        out[f"{key}_hz"] = r_to_hz(float(val), rate_hz)
    return out


def knob_of_hz(hz: float) -> float:
    """Cutoff index in knob semitones of a cutoff in Hz (`13.75 Hz 2^(knob / 12)`)."""
    return float(12.0 * np.log2(max(hz, 1e-3) / KNOB_HZ0))


def knob_of_r(r: float, ladder: bool) -> float:
    """Cutoff index of an oracle tap `R`: exponential law, or the linear ramp for the ladder stage.

    VERIFIED (`synth-cutoff-terms-oracle.csv`, D207): every stage but the ladder maps the index
    sum with `fc = 2F atan(R) / pi`; the ladder reads the ramp `0.36083 + 0.0079301 (knob - 52)`.
    """
    if ladder:
        return 52.0 + (r - 0.36083) / 0.0079301
    return knob_of_hz(r_to_hz(r, ORACLE_STAGE_RATE_HZ))


def stage_word(rows: list[dict]) -> int | None:
    """Last strip 0 word 1 (stage id) of the core 2 recall, or None."""
    last = None
    for r in rows:
        if r["phase"] in ("load", "idle") and r["chip"] == "ORG" and r["core"] == 2:
            if r["dm"] == STRIP_WORD1:
                last = r["w2"]
    return last


def key_note_offset(key_script: str) -> int:
    """Signed note offset (byte 2 of the first note-on word, `note - 60` as the ARM sends it)."""
    for line in key_script.splitlines():
        v = int(line.split()[2], 0)
        if (v >> 24) & 0x0F == 0x0B:
            b = (v >> 16) & 0xFF
            return b - 256 if b > 127 else b
    return 0


def knob_points(k: np.ndarray, on: int, off: int) -> dict:
    """Index at the key-on, its maximum, the end of the hold and the end of the tail."""
    return {
        "before": float(k[on - 1]),
        "max": float(k[on:off].max()),
        "end_of_hold": float(k[off - 1]),
        "released": float(k[-1]),
    }


def compare_native(oracle: dict, native: dict) -> dict:
    """Difference rows oracle minus native for one chain (`attack_index`, `level_at_off`, ...)."""
    row = {}
    for k in ("attack_index", "release_index", "level_at_off", "peak"):
        a, b = oracle.get(k), native.get(k)
        row[k] = None if a is None or b is None else a - b
    return row


def key_byte_script(script: str, g: int) -> str:
    """Key-on words of a key script with the velocity byte (`0x80 + g`, bits 8..15) replaced.

    VERIFIED (synth-velocity s1): the ARM key word carries `0x80 + g` in byte 1, `g` the velocity
    S-curve value 0..127. Only words with bit 27 set (the note-on word, `0x0B......`) change.
    """
    out = []
    for line in script.splitlines():
        p, dm, val = line.split()
        v = int(val, 0)
        if (v >> 24) & 0x0F == 0x0B:
            v = (v & 0xFFFF00FF) | ((0x80 + g) << 8)
        out.append(f"{p} {dm} {v:#010x}")
    return chr(10).join(out) + chr(10)


def effective_index(coeff: float, table: dict[int, float], rising: bool) -> float:
    """Continuous rate index of a measured coefficient: log-linear between table entries.

    `rising` is True for the attack table (coefficient falls with the index) and for the
    decay / release table `1 - a` falls too; both are monotone non-increasing, so the plateaus
    resolve to the middle of the plateau.
    """
    key = (lambda i: table[i]) if rising else (lambda i: 1.0 - table[i])
    ids = sorted(table)
    target = np.log(coeff if rising else 1.0 - coeff)
    xs = np.array([np.log(key(i)) for i in ids])
    hit = [i for i, x in zip(ids, xs, strict=True) if abs(x - target) < 1e-3]
    if hit:
        return float(np.mean(hit))
    return float(np.interp(target, xs[::-1], np.array(ids, dtype=float)[::-1]))


def run_oracle(
    sharc_sys: str, image: str, scripts: list[Path], passes: int, out: Path
) -> tuple[np.ndarray, np.ndarray]:
    """One `sharc-sys` run: chain levels `[passes, 3]` and `R` `[passes]`."""
    cmd = [sharc_sys, image, "--passes", str(passes), "--warmup", "0", "--epdma"]
    cmd += ["--script-flat", str(scripts[0])]
    for s in scripts[1:]:
        cmd += ["--script-at", f"0:{s}"]
    for a in (*ENV_TAPS, R_TAP):
        cmd += ["--tap", f"{a:#x}:1"]
    cmd += ["--tap-out", str(out)]
    r = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if "tap:" not in r.stdout:
        raise RuntimeError(r.stdout[-800:] + r.stderr[-400:])
    f = np.fromfile(out, dtype="<u4").reshape(-1, len(ENV_TAPS) + 1).view("<f4")
    f = f.astype(np.float64)
    return f[:, : len(ENV_TAPS)], f[:, len(ENV_TAPS)]


def run_native(
    native: str, program: str, on: int, off: int, passes: int, note_offset: int = 0
) -> dict:
    r = subprocess.run(
        [native, program, "--on", str(on), "--off", str(off), "--passes", str(passes)]
        + ["--note-offset", str(note_offset), "--kb-track", "3"],  # oracle: unity tracking
        capture_output=True,
        text=True,
        check=False,
    )
    if r.returncode != 0:
        raise RuntimeError(r.stderr[-400:])
    return json.loads(r.stdout.strip().splitlines()[-1])


def report(ns: argparse.Namespace) -> int:  # pragma: no cover - needs local firmware images
    import arm_dsp_capture as cap
    import synth_e2e_oracle as o

    rows = list(cap.read_rows(Path(ns.capture)))
    work = Path(ns.workdir)
    work.mkdir(parents=True, exist_ok=True)
    on = KEY_ON_PASS
    off = on + int(HOLD_S / PASS_S)
    passes = off + int(TAIL_S / PASS_S)
    key_text = o.key_script(rows, on, off)
    offset = key_note_offset(key_text)
    nat = run_native(ns.native, ns.program, on, off, passes, offset)
    words = program_words(nat["knobs"], nat["cutoff_knob"], int(nat["filter_env_amount_knob"]))
    (work / "load.script").write_text(o.core2_load_script(rows))
    (work / "adr.script").write_text(inject_script(words))
    (work / "key.script").write_text(o.key_script(rows, on, off))
    env, r = run_oracle(
        ns.sharc_sys,
        ns.image,
        [work / "load.script", work / "adr.script", work / "key.script"],
        passes,
        work / "trace.bin",
    )
    att, dec = load_rate_table()
    out = {"program": Path(ns.program).stem, "knobs": nat["knobs"], "passes": passes, "chains": {}}
    native_y = {"amp": np.array(nat["amp"]), "filter": np.array(nat["filter"])}
    for k, name in enumerate(CHAIN_NAMES):
        of = chain_facts(env[:, k], on, off, att, dec)
        entry = {"oracle": of}
        if name in native_y:
            nf = chain_facts(native_y[name], on, off, att, dec)
            entry["native"] = nf
            entry["diff"] = compare_native(of, nf)
        out["chains"][name] = entry
    out["cutoff"] = {
        "cutoff_knob": nat["cutoff_knob"],
        "filter_env_amount_knob": nat["filter_env_amount_knob"],
        "oracle_r": r_points(r, on, off, ORACLE_STAGE_RATE_HZ),
        "native_r": r_points(np.array(nat["r"]), on, off, SY_FRAME_RATE_HZ),
    }
    w1 = stage_word(rows)
    ladder = w1 is not None and (w1 & 0xFFFFFF) == STAGE_LADDER_ID
    kn = np.array([knob_of_r(x, ladder) for x in r])
    out["cutoff_index"] = {
        "stage_word1": w1,
        "ladder": ladder,
        "note_offset": offset,
        "oracle": knob_points(kn, on, off),
        "native_default": knob_points(
            np.array([knob_of_hz(h) for h in nat["hz_default"]]), on, off
        ),
        "native_fw_law": knob_points(np.array([knob_of_hz(h) for h in nat["hz_fw"]]), on, off),
    }
    np.save(work / "oracle_env.npy", env)
    np.save(work / "oracle_r.npy", r)
    print(json.dumps(out, indent=1))
    return 0


def sweep(ns: argparse.Namespace) -> int:  # pragma: no cover - needs local firmware images
    """Uniform ADR word sweep per key velocity byte: effective attack / release index per chain."""
    import arm_dsp_capture as cap
    import synth_e2e_oracle as o

    rows = list(cap.read_rows(Path(ns.capture)))
    work = Path(ns.workdir)
    work.mkdir(parents=True, exist_ok=True)
    on, off = 300, 300 + 2600
    passes = off + 400
    att, dec = load_rate_table()
    (work / "load.script").write_text(o.core2_load_script(rows))
    base_key = o.key_script(rows, on, off)
    out = []
    for g in (int(x) for x in ns.gs.split(",")):
        (work / "key.script").write_text(key_byte_script(base_key, g))
        for v in (int(x) for x in ns.words.split(",")):
            words = {ADR_BASE + i: (v << 24) & 0xFFFFFFFF for i in range(9)}
            words[COMMON_CUTOFF] = cutoff_word(100)
            (work / "adr.script").write_text(inject_script(words))
            env, _ = run_oracle(
                ns.sharc_sys,
                ns.image,
                [work / "load.script", work / "adr.script", work / "key.script"],
                passes,
                work / "sweep.bin",
            )
            for k, name in enumerate(CHAIN_NAMES):
                f = chain_facts(env[:, k], on, off, att, dec)
                row = {"g": g, "word": v, "chain": name}
                if f.get("attack_slope"):
                    row["attack_coeff"] = f["attack_slope"]
                    row["attack_index"] = effective_index(f["attack_slope"], att, True)
                if f["release_ratio"]:
                    row["release_ratio"] = f["release_ratio"]
                    row["release_index"] = effective_index(f["release_ratio"], dec, False)
                out.append(row)
            print(g, v, file=sys.stderr, flush=True)
    Path(ns.out).write_text(json.dumps(out, indent=1), encoding="utf-8")
    return 0


CSV_FIELDS = (
    "program", "chain", "attack_knob", "decay_knob", "release_knob",
    "attack_index_word", "attack_index_oracle", "attack_index_native",
    "release_index_word", "release_index_oracle", "release_index_native",
    "peak_oracle", "level_at_off_oracle", "level_at_off_native",
    "release_ratio_oracle", "release_ratio_native",
)  # fmt: skip
CUTOFF_FIELDS = (
    "program", "cutoff_knob", "filter_env_amount_knob",
    "hz_before_oracle", "hz_before_native", "hz_max_oracle", "hz_max_native",
    "hz_end_of_hold_oracle", "hz_end_of_hold_native", "hz_released_oracle", "hz_released_native",
)  # fmt: skip


def csv_rows(rep: dict) -> tuple[list[dict], dict]:
    """Chain rows (amp, filter) and the cutoff row of one `report` JSON document."""
    rows = []
    for name in ("amp", "filter"):
        ent = rep["chains"][name]
        o, n = ent["oracle"], ent["native"]
        a, d, r = rep["knobs"][name]
        rows.append(
            {
                "program": rep["program"], "chain": name,
                "attack_knob": a, "decay_knob": d, "release_knob": r,
                "attack_index_word": a - 64, "attack_index_oracle": o.get("attack_index"),
                "attack_index_native": n.get("attack_index"),
                "release_index_word": r - 64, "release_index_oracle": o.get("release_index"),
                "release_index_native": n.get("release_index"),
                "peak_oracle": round(o["peak"], 4),
                "level_at_off_oracle": round(o["level_at_off"], 4),
                "level_at_off_native": round(n["level_at_off"], 4),
                "release_ratio_oracle": o.get("release_ratio"),
                "release_ratio_native": n.get("release_ratio"),
            }
        )  # fmt: skip
    c = rep["cutoff"]
    cut = {"program": rep["program"], "cutoff_knob": c["cutoff_knob"],
           "filter_env_amount_knob": c["filter_env_amount_knob"]}  # fmt: skip
    for key in ("before", "max", "end_of_hold", "released"):
        cut[f"hz_{key}_oracle"] = round(c["oracle_r"][f"{key}_hz"], 1)
        cut[f"hz_{key}_native"] = round(c["native_r"][f"{key}_hz"], 1)
    return rows, cut


INDEX_FIELDS = (
    "program", "stage", "note_offset", "point", "oracle", "native_default", "native_fw_law",
    "err_default", "err_fw_law",
)  # fmt: skip


def index_rows(rep: dict) -> list[dict]:
    """Per-program cutoff index rows (knob semitones): oracle, default law, firmware law."""
    c = rep["cutoff_index"]
    rows = []
    for pt in ("before", "max", "end_of_hold", "released"):
        o, d, f = c["oracle"][pt], c["native_default"][pt], c["native_fw_law"][pt]
        rows.append(
            {
                "program": rep["program"], "stage": "ladder" if c["ladder"] else "other",
                "note_offset": c["note_offset"], "point": pt, "oracle": round(o, 2),
                "native_default": round(d, 2), "native_fw_law": round(f, 2),
                "err_default": round(d - o, 2), "err_fw_law": round(f - o, 2),
            }
        )  # fmt: skip
    return rows


def write_index_csv(reps: list[dict]) -> str:
    """CSV text of `index_rows` for several reports."""
    import io

    buf = io.StringIO()
    w = csv.DictWriter(buf, fieldnames=INDEX_FIELDS, lineterminator=chr(10))
    w.writeheader()
    for rep in reps:
        w.writerows(index_rows(rep))
    return buf.getvalue()


def summarise(ns: argparse.Namespace) -> int:  # pragma: no cover - CLI wiring
    """Two CSV blocks (chains, cutoff) of several `report` JSON files on stdout."""
    reps = [json.loads(Path(p).read_text(encoding="utf-8")) for p in ns.reports]
    if ns.index:
        sys.stdout.write(write_index_csv(reps))
        return 0
    chains, cuts = [], []
    for rep in reps:
        a, b = csv_rows(rep)
        chains += a
        cuts.append(b)
    w = csv.DictWriter(sys.stdout, fieldnames=CSV_FIELDS, lineterminator=chr(10))
    w.writeheader()
    w.writerows(chains)
    sys.stdout.write(chr(10))
    w2 = csv.DictWriter(sys.stdout, fieldnames=CUTOFF_FIELDS, lineterminator=chr(10))
    w2.writeheader()
    w2.writerows(cuts)
    return 0


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - CLI wiring
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("report")
    p.add_argument("capture")
    p.add_argument("program")
    p.add_argument("--sharc-sys", required=True)
    p.add_argument("--image", required=True)
    p.add_argument("--native", required=True)
    p.add_argument("--workdir", required=True)
    sw = sub.add_parser("sweep")
    sw.add_argument("capture")
    sw.add_argument("--sharc-sys", required=True)
    sw.add_argument("--image", required=True)
    sw.add_argument("--workdir", required=True)
    sw.add_argument("--out", required=True)
    sw.add_argument("--gs", default="10,32,64,98,127")
    sw.add_argument("--words", default="-60,-40,-20,0,20,40")
    q = sub.add_parser("csv")
    q.add_argument("reports", nargs="+", help="report JSON files")
    q.add_argument("--index", action="store_true", help="cutoff index rows (D207) instead")
    ns = ap.parse_args(argv)
    if ns.cmd == "csv":
        return summarise(ns)
    return sweep(ns) if ns.cmd == "sweep" else report(ns)


if __name__ == "__main__":
    sys.exit(main())
