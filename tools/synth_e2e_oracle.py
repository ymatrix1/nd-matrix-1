#!/usr/bin/env python3
r"""Synth chain end to end: SHARC oracle driven by the real ARM stream against native `nord-core`.

For one factory program (analog synth layer A) this

1. rebuilds the ARM -> ORG core 2 recall stream and the key-on / key-off words from an
   `arm_dsp_capture.py` file (phases `load` / `idle` for the strips, `k60` / `k60_off` for the
   voice word `DM(0xC1DCC)`, which the real ARM sends to PNO core 2 only);
2. runs the ORG core 2 image in `sharc-sys` with that stream and taps the stage record `IN0`
   after every pass (4 samples per pass: the B stage writes the filtered voice in place over the
   A record), once with the real filter stage and once with the B stage replaced by the null
   stage before every pass (oscillator and envelope path only);
3. renders the same layer natively (`synth_e2e_native`) with and without the filter;
4. reports levels, envelope shape, pitch, third-octave spectrum and null depth per stage
   (`tools/synth_e2e_metrics.py`). Audio stays in `--workdir`; only numbers are printed.

    cargo build --release -p nord-sharc --bin sharc-sys -p nord-tools --bin synth_e2e_native
    uv run python tools/synth_e2e_oracle.py report CAPTURE.txt.gz PROGRAM.ns4p \
        --sharc-sys target/release/sharc-sys --image DIR/core2_sram.bin \
        --native target/release/synth_e2e_native --workdir target/synth_e2e
"""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path

import numpy as np
import synth_e2e_metrics as m

# ASSUMED (D121): a pass is 2 SY frames at 43,402.34 Hz and the stage runs 4 samples per pass.
FRAME_RATE = 43_402.34375
PASS_S = 2.0 / FRAME_RATE
SAMPLES_PER_PASS = 4
ORACLE_FS = SAMPLES_PER_PASS / PASS_S
NATIVE_FS = 48_000.0
# VERIFIED (synth_stage_oracle.py, org-stage-map s2): channel 0 stage input record `I4 - 13`.
IN0 = 0xC1F96 - 13
# VERIFIED (synth-adr-and-strip-law s3): slot 0 envelope frame F; chain states at F-20, F-8, F+6.
ENV_FRAME = 0xC1188
ENV_TAPS = (ENV_FRAME - 20, ENV_FRAME - 8, ENV_FRAME + 6)
B_ENTRY = 0xC0B8C
NULL_B = 0x125B79
KEY_ON_PASS = 300
HOLD_S, TAIL_S = 0.5, 0.25
STEADY_FROM_S, STEADY_BEFORE_OFF_S = 0.15, 0.05


def core2_load_script(rows: list[dict]) -> str:
    """Flat `0 DM VALUE` lines of the ORG core 2 recall (phases load and idle), file order."""
    sel = [
        r for r in rows if r["phase"] in ("load", "idle") and r["chip"] == "ORG" and r["core"] == 2
    ]
    return "".join(f"0 {r['dm']:#x} {r['w2']:#010x}\n" for r in sel)


def key_script(rows: list[dict], on_pass: int, off_pass: int, key: int = 60) -> str:
    """Key-down / key-up voice words (PNO core 2 messages of the capture) at the given passes.

    Every word of the phase is delivered in capture order: a layer with a second lane sends
    `DM(0xC1DCD)` before `DM(0xC1DCC)`.
    """
    on = [r for r in rows if r["phase"] == f"k{key}" and r["core"] == 2]
    off = [r for r in rows if r["phase"] == f"k{key}_off" and r["core"] == 2]
    if not on or not off:
        raise ValueError(f"capture has no key {key} voice words")
    lines = [f"{on_pass} {r['dm']:#x} {r['w2']:#010x}" for r in on]
    lines += [f"{off_pass} {r['dm']:#x} {r['w2']:#010x}" for r in off]
    return "\n".join(lines) + "\n"


