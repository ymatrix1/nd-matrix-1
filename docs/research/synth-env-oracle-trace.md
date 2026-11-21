# Synth envelope and cutoff trace: oracle vs native (audio-remaining row 3)

**Checked:** 2026-11-21.
**Sources:** `tools/synth_env_trace.py` (ARM-word injection, `sharc-sys` taps of the ORG core 2 strip chains `F-20` amp, `F-8` filter, `F+6` osc and of `R = DM(0xC2925)`), `crates/nord-tools/src/bin/synth_env_native.rs` (native chains and `R`), `data/synth-env-trace.csv` (4 programs), `data/synth-cutoff-env-oracle.csv` (34 cutoff / amount / level points), `data/synth-env-velocity-sweep.csv` (90 amp / filter attack points over key velocity), [synth-e2e-oracle-vs-native](synth-e2e-oracle-vs-native.md), [synth-knob-laws](synth-knob-laws.md), [synth-velocity](synth-velocity.md), [arm-tick-timers](arm-tick-timers.md), D130, D132, D140, D194.
**Question:** how do the envelope, LFO and input gain add into the synth cutoff, and which tick (ARM 1 kHz or 24 kHz, or the DSP pass) drives them? Does the native chain (envelope chains plus `2^(amount * 8 * level)`) reproduce the oracle?

## 1. Method

The recall stream (`arm_dsp_capture.py`) carries no ADR, cutoff, resonance or env-amount word: the ARM edit path writes them. The tool therefore injects the words the VERIFIED knob laws produce (`fw_knob_laws.rs`: rate word `(knob - 64) << 24`, cutoff common `(knob - 52) << 24`, env amount `knob / 127` float at `DM(0xE02F5)`) at pass 0, presses a key at pass 2000 and releases it after the hold, and reads the three chains and `R` every pass. One run per program, key velocity byte `0x80 + g`.

## 2. Tick (answered)

The envelope, the strip smoothing (tau about 256 passes) and the LFO are core 2 DSP pass driven: one record step per pass (2 SY frames, 43,402.34 Hz frame rate), coefficients rewritten every 16th pass by the installer. No ARM tick (1 kHz or 24 kHz) enters the chains. The ARM tick only times the key-on words (D130).

## 3. Cutoff composition (VERIFIED, oracle)

```text
fc0 = 13.75 Hz * 2^(cutoff_knob / 12)        knobs 0..117, 17 base points within 0.6 % in R
fc  = fc0 * 2^(amount_knob * level / 12)     1 semitone per amount knob step at full level, 8 points within 0.5 %
R   = min(tan(pi fc / 2F), 1.99)             2F = 86,804.69 Hz (4 samples per pass of 2 frames)
```

`fc0` agrees with the panel table within display rounding (5 %), which also supports pass = 2 frames (OQ22). The envelope adds in semitones of the stage coefficient: 127 semitones (10.6 octaves) at full Filter Env Amt, against the native default `2^(amount * 8 * level)` = 96 semitones. R saturates at 1.99 (`0x3FFEB852`).

## 4. Per program (data/synth-env-trace.csv)

| Program | chain | attack index word / oracle / native | release index word / oracle / native | level at key off oracle / native |
|---|---|---|---|---|
| 12dB Sweep | amp | -64 / -48 / -64 | -11 / -10 / -16 | 0.95 / 1.00 |
| 12dB Sweep | filter | -1 / 0 / -5 | 22 / 22 / 22 | 0.53 / 0.72 |
| 80s Sci-Fi | amp | -64 / -48 / -64 | -16 / -16 / -16 | 0.95 / 1.00 |
| 80s Sci-Fi | filter | -64 / -64 / -64 | -20 / -21 / -21 | 0.50 / 0.21 |
| Crunch Synth | amp | -64 / -48 / -64 | -34 / -32 / -37 | 0.95 / 1.00 |
| Crunch Synth | filter | -32 / -32 / -32 | -39 / -37 / -42 | 0.50 / 0.39 |
| Smooth Lead | amp | -42 / -32 / -42 | -7 / -5 / -10 | 0.95 / 1.00 |
| Smooth Lead | filter | -29 / -37 / -35 | -14 / -16 / -16 | 0.94 / 0.93 |

Cutoff in Hz (before key / max / end of hold / released), oracle against native:

| Program | oracle | native |
|---|---|---|
| 12dB Sweep (knob 41, amt 74) | 147 / 1401 / 1401 / 934 | 147 / 1527 / 1527 / 1002 |
| Crunch Synth (66, 53) | 622 / 13290 / 2877 / 622 | 622 / 6292 / 1551 / 622 |
| Smooth Lead (57, 31) | 370 / 2218 / 1980 / 453 | 370 / 1432 / 1303 / 412 |
| 80s Sci-Fi (81, 41) | 14744 / 18990 / 16085 / 12836 | 1500 / 8986 / 2197 / 1515 |

Findings:

- **Amp attack** reads `floor(word * G / 128)` for negative words (`G` = `g`, 128 for `g = 127`): index -49 for word -64 and -33 for -42 at the default key velocity (`g` = 98). VERIFIED 256 of 256 rows, keys 36 / 60 / 84 ([synth-env-law](synth-env-law.md), D207; D200 had the plateau accuracy guess `word * g / 127`). The release and the filter chains are unscaled. Native default uses the raw word; opt-in `--synth-fw-adr-law`.
- **Sustain**: the oracle decay stops at level 0.5 (the image default of the decay record, VERIFIED on the chain output, [synth-env-law](synth-env-law.md) s3) for the filter chain and the amp chain; native uses sustain 0 (filter) and 1.0 (amp Hold). Hold droops 4.6 % over 0.5 s. Opt-in `--synth-fw-adr-law` stops both chains at 0.5.
- **Cutoff sweep**: the native law sweeps fewer semitones at full level (amount knob x 1 against knob x 0.756 in the default 96-semitone law). Crunch Synth reaches 13.3 kHz in the oracle, 6.3 kHz native; Smooth Lead 2.2 kHz against 1.4 kHz. Both cap: oracle at R 1.99, native at 0.48 fs.
- **80s Sci-Fi** is an outlier: the oracle `R` before key on is already 0.54 (14.7 kHz) where the law predicts 1.5 kHz for knob 81. Another term sits in front of `R` in that program (Ladder24 type, a second strip or an LFO offset); open.
- LFO and input gain into the cutoff are not measured: the injected program leaves the LFO idle and the recall stream has no LFO depth word.

## 5. Implications

- Opt-in `--synth-fw-env-law` (`synth/fw_cutoff_env.rs`, `SynthLayer::fw_env_law`) applies the VERIFIED semitone composition to the layer cutoff. Default unchanged (D200).
- The amp attack velocity scaling and the 0.5 decay sustain are implemented opt-in as `--synth-fw-adr-law` (D207, [synth-env-law](synth-env-law.md)); the same note finds that the default attack and decay tables are a stale sweep.

## 6. Follow-ups

- ~~Key-note dependence of the amp attack scaling (k36 / k84)~~: none ([synth-env-law](synth-env-law.md) s2).
- LFO depth and input gain terms into `R`; the 80s Sci-Fi base offset.
- Score `--synth-fw-env-law` on the hardware clips before any default flip (owner's call).
