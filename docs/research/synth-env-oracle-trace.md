# Synth envelope and cutoff trace: oracle vs native (audio-remaining row 3)

**Checked:** 2026-11-21 (D200); extended 2026-11-21 (D207: LFO, input terms, 80s Sci-Fi).
**Sources:** `tools/synth_cutoff_terms.py`, `tools/synth_lfo_words.py` (D207, section 7), `data/synth-cutoff-terms-oracle.csv`, `data/synth-lfo-arm-words.csv`, `data/synth-env-trace-index.csv`, `tools/synth_env_trace.py` (ARM-word injection, `sharc-sys` taps of the ORG core 2 strip chains `F-20` amp, `F-8` filter, `F+6` osc and of `R = DM(0xC2925)`), `crates/nord-tools/src/bin/synth_env_native.rs` (native chains and `R`), `data/synth-env-trace.csv` (4 programs), `data/synth-cutoff-env-oracle.csv` (34 cutoff / amount / level points), `data/synth-env-velocity-sweep.csv` (90 amp / filter attack points over key velocity), [synth-e2e-oracle-vs-native](synth-e2e-oracle-vs-native.md), [synth-knob-laws](synth-knob-laws.md), [synth-velocity](synth-velocity.md), [arm-tick-timers](arm-tick-timers.md), D130, D132, D140, D194.
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

- **Amp attack** reads `floor(word * G / 128)` for negative words (`G` = `g`, 128 for `g = 127`): index -49 for word -64 and -33 for -42 at the default key velocity (`g` = 98). VERIFIED 256 of 256 rows, keys 36 / 60 / 84 ([synth-env-law](synth-env-law.md), D210; D200 had the plateau accuracy guess `word * g / 127`). The release and the filter chains are unscaled. Native default uses the raw word; opt-in `--synth-fw-adr-law`.
- **Sustain**: the oracle decay stops at level 0.5 (the image default of the decay record, VERIFIED on the chain output, [synth-env-law](synth-env-law.md) s3) for the filter chain and the amp chain; native uses sustain 0 (filter) and 1.0 (amp Hold). Hold droops 4.6 % over 0.5 s. Opt-in `--synth-fw-adr-law` stops both chains at 0.5.
- **Cutoff sweep**: the native law sweeps fewer semitones at full level (amount knob x 1 against knob x 0.756 in the default 96-semitone law). Crunch Synth reaches 13.3 kHz in the oracle, 6.3 kHz native; Smooth Lead 2.2 kHz against 1.4 kHz. Both cap: oracle at R 1.99, native at 0.48 fs.
- **80s Sci-Fi** is not an outlier (D207, section 7): its stage is the Ladder24 `0x12549D`, which reads the linear ramp table `R = 0.36083 + 0.0079301 (knob - 52)` as its cutoff position, so the tap `DM(0xC2925)` reads 0.5908 (14.7 kHz by the exponential reading) for knob 81 where the exponential law gives 0.054. Same index sum, other map to `R`; no offset.
- LFO and input terms: measured in section 7 (D207).

## 5. Implications

- Opt-in `--synth-fw-env-law` (`synth/fw_cutoff_env.rs`, `SynthLayer::fw_env_law`) applies the VERIFIED semitone composition to the layer cutoff, since D207 also the LFO term for a cutoff destination. Default unchanged (D200, D207).
- The amp attack velocity scaling and the 0.5 decay sustain are implemented opt-in as `--synth-fw-adr-law` (D210, [synth-env-law](synth-env-law.md)); the same note finds that the default attack and decay tables are a stale sweep.

## 6. Follow-ups