def null_b_script(passes: int) -> str:
    """Poke the null stage into the channel 0 B entry before every pass."""
    return "".join(f"{p} {B_ENTRY:#x} {NULL_B:#x}\n" for p in range(passes))


def voice_samples(words: np.ndarray, passes: int) -> np.ndarray:
    """Channel 0 samples (PEx lane) from the tap file words: 4 per pass, in time order."""
    f = words.astype("<u4").view("<f4").reshape(passes, 2 * SAMPLES_PER_PASS)
    return f[:, ::2].reshape(-1).astype(np.float64)


def run_oracle(
    sys_bin: str, image: str, scripts: list[Path], passes: int, out: Path
) -> tuple[np.ndarray, np.ndarray]:
    """One `sharc-sys` run (first script flat): channel 0 samples and the 3 envelope states."""
    cmd = [sys_bin, image, "--passes", str(passes), "--warmup", "0", "--epdma"]
    cmd += ["--script-flat", str(scripts[0])]
    for s in scripts[1:]:
        cmd += ["--script-at", f"0:{s}"]
    cmd += ["--tap", f"{IN0:#x}:{2 * SAMPLES_PER_PASS}"]
    for a in ENV_TAPS:
        cmd += ["--tap", f"{a:#x}:1"]
    cmd += ["--tap-out", str(out)]
    r = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if "tap:" not in r.stdout:
        raise RuntimeError(r.stdout[-800:] + r.stderr[-400:])
    width = 2 * SAMPLES_PER_PASS + len(ENV_TAPS)
    words = np.fromfile(out, dtype="<u4").reshape(-1, width)
    audio = voice_samples(words[:, : 2 * SAMPLES_PER_PASS].copy(), len(words))
    return audio, words[:, 2 * SAMPLES_PER_PASS :].copy().view("<f4").astype(np.float64)


def run_native(
    native: str, program: str, out: Path, frames: tuple[int, int, int], extra: list[str]
) -> tuple[np.ndarray, dict]:
    """Native layer A render (left channel) of the held note."""
    total, start, hold = frames
    cmd = [native, program, "--out", str(out), "--frames", str(total)]
    cmd += ["--start", str(start), "--hold", str(hold)]
    r = subprocess.run(cmd + extra, capture_output=True, text=True, check=False)
    if r.returncode != 0:
        raise RuntimeError(r.stderr[-400:])
    info = json.loads(r.stdout.strip().splitlines()[-1])["layer"]
    return np.fromfile(out, dtype="<f4").reshape(-1, 2)[:, 0].astype(np.float64), info


def steady_slice(x: np.ndarray, fs: float, on_s: float, hold_s: float) -> np.ndarray:
    """Samples of the sustained part of the note (after the attack, before the release)."""
    a = on_s + STEADY_FROM_S
    b = on_s + hold_s - STEADY_BEFORE_OFF_S
    return x[int(a * fs) : int(b * fs)]


