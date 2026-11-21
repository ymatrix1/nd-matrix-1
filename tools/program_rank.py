#!/usr/bin/env python3
"""Join the native renders and the chain census of the 384 factory programs, score, rank, tabulate.

Inputs (numbers only, produced by local tools, see `docs/research/program-ranking.md` s2):
  `program_rank` (Rust, `crates/nord-tools`)  -> native.jsonl : level, centroid, 8 band shares per
      program for the engine defaults and for the firmware-derived options, key 60 velocity 100;
  `tools/program_rank_chain.py`               -> chain.jsonl  : the live ARM + SHARC link census.

    python tools/program_rank.py --native native.jsonl --chain chain.jsonl \
        --csv docs/research/data/program-ranking.csv --md-tables

Score (ASSUMED weights, tool convention): `|level delta dB| + RMS of the band share deltas dB
+ 6 x |centroid delta in octaves|`, default against firmware-derived render. It says how far the
default native path is from the oracle-verified firmware paths, not how far it is from the
hardware. The chain columns are evidence of whether the firmware chain voices the program at all.
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import statistics
from pathlib import Path

BANDS = 8
BAND_RELEVANT_DB = -50.0  # a band counts when either render holds more than this share
CENTROID_DB_PER_OCTAVE = 6.0  # ASSUMED weight of one octave of centroid shift, in score dB
SILENT_DB = -120.0  # rms below this counts as silent
MISMATCH_SCORE = 99.0  # one render silent, the other not

NL = chr(10)
ENGINE_ORDER = ("organ", "piano", "synth", "sample")


def load_jsonl(path: Path) -> list[dict]:
    """Rows of a JSON lines file."""
    return [json.loads(x) for x in path.read_text("utf-8").splitlines() if x.strip()]


def engines(row: dict) -> list[str]:
    """Engines present in scene I (`sample` replaces `synth` when a synth layer is SAMPLES)."""
    s1, lay = row["scene_1"], row["layers"]
    out = [e for e in ("organ", "piano") if s1[e]]
    if s1["synth"]:
        out.append("sample" if lay["synth_sample"] > 0 else "synth")
    return out or ["none"]


def distance(a: dict, b: dict) -> dict:
    """Level / centroid / band deltas of render `b` (firmware) against `a` (default)."""
    if "panic" in a or "panic" in b:
        return {"d_level_db": None, "d_centroid_oct": None, "d_band_db": None, "score": None}
    silent = (a["rms_db"] < SILENT_DB, b["rms_db"] < SILENT_DB)
    if any(silent):
        both = all(silent)
        return {
            "d_level_db": 0.0 if both else b["rms_db"] - a["rms_db"],
            "d_centroid_oct": 0.0, "d_band_db": 0.0,
            "score": 0.0 if both else MISMATCH_SCORE,
        }  # fmt: skip
    d_level = b["rms_db"] - a["rms_db"]
    d_oct = math.log2(b["centroid_hz"] / a["centroid_hz"])
    diffs = [y - x for x, y in zip(a["bands_db"], b["bands_db"]) if max(x, y) > BAND_RELEVANT_DB]
    d_band = math.sqrt(sum(d * d for d in diffs) / len(diffs)) if diffs else 0.0
    score = abs(d_level) + d_band + CENTROID_DB_PER_OCTAVE * abs(d_oct)
    return {"d_level_db": d_level, "d_centroid_oct": d_oct, "d_band_db": d_band, "score": score}


def dominant(d: dict) -> str:
    """Which term of the score is largest: `level`, `bands` or `centroid`."""
    if d["score"] is None:
        return "panic"
    if d["score"] == MISMATCH_SCORE:
        return "silent_mismatch"
    terms = {
        "level": abs(d["d_level_db"]),
        "bands": d["d_band_db"],
        "centroid": CENTROID_DB_PER_OCTAVE * abs(d["d_centroid_oct"]),
    }
    return max(terms, key=lambda k: terms[k])


def silent_flag(engine: str, expect: dict | None) -> str:
    """`<engine>_chain_silent` (a defect) unless `expect` (`tools/program_expect.py`) says the
    firmware itself plays nothing of that engine at the key: then `<engine>_expected_silent`."""
    if expect is not None and not expect["audible"][engine]:
        return f"{engine}_expected_silent"
    return f"{engine}_chain_silent"


def chain_flags(row: dict, chain: dict | None, expect: dict | None = None) -> dict:
    """Chain voicing facts and flags for one program (see the doc s3 for what each tap means)."""
    eng = engines(row)
    out = {
        "chain_run": bool(chain and chain["census"]),
        "pno1_peak": None, "pno2_peak": None, "org2_peak": None,
        "sy_prepare": None, "sy_audio_peak": None, "flags": [],
    }  # fmt: skip
    if not out["chain_run"]:
        out["flags"].append("chain_run_failed")
        return out
    c = chain["census"]["chips"]
    out["pno1_peak"], out["pno2_peak"], out["org2_peak"] = (
        c.get(n, {}).get("peak") for n in ("PNO1", "PNO2", "ORG2")
    )
    out["sy_prepare"] = chain["census"]["sy"]["prepare"]
    audio = chain.get("audio")
    out["sy_audio_peak"] = audio["sy"]["peak"] if audio else None
    flags = out["flags"]
    if "organ" in eng and not out["pno1_peak"]:
        flags.append(silent_flag("organ", expect))
    if "synth" in eng and not (out["pno2_peak"] or out["org2_peak"]):
        flags.append(silent_flag("synth", expect))
    if "sample" in eng:
        layers = row["layers"]["synth_sample"]
        if not out["sy_prepare"]:
            flags.append("sample_no_voice")
        elif out["sy_prepare"] < layers:
            flags.append("sample_layer_lost")
        if audio is not None and not out["sy_audio_peak"] and out["sy_prepare"]:
            flags.append("sample_voiced_no_audio")
    if "piano" in eng:
        flags.append("piano_no_chain_tap")
    return out | {"flags": flags}


def bank_of(entry: str) -> str:
    """`A` of `Program/Bank A/Name.ns4p`."""
    return entry.split("/")[1].removeprefix("Bank ")


def build_rows(
    native: list[dict], chain: list[dict], expect: list[dict] | None = None
) -> list[dict]:
    """One flat row per program, sorted by entry (`expect`: `program_expect.py` lines, optional)."""
    by_chain = {c["entry"]: c for c in chain}
    by_expect = {e["entry"]: e for e in expect or []}
    rows = []
    for n in sorted(native, key=lambda r: r["entry"]):
        d = distance(n["default"], n["firmware"])
        ch = chain_flags(n, by_chain.get(n["entry"]), by_expect.get(n["entry"]))
        rows.append({"entry": n["entry"], "engines": "+".join(engines(n)), "native": n,
                     "dist": d, "dominant": dominant(d), "chain": ch})  # fmt: skip
    return rows


def f(x: float | None, nd: int = 3) -> str | float:
    """Rounded number or empty string."""
    return "" if x is None else round(x, nd)


def csv_header() -> list[str]:
    """Column names of the committed table."""
    cols = [
        "entry",
        "bank",
        "engines",
        "organ_model",
        "piano_layers",
        "synth_layers",
        "sample_layers",
    ]
    for mode in ("def", "fw"):
        cols += [f"{mode}_rms_db", f"{mode}_centroid_hz"] + [f"{mode}_b{i}" for i in range(BANDS)]
    cols += ["d_level_db", "d_centroid_oct", "d_band_db", "score", "dominant"]
    cols += ["pno1_peak", "pno2_peak", "org2_peak", "sy_prepare", "sy_audio_peak", "flags"]
    return cols


def csv_row(r: dict) -> list:
    """One CSV row (numbers rounded to 3 decimals; deterministic)."""
    n, lay = r["native"], r["native"]["layers"]
    out = [r["entry"], bank_of(r["entry"]), r["engines"], lay["organ_model"], lay["piano"],
           lay["synth"], lay["synth_sample"]]  # fmt: skip
    for key in ("default", "firmware"):
        m = n[key]
        out += [f(m.get("rms_db")), f(m.get("centroid_hz"), 1)] + [
            f(b, 2) for b in m.get("bands_db", [None] * BANDS)
        ]
    d, c = r["dist"], r["chain"]
    out += [
        f(d["d_level_db"]),
        f(d["d_centroid_oct"], 4),
        f(d["d_band_db"]),
        f(d["score"]),
        r["dominant"],
    ]
    sy_prepare = "" if c["sy_prepare"] is None else c["sy_prepare"]
    out += [f(c["pno1_peak"], 5), f(c["pno2_peak"], 5), f(c["org2_peak"], 5), sy_prepare]
    out += [f(c["sy_audio_peak"], 5), ";".join(c["flags"])]
    return out


def write_csv(rows: list[dict], path: Path) -> None:
    """Writes the committed table (LF line ends)."""
    with path.open("w", newline="", encoding="utf-8") as fh:
        w = csv.writer(fh, lineterminator="\n")
        w.writerow(csv_header())
        w.writerows(csv_row(r) for r in rows)


def md_table(header: list[str], body: list[list]) -> str:
    """GitHub table."""
    lines = ["| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    lines += ["| " + " | ".join(str(x) for x in row) + " |" for row in body]
    return "\n".join(lines)


def worst(rows: list[dict], k: int = 20) -> list[dict]:
    """Top `k` by score (silent mismatches first, ties by entry)."""
    ranked = [r for r in rows if r["dist"]["score"] is not None]
    return sorted(ranked, key=lambda r: (-r["dist"]["score"], r["entry"]))[:k]


def by_engine(rows: list[dict]) -> list[list]:
    """Per engine group: count, median / max score, silent-flag counts."""
    groups: dict[str, list[dict]] = {}
    for r in rows:
        groups.setdefault(r["engines"], []).append(r)
    out = []
    for g, rs in sorted(groups.items()):
        sc = sorted(r["dist"]["score"] for r in rs if r["dist"]["score"] is not None)
        med = statistics.median(sc) if sc else None
        flagged = sum(
            1 for r in rs if [x for x in r["chain"]["flags"] if x != "piano_no_chain_tap"]
        )
        out.append([g, len(rs), f(med, 2), f(sc[-1] if sc else None, 2), flagged])
    return out


def flag_counts(rows: list[dict]) -> list[list]:
    """Count of programs per chain flag."""
    cnt: dict[str, int] = {}
    for r in rows:
        for x in r["chain"]["flags"]:
            cnt[x] = cnt.get(x, 0) + 1
    return [[k, v] for k, v in sorted(cnt.items())]


def cause_row(row: dict) -> dict:
    """Score left when one option group is put back to the default, and the group that carries it.

    `row` is a `program_rank --ablate` line. The carrier is the group whose revert leaves the
    smallest score (it removes the most of the distance); `none` when the full score is 0.
    """
    full = distance(row["default"], row["firmware"])["score"]
    left = {g: distance(row["default"], m)["score"] for g, m in sorted(row["ablate"].items())}
    valid = {g: v for g, v in left.items() if v is not None}
    carrier = min(valid, key=lambda g: valid[g]) if valid and full else "none"
    return {"entry": row["entry"], "score": full, "left": left, "carrier": carrier}


def write_cause_csv(rows: list[dict], path: Path) -> None:
    """Committed ablation table: score left per reverted group."""
    groups = sorted({g for r in rows for g in r["left"]})
    with path.open("w", newline="", encoding="utf-8") as fh:
        w = csv.writer(fh, lineterminator=NL)
        w.writerow(["entry", "score", *[f"left_without_{g}" for g in groups], "carrier"])
        for r in rows:
            left = [f(r["left"].get(g)) for g in groups]
            w.writerow([r["entry"], f(r["score"]), *left, r["carrier"]])


def tables(rows: list[dict]) -> str:
    """Markdown tables for the research note."""
    w = []
    for i, r in enumerate(worst(rows)):
        d = r["dist"]
        name = r["entry"].split("/", 1)[1][:-5]
        w.append(
            [i + 1, name, r["engines"], f(d["d_level_db"], 2), f(d["d_band_db"], 2)]
            + [f(d["d_centroid_oct"], 2), f(d["score"], 2), r["dominant"]]
            + [";".join(r["chain"]["flags"]) or "-"]
        )
    parts = [
        md_table(
            [
                "#",
                "Program",
                "Engines",
                "dLevel dB",
                "dBands dB",
                "dCentroid oct",
                "Score",
                "Dominant",
                "Chain flags",
            ],
            w,
        ),
        md_table(
            [
                "Engines (scene I)",
                "Programs",
                "Median score",
                "Max score",
                "Programs with chain flag",
            ],
            by_engine(rows),
        ),
        md_table(["Chain flag", "Programs"], flag_counts(rows)),
    ]
    return "\n\n".join(parts)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--native", type=Path, required=True)
    ap.add_argument("--chain", type=Path, required=True)
    ap.add_argument("--expect", type=Path, help="program_expect.py lines (silent by program)")
    ap.add_argument("--csv", type=Path)
    ap.add_argument("--md-tables", action="store_true")
    ap.add_argument("--ablate", type=Path, help="program_rank --ablate lines of the worst programs")
    ap.add_argument("--cause-csv", type=Path)
    ns = ap.parse_args(argv)
    expect = load_jsonl(ns.expect) if ns.expect else None
    rows = build_rows(load_jsonl(ns.native), load_jsonl(ns.chain), expect)
    if ns.csv:
        write_csv(rows, ns.csv)
    if ns.md_tables:
        print(tables(rows))
    if ns.ablate:
        causes = [cause_row(r) for r in load_jsonl(ns.ablate)]
        if ns.cause_csv:
            write_cause_csv(causes, ns.cause_csv)
        body = [[c["entry"], f(c["score"], 2), c["carrier"]] for c in causes]
        print(md_table(["Program", "Score", "Carrier"], body))
    print(f"{len(rows)} programs", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
