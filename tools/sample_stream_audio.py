# ruff: noqa: E501
#!/usr/bin/env python3
r"""SAMPLE programs through the SY1SY2 oracle with the real sample, measured (D150).

For each program: resolve the layer sample files (native name hash, `smp_stream resolve`), run the
firmware recall with that sample bound (`arm_dsp_capture one --member`), build the oracle scenario
of the played key from the captured ARM to SY messages (`sample_stream_scn`), run `sharc-ref`
twice (real packed stream with the unpacker, then decoded PCM ring refill) and compare with the
native engine render of the same note.

    uv run --with unicorn python tools/sample_stream_audio.py run \
        --backup "<ns4b>" --programs-dir factory_programs/Program --workdir target/ss \
        --out docs/research/data/sample-stream-audio.json

Audio, streams and PCM stay in `--workdir`; the JSON holds metrics only.
"""

from __future__ import annotations

import argparse
import json
import math
import struct
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np
import sample_stream_scn as scn

SY_RATE = 43402.34375  # VERIFIED (pitch_law::SY_FRAME_RATE_HZ)
NATIVE_RATE = 48000.0
KEY, VELOCITY = 60, 100
FRAMES = 65000  # SY frames of one oracle run (1.5 s)
# The 10 programs: first SAMPLE layer, one library category each (loops and one shots).
PROGRAMS = (
    "Bank_A__Acoustic_Guitar", "Bank_A__Hybrid_Str_Whl", "Bank_A__NS4_Palette",
    "Bank_A__Mm-Oh_Choir_Whl", "Bank_C__Pop_Brass_3", "Bank_D__Finger_Bass",
    "Bank_B__Cathedral_Pipes", "Bank_D__3_Vlns_Mellotron", "Bank_E__Solo_Flute",
    "Bank_F__Bell_Pad",
)  # fmt: skip


def entry_of(stem: str) -> str:
    """Archive entry of a capture stem `Bank_A__Name_With_Spaces`."""
    bank, name = stem.split("__", 1)
    return f"Program/{bank.replace('_', ' ')}/{name.replace('_', ' ')}.ns4p"


def first_member(resolved: dict[str, str | None]) -> str | None:
    """Sample file of the first SAMPLE synth layer (layer order A, B, C)."""
    return next((resolved[k] for k in ("synth_a", "synth_b", "synth_c") if resolved.get(k)), None)


def expected_hz(key: int) -> float:
    """Equal tempered frequency of a MIDI key (A4 = 440 Hz)."""
    return 440.0 * 2.0 ** ((key - 69) / 12.0)


def dbfs(v: float) -> float:
    return 20.0 * math.log10(max(abs(v), 1e-12))


def f0_autocorr(
    x: np.ndarray, rate: float, fmin: float = 50.0, fmax: float = 2000.0
) -> float | None:
    """Fundamental by the first strong autocorrelation peak (EMPIRICAL heuristic, `None` if flat)."""
    x = x - np.mean(x)
    if not np.any(x):
        return None
    n = len(x)
    spec = np.fft.rfft(x, 2 * n)
    ac = np.fft.irfft(spec * np.conj(spec))[:n]
    lo, hi = int(rate / fmax), min(int(rate / fmin), n - 1)
    if hi <= lo or ac[0] <= 0:
        return None
    seg = ac[lo:hi]
    # first local maximum above 0.8 of the best one: avoids the octave below on harmonic tones
    best = float(seg.max())
    for i in range(1, len(seg) - 1):
        if seg[i] >= 0.8 * best and seg[i] >= seg[i - 1] and seg[i] >= seg[i + 1]:
            return rate / (lo + i)
    return None


def centroid_hz(x: np.ndarray, rate: float) -> float | None:
    """Magnitude weighted spectral centroid of a Hann windowed segment."""
    if len(x) < 64:
        return None
    mag = np.abs(np.fft.rfft(x * np.hanning(len(x))))
    if mag.sum() <= 0:
        return None
    return float(np.sum(np.fft.rfftfreq(len(x), 1.0 / rate) * mag) / mag.sum())