def stage_report(oracle: np.ndarray, native: np.ndarray, on_s: float, hold_s: float) -> dict:
    """Level, envelope, pitch, spectrum and null numbers of one stage (same note, both signals)."""
    hop = 0.005
    so = steady_slice(oracle, ORACLE_FS, on_s, hold_s)
    sn = steady_slice(native, NATIVE_FS, on_s, hold_s)
    env_o = m.windowed_rms(oracle, int(hop * ORACLE_FS))
    env_n = m.windowed_rms(native, int(hop * NATIVE_FS))
    k0 = int(on_s / hop)
    ref_o, ref_n = m.rms(so), m.rms(sn)
    k1 = int((on_s + hold_s) / hop)
    rel_o = m.time_to_below(env_o[k1:], hop, 0.01, ref_o)
    rel_n = m.time_to_below(env_n[k1:], hop, 0.01, ref_n)
    rel_o, rel_n = max(rel_o, 0.0), max(rel_n, 0.0)
    edges = m.third_octave_edges()
    bo = np.array(m.band_levels_db(so, ORACLE_FS, edges))
    bn = np.array(m.band_levels_db(sn, NATIVE_FS, edges))
    n_env = min(len(env_o), len(env_n))
    depth_env, _ = m.null_depth_db(env_o[:n_env], env_n[:n_env])
    depth_spec, _ = m.null_depth_db(10 ** (bo / 20), 10 ** (bn / 20))
    rs = m.resample_fft(so, ORACLE_FS, NATIVE_FS)
    n = min(len(rs), len(sn))
    depth_wave, _ = m.null_depth_db(rs[:n], sn[:n])
    return {
        "oracle_peak": float(np.abs(so).max()),
        "native_peak": float(np.abs(sn).max()),
        "oracle_rms_db": m.db(ref_o),
        "native_rms_db": m.db(ref_n),
        "level_delta_db": m.db(ref_o) - m.db(ref_n),
        "f0_oracle_hz_at_assumed_rate": m.fundamental_hz(so, ORACLE_FS),
        "f0_native_hz": m.fundamental_hz(sn, NATIVE_FS),
        "t90_oracle_ms": 1000 * m.time_to_fraction(env_o[k0:], hop, 0.9, ref_o),
        "t90_native_ms": 1000 * m.time_to_fraction(env_n[k0:], hop, 0.9, ref_n),
        "release_to_m40db_oracle_ms": 1000 * rel_o,
        "release_to_m40db_native_ms": 1000 * rel_n,
        "env_null_db": depth_env,
        "band_delta_db_mean_abs": m.band_delta_db(bo, bn),
        "spectrum_null_db": depth_spec,
        "waveform_null_db_unaligned": depth_wave,
    }


def out_gain(filtered: np.ndarray, bypass: np.ndarray, on_s: float) -> float:
    """Native filter gain in dB: steady RMS of the filtered note over the unfiltered one."""
    a = m.rms(steady_slice(filtered, NATIVE_FS, on_s, HOLD_S))
    b = m.rms(steady_slice(bypass, NATIVE_FS, on_s, HOLD_S))
    return m.db(a) - m.db(b)


def envelope_facts(y: np.ndarray, on_pass: int, off_pass: int) -> dict:
    """Passes per full ramp of the linear attack and the release time constant of one chain.

    The attack record is a linear ramp `y' = y + cA` (VERIFIED, synth-adr-and-strip-law s2), so
    `cA` is the early slope; the release is `y' = a y`, `tau = -1 / ln a` passes.
    """
    k = on_pass + 20
    slope = (y[k + 20] - y[k]) / 20.0
    r0, r1 = y[off_pass + 100], y[off_pass + 200]
    tau = -100.0 / np.log(r1 / r0) if r0 > 0 and r1 > 0 and r1 != r0 else float("inf")
    return {
        "attack_passes_per_unit": float(1.0 / slope) if slope > 0 else float("inf"),
        "attack_s_per_unit": float(PASS_S / slope) if slope > 0 else float("inf"),
        "release_tau_s": float(tau * PASS_S),
        "level_at_off": float(y[off_pass]),
    }