- ~~Key-note dependence of the amp attack scaling (k36 / k84)~~: none ([synth-env-law](synth-env-law.md) s2).
- Done in D207: LFO term, input terms, 80s Sci-Fi base offset (section 7). Open: LFO stage output scale with a running LFO (rate words of PNO core 1), injection into the strip of layers B / C (Phat 3 Osc MiniM), key tracking word `w38` (the oracle runs at unity).
- Score `--synth-fw-env-law` on the hardware clips before any default flip (owner's call).

## 7. Cutoff index sum (D207)

The pass control block of channel 0 (`sharc-disasm IMAGE 0x24C508 0x24C5DC`, pc `0x126284`) adds, per lane, in index units (`2^24` of a Q31 word = one semitone):

```text
idx = w3 + m0 + ((w5 + m1) * lfo << 2) + 2 * w7 * FIX(F-4, 30) + DM(0xC1E31) * DM(F+27)
w3, w5, w7 : strip 0 words 3, 5, 7, smoothed (0.99609375 w + 0.0048828 c) toward common 0 words 1, 3, 5
m0, m1     : DM(0xC2717), DM(0xC2719)   key tracking offset (note - 60 as the ARM sends it), second input
lfo        : DM(0xC1E37)                LFO stage output;  F-4 = DM(0xC1184) = filter env level x amount
```

Every term was injected alone on the 12dB Sweep layer (`tools/synth_cutoff_terms.py measure`, 40 rows, `data/synth-cutoff-terms-oracle.csv`); `oracle` is the index change read from `R`:

| Term | Law | Rows | Max error |
|---|---|---|---|
| LFO | `512 * depth * lfo` semitones, both Q31 fractions (`depth` = common word 3 steady state) | 12 (depth 0.125 .. 0.5, lfo +-0.1, +-0.2) | 0.008 |
| Key offset `m0` | `m0` semitones (-24 .. +12) | 6 | 0.000 |
| Envelope | `128 * w7 * F-4`; `w7 = 0x7F000000` gives `127 * F-4` (D200: 1 semitone per amount knob step) | 6 (`w7` and `F-4` varied) | 0.008 |
| Filter drive | strip word 1 top byte 0, 16, 64, 127: no change of `R` | 4 | 0.000 |
| `DM(0xC1E31) * DM(F+27)` | `DM(F+27) = 1/32` idle, 0 with a voice; `DM(0xC1E31) = 0xFFF97247` constant: under 0.001 semitone | probe | - |

**LFO depth word from the ARM** (`tools/synth_lfo_words.py`, `data/synth-lfo-arm-words.csv`; kLFODst then kLFOAmt, no restore between): destination 3 (`FilterCutoff` in the decoder) writes common word 3 = `amt / 127 * 0x40000000` (amt 32, 64, 127 -> `0x10204080`, `0x20408100`, `0x40000000`); destination 1 writes common word 8 (float `amt / 254`), destination 2 common word 2 (table). So a full-depth cutoff LFO swings `512 * 0.5 * lfo = 256 lfo` semitones for a Q31 LFO word `lfo`, clipped by the table range. **LFO output scale:** the LFO stages (`DM(0xC13A6)` = `0x12539E`, `0x1253AE`, `0x1253BA`) settle at 0.49997 (0.25 for `0x1253BA`) with no rate word, so full scale 0.5 is the largest value seen: `semitones = 128 * amount * lfo_val` for the native `lfo_val` in -1..1 (**ASSUMED** scale; the rate source is PNO core 1, no running LFO was traced). The native default is `1 + 0.8 * amount * lfo_val` on the Hz cutoff: 20 % swing at amount 0.25 against 32 semitones.

**Input gain.** There is none in this sum: the filter drive byte (strip word 1 top byte) does not move `R`, and the only other float input is `F-4`. The nearest "gain" terms are `w7` (envelope depth multiplier, default 127/128, so the 127 semitones of D200 are `128 * w7`) and the LFO depth word.

**80s Sci-Fi base offset (explained).** Stage `0x12549D` (Ladder24) reads the ramp table (`synth_adr_model.cutoff_r`) instead of the exponential map: 4 knobs (30, 60, 81, 100) measured on the 80s program with the stage word forced to `0x12549D` and `0x1253BD`: ladder `R` = 0.186, 0.424, 0.591, 0.742 = ramp at index `knob - 52`; LP12 `R` = 0.0028, 0.0159, 0.0536, 0.162 = the exponential law (8 rows, 0.0 % error). Key offset -12 (the program sends `0xF4` in the note word) subtracts 12 index units in both maps (4 rows). `fw_stage_r_of_knob(knob, ladder)` encodes both. Compared in index (knob semitones), oracle against native (`data/synth-env-trace-index.csv`, 4 programs, `synth_env_native --kb-track 3`, unity because the oracle injects no tracking word):

| Program | stage | before oracle / default / fw law | peak | end of hold | released |
|---|---|---|---|---|---|
| 12dB Sweep | LP12 | 41.0 / 41.0 / 41.0 | 80.1 / 81.5 / 94.6 | 80.1 / 81.5 / 94.6 | 73.0 / 74.2 / 85.0 |
| 80s Sci-Fi | Ladder24 | 81.0 / 81.2 / 81.2 | 110.0 / 100.2 / 110.2 | 89.5 / 75.8 / 78.0 | 69.7 / 69.4 / 69.5 |
| Crunch Synth | LP12 | 66.0 / 66.0 / 66.0 | 119.0 / 106.1 / 119.0 | 92.5 / 81.8 / 86.9 | 66.0 / 66.0 / 66.0 |
| Smooth Lead | LP24 | 57.0 / 57.0 / 57.0 | 88.0 / 80.4 / 88.0 | 86.0 / 78.8 / 85.8 | 60.5 / 58.9 / 59.5 |

The firmware law matches the oracle peak to 0.25 semitone on three of four programs (default law: 7.6 .. 13 semitones low); the end-of-hold and 12dB Sweep rows differ by the chain (oracle decays to sustain 0.5 and attacks slower, section 4), not by the law. **Phat 3 Osc MiniM is excluded:** its recall points channel 0 at the strip `0xE0118` (another layer), so the injection of strip 0 and common block 0 does not reach the tapped lane (R stays at the ramp of `m0` for any cutoff).