def audio_metrics(x: np.ndarray, rate: float, key: int = KEY) -> dict[str, object]:
    """Level, duration, centroid and pitch of a mono signal (windows in seconds after the start)."""
    peak = float(np.max(np.abs(x))) if len(x) else 0.0
    out: dict[str, object] = {"nonsilent": peak > 1e-7, "peak": peak, "peak_dbfs": dbfs(peak)}
    if peak <= 1e-7:
        return out | {
            "rms_dbfs": None,
            "duration_s": 0.0,
            "centroid_hz": None,
            "f0_hz": None,
            "cents": None,
        }
    seg = lambda a, b: x[int(a * rate) : int(b * rate)]  # noqa: E731
    w = seg(0.1, 0.6)
    out["rms_dbfs"] = dbfs(float(np.sqrt(np.mean(w**2)))) if len(w) else None
    win = max(1, int(0.02 * rate))
    env = np.sqrt(np.convolve(x**2, np.ones(win) / win, mode="same"))
    live = np.nonzero(env >= env.max() * 1e-3)[0]  # -60 dB re the envelope peak
    out["duration_s"] = float((live[-1] - live[0]) / rate) if len(live) else 0.0
    out["centroid_hz"] = centroid_hz(seg(0.05, 0.55), rate)
    f0 = f0_autocorr(seg(0.1, 0.5), rate)
    out["f0_hz"] = f0
    out["cents"] = 1200.0 * math.log2(f0 / expected_hz(key)) if f0 else None
    return out