def report(ns: argparse.Namespace) -> int:  # pragma: no cover - needs local firmware images
    import arm_dsp_capture as cap

    rows = list(cap.read_rows(Path(ns.capture)))
    work = Path(ns.workdir)
    work.mkdir(parents=True, exist_ok=True)
    off_pass = KEY_ON_PASS + int(HOLD_S / PASS_S)
    passes = off_pass + int(TAIL_S / PASS_S)
    load, key, nullb = work / "load.script", work / "key.script", work / "nullb.script"
    load.write_text(core2_load_script(rows))
    key.write_text(key_script(rows, KEY_ON_PASS, off_pass))
    nullb.write_text(null_b_script(passes))
    full, env = run_oracle(ns.sharc_sys, ns.image, [load, key], passes, work / "full.bin")
    osc, _ = run_oracle(ns.sharc_sys, ns.image, [load, key, nullb], passes, work / "nullb.bin")
    on_s = KEY_ON_PASS * PASS_S
    fr = (
        int((on_s + HOLD_S + TAIL_S) * NATIVE_FS),
        int(on_s * NATIVE_FS),
        int((on_s + HOLD_S) * NATIVE_FS),
    )
    nat, info = run_native(ns.native, ns.program, work / "nat.f32", fr, ns.native_args)
    nat_nf, _ = run_native(
        ns.native, ns.program, work / "natnf.f32", fr, ns.native_args + ["--no-filter"]
    )
    gain = m.db(m.rms(steady_slice(full, ORACLE_FS, on_s, HOLD_S))) - m.db(
        m.rms(steady_slice(osc, ORACLE_FS, on_s, HOLD_S))
    )
    out = {
        "program": Path(ns.program).stem,
        "passes": passes,
        "filter_out": stage_report(full, nat, on_s, HOLD_S),
        "oscillator_path": stage_report(osc, nat_nf, on_s, HOLD_S),
        "oracle_filter_gain_db": gain,
        "native_filter_gain_db": out_gain(nat, nat_nf, on_s),
        "native_layer": info,
        "oracle_envelope": [envelope_facts(env[:, k], KEY_ON_PASS, off_pass) for k in range(3)],
    }
    print(json.dumps(out, indent=1))
    return 0


CSV_FIELDS = (
    "program", "variant", "stage", "oracle_rms_db", "native_rms_db", "level_delta_db",
    "f0_oracle_hz", "f0_native_hz", "t90_oracle_ms", "t90_native_ms",
    "band_delta_db", "spectrum_null_db", "env_null_db", "oracle_filter_gain_db",
    "native_filter_gain_db",
)  # fmt: skip


def csv_rows(report_json: dict, variant: str) -> list[dict]:
    """One CSV row per stage (`oscillator_path`, `filter_out`) of a `report` JSON document."""
    rows = []
    for stage in ("oscillator_path", "filter_out"):
        s = report_json[stage]
        rows.append(
            {
                "program": report_json["program"],
                "variant": variant,
                "stage": stage,
                "oracle_rms_db": round(s["oracle_rms_db"], 2),
                "native_rms_db": round(s["native_rms_db"], 2),
                "level_delta_db": round(s["level_delta_db"], 2),
                "f0_oracle_hz": round(s["f0_oracle_hz_at_assumed_rate"], 2),
                "f0_native_hz": round(s["f0_native_hz"], 2),
                "t90_oracle_ms": round(s["t90_oracle_ms"], 1),
                "t90_native_ms": round(s["t90_native_ms"], 1),
                "band_delta_db": round(s["band_delta_db_mean_abs"], 2),
                "spectrum_null_db": round(s["spectrum_null_db"], 2),
                "env_null_db": round(s["env_null_db"], 2),
                "oracle_filter_gain_db": round(report_json["oracle_filter_gain_db"], 2),
                "native_filter_gain_db": round(report_json["native_filter_gain_db"], 2),
            }
        )
    return rows


def summarise(ns: argparse.Namespace) -> int:  # pragma: no cover - CLI wiring
    """CSV of several `report` JSON files, `VARIANT=FILE` each, on stdout."""
    import csv
    import sys

    w = csv.DictWriter(sys.stdout, fieldnames=CSV_FIELDS, lineterminator=chr(10))
    w.writeheader()
    for item in ns.reports:
        variant, _, path = item.partition("=")
        w.writerows(csv_rows(json.loads(Path(path).read_text(encoding="utf-8")), variant))
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
    p.add_argument("--native-arg", dest="native_args", action="append", default=[])
    q = sub.add_parser("csv")
    q.add_argument("reports", nargs="+", help="VARIANT=report.json")
    ns = ap.parse_args(argv)
    return summarise(ns) if ns.cmd == "csv" else report(ns)


if __name__ == "__main__":
    raise SystemExit(main())
