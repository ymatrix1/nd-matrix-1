#!/usr/bin/env python3
r"""Osc Env Amount under Env To Pitch: ORG core 2 oracle against the native engine (OQ44, D217).

The ARM sends the dispatched amount as common word 12 (`DM(0xE02F4)`, `osc_env_dsp_word`) and, for
Env To Pitch, zeroes strip words 20 / 24 (the Osc Ctrl routes) and writes common word 4 =
`0x30000000` (Time Warp Saw recall diff, `docs/research/synth-osc-env-pitch.md`). The oscillator
stage then adds `48 * word * level` semitones to its pitch, `level` the osc envelope chain
`F + 6 = DM(0xC118E)`. Two subcommands:

* `law`: Time Warp Saw recall, osc ADR slowed to nothing, `F + 6` poked to `level` before every
  pass, word 12 swept; the pitch is the FFT peak of the oscillator output (`DM(0xC1F89)` PEx lane)
  against the word 0 run. Writes `data/synth-osc-env-pitch-oracle.csv`.
* `layers`: per factory layer with an amount, the law at held levels 0.5 / 1.0 (probe word and the
  program's own word), layers A, B, C (voice slot = layer, addresses `osc_adr_addr` ...).
* `velocity`: pitch against the key byte `g` for a capture of a program with `kOscEnvVel` on.
* `trace`: per factory program and layer: recall capture + injected osc ADR words and word 12,
  the oscillator path (filter stage nulled), pitch trace per window next to the model
  `48 * word * F+6` and next to the native render (`synth_e2e_native --synth-osc-env-morph`).

    cargo build --release -p nord-sharc --bin sharc-sys -p nord-tools --bin synth_e2e_native
    uv run python tools/synth_osc_env_pitch.py law --sharc-sys target/release/sharc-sys.exe \
        --image DIR/core2_sram.bin --work target/tws/work1 --out docs/research/data/x.csv
"""

from __future__ import annotations

import argparse
import csv
import subprocess
import sys
from pathlib import Path

import numpy as np

# VERIFIED (synth-voice-control s3): layer A osc ADR strip words 32..34, common word 12, F + 6.
OSC_ADR = 0xE0020
WORD12 = 0xE02F4
OSC_LEVEL = 0xC118E
WORD_KEY = 0xC1DCC  # slot 0 controller (key) word
OSC_SAMPLES = 0xC1F89  # 8 words per pass, PEx lane = every other word
# VERIFIED (this tool, `layers`): layer L plays in voice slot L (Cinema Rise A, B, C; Detuned
# Fanfare A, B). Its strip is 2 L (62 words each), its common block `0xE02E8 + 32 L`, its osc
# envelope chain `F + 6` sits at `0xC118E + 62 (L >> 1) + (L & 1)` (two lanes per strip chain) and
# its voice record at `0xC1F89 + 54 L`.
STRIP_WORDS, COMMON_STRIDE, RECORD_STRIDE = 62, 32, 54
# VERIFIED (synth-env-oracle-trace s3): 2 frames per pass, F = 43,402.34 Hz, 4 PEx samples per pass.
ORACLE_RATE_HZ = 86_804.69
SLOWEST_RATE = 0x3F000000  # rate index +63: the chain does not move on its own
NULL_B_STAGE = (0xC0B8C, 0x125B79)  # oscillator path: filter B stage nulled (e2e oracle)
SEMITONES_FULL = 48.0
MIN_MEASURABLE_HZ = 40.0  # the peak search starts at 20 Hz; the window holds too few cycles below
LAW_HOLD = (2400, 4200)  # passes of the steady window (key down from pass 2000)


def osc_adr_addr(layer: int) -> int:
    """`DM` of the layer osc ADR attack word (strip `2 layer`)."""
    return OSC_ADR + 2 * STRIP_WORDS * layer


def word12_addr(layer: int) -> int:
    """`DM` of the layer common word 12 (`osc_env_dsp_word`)."""
    return WORD12 + COMMON_STRIDE * layer