def log_spectrum(x: np.ndarray, rate: float, grid: np.ndarray) -> np.ndarray:
    """Mean log magnitude (dB) of Hann frames of `x` on the log-frequency `grid` (Hz)."""
    n = 8192
    frames = [
        x[i : i + n] * np.hanning(n)
        for i in range(0, max(len(x) - n, 1), n // 2)
        if len(x[i : i + n]) == n
    ]
    if not frames:
        return np.zeros(len(grid))
    mag = np.mean([np.abs(np.fft.rfft(f)) for f in frames], axis=0)
    freqs = np.fft.rfftfreq(n, 1.0 / rate)
    return 20.0 * np.log10(np.interp(grid, freqs, mag) + 1e-12)


def pitch_shift_cents(
    a: np.ndarray, rate_a: float, b: np.ndarray, rate_b: float, max_cents: float = 1200.0
) -> float | None:
    """Pitch of `a` relative to `b` (cents) by sliding the log spectra of the 0.1 to 1.2 s part.

    EMPIRICAL: valid when both signals play the same sample (same spectral shape); the best
    alignment of the mean-removed log spectra over 150 Hz .. 5 kHz, 1/48 octave grid.
    """
    cut = lambda x, r: x[int(0.1 * r) : int(1.2 * r)]  # noqa: E731
    step = 1200.0 / 48.0
    grid = 150.0 * 2.0 ** (np.arange(0, int(math.log2(5000 / 150) * 48)) / 48.0)
    sa = log_spectrum(cut(a, rate_a), rate_a, grid)
    sb = log_spectrum(cut(b, rate_b), rate_b, grid)
    if not (np.any(sa > -200) and np.any(sb > -200)) or sa.std() == 0 or sb.std() == 0:
        return None
    sa, sb = sa - sa.mean(), sb - sb.mean()
    best, best_c = -2.0, 0
    for k in range(-int(max_cents / step), int(max_cents / step) + 1):
        lo, hi = max(0, k), min(len(grid), len(grid) + k)  # sa shifted up by k bins vs sb
        if hi - lo < 24:
            continue
        c = float(np.corrcoef(sa[lo - k : hi - k], sb[lo:hi])[0, 1])
        if c > best:
            best, best_c = c, k
    return -best_c * step  # a feature at bin j of `a` sits at j + k in `b`: `a` is k bins lower


def read_wav_f32(path: Path) -> tuple[np.ndarray, int]:
    """Mono mix and rate of a 32-bit float stereo WAV written by `ns4 render-patch`."""
    data = path.read_bytes()
    rate = struct.unpack_from("<I", data, 24)[0]
    at = data.index(b"data") + 8
    st = np.frombuffer(data[at:], dtype="<f4")
    return st.reshape(-1, 2).mean(axis=1).astype(np.float64), rate


def mono_of(dump: dict[str, list[float]]) -> np.ndarray:
    return 0.5 * (np.array(dump["L"]) + np.array(dump["R"]))


def run_cmd(args: list[str], **kw) -> subprocess.CompletedProcess[str]:  # pragma: no cover
    return subprocess.run([str(a) for a in args], capture_output=True, text=True, check=False, **kw)


def step_ratio(word_as_float: float) -> float:
    """SY step (source frames per SY frame) of the record word `rec+0x2D` the dump prints as a float."""
    return struct.unpack("<I", struct.pack("<f", word_as_float))[0] / 2.0**24


def prepare_sent(rows: list[dict[str, object]]) -> bool:
    """Whether the ARM sent the prepare state `0x1C07CD` for voice 0 in these rows."""
    return any(
        r["chip"] == "SY"
        and r["path"] == 1
        and int(r["dm"]) == scn.REC_BASE + scn.STATE_FIELD
        and int(r["w2"]) == scn.STATE_PREPARE
        for r in rows
    )  # type: ignore[call-overload]


def trap_summary(stderr: str) -> str:
    """Short reason of a failed `sharc-ref` run: `interrupt 17 not finished ... (pc 0x1c12dd` -> `trap pc 0x1c12dd`."""
    text = stderr.strip().splitlines()[-1] if stderr.strip() else "no output"
    if "not finished" in text and "(pc " in text:
        return "interrupt 17 trap at pc " + text.split("(pc ")[1].split(",")[0].rstrip(")")
    return text[:120]


def pad_stats(ring: list[float]) -> dict[str, int]:
    """Non-zero count of the captured source ring frames (`stream` mode)."""
    return {"ring_nonzero": int(np.count_nonzero(ring)), "ring_len": len(ring)}


def program_row(
    stem: str, ctx: dict[str, object]
) -> dict[str, object]:  # pragma: no cover - runs tools
    """Run one program end to end and return its metrics row."""
    work = Path(str(ctx["workdir"]))
    exe = Path(str(ctx["exe_dir"]))
    backup = str(ctx["backup"])
    resolved = json.loads(
        run_cmd([exe / "smp_stream.exe", "resolve", backup, entry_of(stem)]).stdout
    )
    member = first_member(resolved)
    row: dict[str, object] = {"program": stem, "layers": resolved, "member": member}
    if member is None:
        return row | {"error": "no resolved sample"}
    cap = next((work / "cap2" / stem).glob("*.txt.gz"), None)
    if cap is None:
        return row | {"error": "no capture (run arm_dsp_capture one with --member)"}
    import arm_dsp_capture as ac

    rows = [r for r in ac.read_rows(cap) if r["phase"] == f"k{KEY}"]
    pro = scn.prologue(Path(str(ctx["scenario"])))
    msgs, index = scn.voice0_messages(rows)
    row["arm_prepare"] = prepare_sent(rows)
    row["entry_index"] = index
    out_dir = work / "runs" / stem
    out_dir.mkdir(parents=True, exist_ok=True)
    info = json.loads(
        run_cmd(
            [
                exe / "smp_stream.exe",
                "stream",
                backup,
                member,
                KEY,
                VELOCITY,
                out_dir / "s.stream",
                out_dir / "s.f32",
            ]
        ).stdout
    )
    row["stroke"] = info
    ent = json.loads(
        run_cmd(
            [exe / "smp_stream.exe", "entry", KEY, info["zone_root"], info["sample_rate"]]
        ).stdout
    )
    row["model_entry"] = ent
    c0, c1 = scn.coefficients(rows)
    ring = scn.ring_signal(scn.read_pcm(out_dir / "s.f32"), c0, c1)
    scn.write_pcm(out_dir / "ring.f32", ring)
    e_addr = scn.ENTRY_BASE + index
    runs = {
        "pcm": scn.build_pcm_scenario(
            pro, out_dir / "ring.f32", msgs, e_addr, ent["entry"], FRAMES
        ),
        "pcm_noentry": scn.build_pcm_scenario(pro, out_dir / "ring.f32", msgs, e_addr, 0, 4000),
        "stream": scn.build_stream_scenario(
            pro, out_dir / "s.stream", scn_arm_order(rows), e_addr, ent["entry"], 3000
        ),
    }
    for name, text in runs.items():
        (out_dir / f"{name}.scn").write_text(text, encoding="utf-8")
        (out_dir / f"{name}.csv").unlink(missing_ok=True)
        res = run_cmd(
            [
                exe / "sharc-ref.exe",
                Path(str(ctx["image"])),
                out_dir / f"{name}.scn",
                "--out",
                out_dir / name,
            ]
        )
        if not (out_dir / f"{name}.csv").exists():
            row[name] = {"error": trap_summary(res.stderr)}
            continue
        dump = scn.read_dump(out_dir / f"{name}.csv")
        m = audio_metrics(mono_of(dump), SY_RATE)
        if dump.get("step"):
            m["step_ratio"] = step_ratio(dump["step"][-1])
        if name == "stream":
            m |= pad_stats(dump.get("ringL", []))
        row[name] = m
    wav = out_dir / "native.wav"
    run_cmd([exe / "ns4.exe", "render-patch", entry_of(stem), "--backup", backup, "--note", KEY, "--velocity", VELOCITY,
             "--frames", 72000, "--hold", 60000, "--wav", wav])  # fmt: skip
    if wav.exists():
        x, rate = read_wav_f32(wav)
        row["native"] = audio_metrics(x, rate)
        dump = scn.read_dump(out_dir / "pcm.csv") if (out_dir / "pcm.csv").exists() else None
        if dump and isinstance(row.get("pcm"), dict) and row["pcm"]["nonsilent"]:  # type: ignore[index]
            row["pcm"]["vs_native_cents"] = pitch_shift_cents(mono_of(dump), SY_RATE, x, rate)  # type: ignore[index]
    return row


def scn_arm_order(rows: list[dict[str, object]]) -> list[tuple[int, int]]:
    """Every SY path 1 message in ARM order."""
    return [(int(r["dm"]), int(r["w2"])) for r in rows if r["chip"] == "SY" and r["path"] == 1]  # type: ignore[call-overload]


def main(argv: list[str] | None = None) -> int:  # pragma: no cover
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--backup", required=True)
    r.add_argument("--workdir", type=Path, default=Path("target/ss"))
    r.add_argument("--exe-dir", type=Path, default=Path("target/release"))
    r.add_argument("--image", type=Path, default=Path("target/oracle/sy.bin"))
    r.add_argument(
        "--scenario", type=Path, default=Path("tools/oracle_scenarios/sy_isr_stream.scn")
    )
    r.add_argument("--out", type=Path, required=True)
    r.add_argument("--programs", nargs="*", default=list(PROGRAMS))
    r.add_argument("--workers", type=int, default=4)
    ns = ap.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    ctx = {k: str(v.resolve()) if isinstance(v, Path) else v for k, v in vars(ns).items()}
    with ThreadPoolExecutor(ns.workers) as pool:
        rows = list(pool.map(lambda s: program_row(s, ctx), ns.programs))
    ns.out.write_text(json.dumps(rows, indent=1) + "\n", encoding="utf-8")
    print(f"wrote {ns.out}: {len(rows)} programs")
    return 0


if __name__ == "__main__":
    sys.exit(main())