def level_addr(layer: int) -> int:
    """`DM` of the layer osc envelope chain `F + 6` (lane `layer & 1` of chain `layer >> 1`)."""
    return OSC_LEVEL + STRIP_WORDS * (layer >> 1) + (layer & 1)


def samples_addr(layer: int) -> int:
    """`DM` of the layer voice record (8 words per pass, PEx lane = every other word)."""
    return OSC_SAMPLES + RECORD_STRIDE * layer


def vel_factor(g: int) -> float:
    """Velocity factor `G / 128` of the key byte `g` (`G = 128` for `g = 127`, VERIFIED)."""
    return (128 if g >= 127 else g) / 128.0


def key_word(g: int, layer_byte: int = 0x0B) -> int:
    """Synth key word of slot 0 with the velocity byte `0x80 + g` (byte 1), `layer_byte` on top."""
    return (layer_byte << 24) | ((0x80 + g) << 8)


def f2w(x: float) -> int:
    """float32 bits of `x`."""
    return int(np.float32(x).view(np.uint32))


def pitch_peak_hz(x: np.ndarray, fs: float, fmin: float = 20.0, fmax: float | None = None) -> float:
    """Frequency of the spectral peak of `x` (Hann window, 8x zero padding, parabolic peak)."""
    x = np.asarray(x, dtype=np.float64)
    x = x - x.mean()
    n = len(x)
    nfft = 8 * int(2 ** np.ceil(np.log2(n)))
    m = np.abs(np.fft.rfft(x * np.hanning(n), nfft))
    freqs = np.arange(len(m)) * fs / nfft
    lo = int(np.searchsorted(freqs, fmin))
    hi = len(m) - 1 if fmax is None else int(np.searchsorted(freqs, fmax))
    if hi - lo < 3 or not np.any(m[lo:hi] > 0):
        return 0.0
    k = lo + int(np.argmax(m[lo:hi]))
    # Octave guard: a saw whose strongest line is a harmonic reads the subharmonic peak instead.
    while k // 2 >= max(lo, 3):
        win = m[k // 2 - 3 : k // 2 + 4]
        if win.max() < 0.3 * m[k]:
            break
        k = k // 2 - 3 + int(np.argmax(win))
    if k <= 0 or k >= len(m) - 1:
        return float(freqs[k])
    y0, y1, y2 = m[k - 1], m[k], m[k + 1]
    den = y0 - 2.0 * y1 + y2
    d = 0.5 * (y0 - y2) / den if den != 0.0 else 0.0
    return float((k + d) * fs / nfft)


def pitch_trace(
    x: np.ndarray, fs: float, win: int, hop: int, fmin: float = 20.0, fmax: float | None = None
) -> np.ndarray:
    """`[centre sample, Hz]` rows of the windowed peak pitch of `x`."""
    rows = [
        (s + win / 2.0, pitch_peak_hz(x[s : s + win], fs, fmin, fmax))
        for s in range(0, max(len(x) - win, 0) + 1, hop)
    ]
    return np.array(rows, dtype=np.float64).reshape(-1, 2)


def semitones(f: float, ref: float) -> float:
    """Semitones from `ref` to `f` (0 when either is not positive)."""
    return 12.0 * float(np.log2(f / ref)) if f > 0.0 and ref > 0.0 else 0.0


def model_semitones(word: float, level: float) -> float:
    """The oracle law: `48 * word * level` semitones."""
    return SEMITONES_FULL * word * level


class Oracle:  # pragma: no cover - needs the local firmware image
    """`sharc-sys` runs of one loaded program (`load.script`, `adr.script`, `key.script`)."""

    def __init__(self, sharc_sys: str, image: str, work: Path, scratch: Path) -> None:
        self.sys, self.image, self.work, self.scratch = sharc_sys, image, Path(work), scratch
        self.scratch.mkdir(parents=True, exist_ok=True)

    def run(self, writes: dict[int, int], hold: list[tuple[int, int]], passes: int,
            taps: tuple[tuple[int, int], ...]) -> np.ndarray:  # fmt: skip
        """Per pass float32 columns of `taps` (`(address, words)` each); `writes` at pass 0."""
        import synth_env_trace as t
        from synth_cutoff_terms import every_pass

        ex = self.scratch / "run.ex"
        ex.write_text(t.inject_script(writes) + every_pass(hold, passes), encoding="utf-8")
        out = self.scratch / "run.bin"
        cmd = [self.sys, self.image, "--passes", str(passes), "--warmup", "0", "--epdma"]
        cmd += ["--script-flat", str(self.work / "load.script")]
        for f in (self.work / "adr.script", self.work / "key.script", ex):
            cmd += ["--script-at", f"0:{f}"]
        for a, n in taps:
            cmd += ["--tap", f"{a:#x}:{n}"]
        cmd += ["--tap-out", str(out)]
        subprocess.run(cmd, capture_output=True, text=True, check=True)
        width = sum(n for _, n in taps)
        return np.fromfile(out, dtype="<u4").reshape(passes, width).view("<f4").astype(np.float64)

    def pitch(self, word: float, level: float | None, passes: int = 4200, layer: int = 0) -> float:
        """Steady frequency in Hz of `layer` for its word 12 = `word`, `F + 6` held at `level`."""
        adr = osc_adr_addr(layer)
        writes = {word12_addr(layer): f2w(word), adr: SLOWEST_RATE, adr + 1: SLOWEST_RATE,
                  adr + 2: SLOWEST_RATE}  # fmt: skip
        hold = [] if level is None else [(level_addr(layer), f2w(level))]
        a = self.run(writes, hold, passes, ((samples_addr(layer), 8),))
        x = a[LAW_HOLD[0] : LAW_HOLD[1], 0:8:2].reshape(-1)
        return pitch_peak_hz(x, ORACLE_RATE_HZ)


LAW_WORDS = (-1.0, -0.6, -0.3, -0.1, 0.02, 0.05, 0.1, 0.2, 0.3, 0.5, 0.75, 1.0)
LAW_LEVELS = (0.25, 0.5, 0.75, 1.0)


def law(ns: argparse.Namespace) -> int:  # pragma: no cover - needs local firmware images
    o = Oracle(ns.sharc_sys, ns.image, Path(ns.work), Path(ns.scratch))
    base = o.pitch(0.0, 1.0)
    rows = []
    for w in LAW_WORDS:
        for level in LAW_LEVELS:
            if base * 2.0 ** (model_semitones(w, level) / 12.0) < MIN_MEASURABLE_HZ:
                continue  # below the peak search floor (word -1 at level 1 is 16 Hz)
            st = semitones(o.pitch(w, level), base)
            rows.append({"word": w, "level": level, "oracle_st": round(st, 3),
                         "model_st": round(model_semitones(w, level), 3)})  # fmt: skip
    with Path(ns.out).open("w", encoding="utf-8", newline="") as f:
        wr = csv.DictWriter(f, fieldnames=("word", "level", "oracle_st", "model_st"),
                            lineterminator="\n")  # fmt: skip
        wr.writeheader()
        wr.writerows(rows)
    worst = max(abs(r["oracle_st"] - r["model_st"]) for r in rows)
    print(f"{len(rows)} rows, base {base:.2f} Hz, worst error {worst:.3f} st", file=sys.stderr)
    return 0


def pitch_table(root: Path) -> list[float]:  # pragma: no cover - reads the Rust source
    """`LFO_PITCH_SEMITONES` (the Env To Pitch amount taper) from `synth_tables.rs`."""
    text = (root / "crates/nord-core/src/synth_tables.rs").read_text(encoding="utf-8")
    body = text.split("LFO_PITCH_SEMITONES: [f32; 128] = [", 1)[1].split("];", 1)[0]
    return [float(v) for v in body.split(",") if v.strip()]


def dsp_word(steps: float, table: list[float]) -> float:
    """Python port of `osc_env_dsp_word(steps, true)`: `sign * T(127 abs(x) / 60) / 48`."""
    u = max(-1.0, min(1.0, steps / 60.0))
    pos = abs(u) * 127.0
    i = int(pos)
    st = table[127] if i >= 127 else table[i] + (table[i + 1] - table[i]) * (pos - i)
    return float(np.copysign(st, u)) / SEMITONES_FULL


def at_times(trace: np.ndarray, rate: float, t0: float, times: list[float]) -> list[float]:
    """Trace value (column 1) interpolated at `times` seconds after `t0` samples."""
    t = (trace[:, 0] - t0) / rate
    return [float(np.interp(x, t, trace[:, 1])) for x in times]


TIMES_S = (0.01, 0.02, 0.04, 0.08, 0.16, 0.25)
KEY_ON_PASS, TRACE_PASSES = 2000, 8000
CSV_TRACE = ("program", "layer", "t_ms", "oracle_st", "model_st", "native_fw_st", "native_lin_st")
CSV_LAYERS = ("program", "layer", "amount", "word", "level", "probe_word", "oracle_probe_st",
              "model_probe_st", "oracle_own_st", "model_own_st")  # fmt: skip
PROBE_WORD = 0.1  # 4.8 st at level 1: far above the estimator floor for every voice
HOLD_LEVELS = (0.5, 1.0)


def native_trace(
    exe: str, prog: Path, amount: float | None, fw: bool, layer: int = 0
) -> np.ndarray:  # pragma: no cover
    """Native pitch trace `[sample, Hz]` of `layer` (filter off, unity amp), note 60 at frame 0."""
    out = Path("target/synth_osc_env_pitch/native.f32")
    out.parent.mkdir(parents=True, exist_ok=True)
    cmd = [exe, str(prog), "--out", str(out), "--layer", str(layer), "--note", "60",
           "--velocity", "100", "--frames", "24000", "--start", "0", "--hold", "20000",
           "--no-filter", "--unity"]  # fmt: skip
    if fw:
        cmd.append("--synth-osc-env-morph")
    if amount is not None:
        cmd += ["--osc-env-amount", str(amount)]
    subprocess.run(cmd, capture_output=True, text=True, check=True)
    x = np.fromfile(out, dtype="<f4").reshape(-1, 2)[:, 0].astype(np.float64)
    return pitch_trace(x, 48_000.0, 1024, 480, 20.0, 4000.0)


def env_layers(prog: Path) -> list[tuple[int, dict[str, int]]]:  # pragma: no cover - reads files
    """`(layer, fields)` of the layers with Env To Pitch and a non-zero amount."""
    import ns4_synth_fields as nf

    pl = nf.synth_payload_from_ns4p(prog.read_bytes())
    out = [(i, nf.read_layer_fields(pl, i)) for i in range(3)]
    return [(i, f) for i, f in out if f["osc_env_to_pitch"] and f["osc_env_amt"]]


def program_work(ns: argparse.Namespace, prog: Path) -> Path:  # pragma: no cover - needs captures
    """Oracle work dir (`load` / `key` / `adr` scripts) of the recall capture of `prog`."""
    import arm_dsp_capture as cap
    import synth_e2e_oracle as eo

    capture = Path(ns.captures) / f"{Path(ns.programs).name}__{prog.stem.replace(' ', '_')}.txt.gz"
    if not capture.exists():
        capture = Path(ns.captures) / f"{cap.out_name(prog)}.txt.gz"
    rows = list(cap.read_rows(capture))
    work = Path(ns.scratch) / f"work_{prog.stem.replace(' ', '_')}"
    work.mkdir(parents=True, exist_ok=True)
    (work / "load.script").write_text(eo.core2_load_script(rows))
    (work / "key.script").write_text(eo.key_script(rows, KEY_ON_PASS, KEY_ON_PASS + 10_850))
    (work / "adr.script").write_text("")
    return work


def write_csv(path: Path, fields: tuple[str, ...], rows: list[dict]) -> None:  # pragma: no cover
    with path.open("w", encoding="utf-8", newline="") as fh:
        wr = csv.DictWriter(fh, fieldnames=fields, lineterminator="\n")
        wr.writeheader()
        wr.writerows(rows)


def trace(ns: argparse.Namespace) -> int:  # pragma: no cover - needs local firmware images
    import synth_env_trace as t

    table = pitch_table(Path(ns.root))
    rows_out = []
    for prog in sorted(Path(ns.programs).glob("*.ns4p")):
        o = Oracle(ns.sharc_sys, ns.image, program_work(ns, prog), Path(ns.scratch))
        for layer, f in env_layers(prog):
            steps = float(f["osc_env_amt"])
            adr0 = osc_adr_addr(layer)
            adr = {adr0 + i: t.rate_word(f[k]) for i, k in
                   enumerate(("osc_env_attack", "osc_env_decay", "osc_env_release"))}  # fmt: skip
            word = dsp_word(steps, table)

            def run(w: float, layer: int = layer, adr: dict[int, int] = adr) -> np.ndarray:
                return o.run({**adr, word12_addr(layer): f2w(w)}, [], KEY_ON_PASS + TRACE_PASSES,
                             ((level_addr(layer), 1), (samples_addr(layer), 8)))  # fmt: skip

            base_run, mod_run = run(0.0), run(word)

            def tr(a: np.ndarray) -> np.ndarray:
                x = a[KEY_ON_PASS:, 1:9:2].reshape(-1)
                return pitch_trace(x, ORACLE_RATE_HZ, 2048, 1024, 20.0, 4000.0)

            tb, tm = tr(base_run), tr(mod_run)
            ref = float(np.median(tb[:, 1]))
            o_st = np.column_stack([tm[:, 0], [semitones(v, ref) for v in tm[:, 1]]])
            lvl = mod_run[KEY_ON_PASS:, 0]
            m_st = np.column_stack(
                [tm[:, 0], [model_semitones(word, lvl[min(int(c / 4), len(lvl) - 1)])
                            for c in tm[:, 0]]])  # fmt: skip
            nb = native_trace(ns.native, prog, 0.0, True, layer)
            nref = float(np.median(nb[:, 1]))
            series = {}
            for key, fw in (("fw", True), ("lin", False)):
                n = native_trace(ns.native, prog, None, fw, layer)
                series[key] = np.column_stack([n[:, 0], [semitones(v, nref) for v in n[:, 1]]])
            for tt in TIMES_S:
                rows_out.append({
                    "program": prog.stem, "layer": "ABC"[layer], "t_ms": round(tt * 1000),
                    "oracle_st": round(at_times(o_st, ORACLE_RATE_HZ, 0, [tt])[0], 2),
                    "model_st": round(at_times(m_st, ORACLE_RATE_HZ, 0, [tt])[0], 2),
                    "native_fw_st": round(at_times(series["fw"], 48_000.0, 0, [tt])[0], 2),
                    "native_lin_st": round(at_times(series["lin"], 48_000.0, 0, [tt])[0], 2),
                })  # fmt: skip
            print(prog.stem, "ABC"[layer], "amount", steps, "word", round(word, 4), file=sys.stderr)
    write_csv(Path(ns.out), CSV_TRACE, rows_out)
    return 0


VEL_KEYS = (0, 16, 32, 64, 98, 127)


def velocity(ns: argparse.Namespace) -> int:  # pragma: no cover - needs local firmware images
    """Pitch of layer A against the key byte `g`, `F + 6` held at 1.0, for one capture.

    Needs a capture of a program with `kOscEnvVel` on (a factory program with the bit patched in
    the `.ns4p`); `--captures` is that gz file.
    """
    import arm_dsp_capture as cap
    import synth_e2e_oracle as eo

    rows = list(cap.read_rows(Path(ns.captures)))
    work = Path(ns.scratch) / "work_velocity"
    work.mkdir(parents=True, exist_ok=True)
    (work / "load.script").write_text(eo.core2_load_script(rows))
    (work / "adr.script").write_text("")
    o = Oracle(ns.sharc_sys, ns.image, work, Path(ns.scratch))
    slow = {OSC_ADR + i: SLOWEST_RATE for i in range(3)}
    rows_out, base = [], 0.0
    for g in VEL_KEYS:
        (work / "key.script").write_text(f"{KEY_ON_PASS} {WORD_KEY:#x} {key_word(g):#x}\n")
        a = o.run(slow, [(OSC_LEVEL, f2w(1.0))], 4200, ((OSC_SAMPLES, 8),))
        f = pitch_peak_hz(a[LAW_HOLD[0] : LAW_HOLD[1], 0:8:2].reshape(-1), ORACLE_RATE_HZ)
        base = base or f
        rows_out.append({"g": g, "hz": round(f, 2), "oracle_st": round(semitones(f, base), 3)})
    full = rows_out[-1]["oracle_st"]
    for r in rows_out:
        r["model_st"] = round(full * vel_factor(r["g"]) / vel_factor(VEL_KEYS[-1]), 3)
    write_csv(Path(ns.out), ("g", "hz", "oracle_st", "model_st"), rows_out)
    return 0


def layers(ns: argparse.Namespace) -> int:  # pragma: no cover - needs local firmware images
    """Held-level law of every layer with an amount: the probe word and the program's own word."""
    table = pitch_table(Path(ns.root))
    rows_out = []
    for prog in sorted(Path(ns.programs).glob("*.ns4p")):
        o = Oracle(ns.sharc_sys, ns.image, program_work(ns, prog), Path(ns.scratch))
        for layer, f in env_layers(prog):
            word = dsp_word(float(f["osc_env_amt"]), table)
            base = o.pitch(0.0, 1.0, layer=layer)
            for level in HOLD_LEVELS:
                probe = semitones(o.pitch(PROBE_WORD, level, layer=layer), base)
                own = semitones(o.pitch(word, level, layer=layer), base)
                rows_out.append({
                    "program": prog.stem, "layer": "ABC"[layer], "amount": f["osc_env_amt"],
                    "word": round(word, 5), "level": level, "probe_word": PROBE_WORD,
                    "oracle_probe_st": round(probe, 3),
                    "model_probe_st": round(model_semitones(PROBE_WORD, level), 3),
                    "oracle_own_st": round(own, 3),
                    "model_own_st": round(model_semitones(word, level), 3),
                })  # fmt: skip
                print(rows_out[-1], file=sys.stderr)
    write_csv(Path(ns.out), CSV_LAYERS, rows_out)
    return 0


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - CLI wiring
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("law")
    for a in ("--sharc-sys", "--image", "--work", "--out"):
        p.add_argument(a, required=True)
    p.add_argument("--scratch", default="target/synth_osc_env_pitch")
    for name in ("trace", "layers"):
        q = sub.add_parser(name)
        for a in ("--sharc-sys", "--image", "--programs", "--captures", "--out"):
            q.add_argument(a, required=True)
        q.add_argument("--native", default="")
        q.add_argument("--root", default=".")
        q.add_argument("--scratch", default="target/synth_osc_env_pitch")
    v = sub.add_parser("velocity")
    for a in ("--sharc-sys", "--image", "--captures", "--out"):
        v.add_argument(a, required=True)
    v.add_argument("--scratch", default="target/synth_osc_env_pitch")
    ns = ap.parse_args(argv)
    return {"law": law, "trace": trace, "layers": layers, "velocity": velocity}[ns.cmd](ns)


if __name__ == "__main__":
    sys.exit(main())
