# Synth oscillator level and voice gain: SHARC oracle against native, per wave (D140)

- **Checked:** 2026-11-17
- **Sources (local, never committed):** ORG core 2 image (`tools/ns4_dsp_ldr.py target/DSP/dsp_image_2_ORG.bin --sram DIR`), the 384 ARM captures of [arm-dsp-capture](arm-dsp-capture.md) (key 60 velocity 100), `sharc-sys --tap`, native `synth_e2e_native` (new `--unity`, `--unison N`), new driver `tools/synth_osc_gain.py` (tests `tools/tests/test_synth_osc_gain.py`), `tools/emu_vs_hw.py` with the Part A video (`_zksT69H30c`). Derived numbers only: [data/synth-osc-gain-oracle.csv](data/synth-osc-gain-oracle.csv) (43 programs). No audio, firmware or sample content is stored or printed. Background: [synth-e2e-oracle-vs-native](synth-e2e-oracle-vs-native.md) (D132), [synth-p-writer](synth-p-writer.md) (P words), [synth-p-fit](synth-p-fit.md), [synth-validation](synth-validation.md).
- **Question:** D132 measured the synth voice end to end on five programs and found the oscillator offset at least as large as the filter offset in two of them, with unison, layer count and envelope as confounds. Per wave type, how far is the native oscillator level from the oracle oscillator stage with the voice count fixed, what is the voice gain behind the stage, and which differences are native constants that are wrong and which are oracle inputs that are incomplete?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Is the native oscillator stage level right? | **Yes, for every wave that was cleanly measurable.** Native oscillator times the fixed voice constant `0.25` (`render.rs` `env_gain`) equals the ORG core 2 oscillator stage (`P[16] = 0.25`) at table row gain 1.0: needed row gain `1.006` (PURE saw, 2 of 3 programs), `0.999` (noise), `1.006 x 3` (SHAPE square), `1.001 .. 0.994` (SHAPE triangle, 3 programs), `1.001` (PURE sine), `0.999 / 1.003` (FM; the third FM program, `Necklace Upright`, needs `0.956` = -0.4 dB, f0 1046.5 Hz oracle against 523.3 Hz native, an octave mismatch), `0.70 .. 0.75` (pulse, = `0.175 / 0.25`, the words), `3.9 .. 4.2` (MULTI) and `4.6 .. 4.9` (SUPER). Spread inside a wave: under 0.1 dB on noise, SHAPE square, SHAPE triangle, saw (2 of 3) and FM (2 of 3). The voice constant `0.25` is the P gain, not a free fit | VERIFIED (43 programs with sound of 46 run, `tools/synth_osc_gain.py`) |
| What is the voice gain behind the stage? | The oracle path has no stage after the oscillator on this stream (D132 s4: no mix or output stage installed), so the **absolute** native-to-hardware level is not in the oracle. The saw row `0.4` (hardware fit, [synth-p-fit](synth-p-fit.md)) divided by the measured `1.0` row gain is `0.4` = `GAIN_PER_P16 x 0.25`: `-7.96 dB` between the oracle stage and the engine. Pulse (fitted `0.27` against `0.28`) and noise (`0.4`) agree with it independently | EMPIRICAL (one anchor, two cross-checks) |
| Which default rows disagree with that voice gain? | `Other` (PURE sine / triangle, SUB OSC, SYNC, MISC), `Fm` and SHAPE square sit at the idle gain `1.0`, **+8.0 dB** above the saw-anchored level (ASSUMED rows, never fitted). SHAPE triangle `0.6` (+3.5 dB, fitted with the cutoff factor on one clip), MULTI `3.8` (+7.4 dB), SUPER `2.4` (+2.0 dB), SHAPE SINE `0.5` (+0.5 .. +5 dB by program) are fitted rows that disagree too, but see the clip numbers: they compensate other errors | VERIFIED (measurement), EMPIRICAL (anchor) |
| Unison and layer confound | **Layer count: none.** Oracle channel 0 is layer A; programs with 2 and 3 layers give the same needed row gain as single layer ones (pulse `0.70` on 1, 2 and 3 layers; saw `1.003 / 1.006`). **Unison: real and large on the native side.** Relative to the same wave at unison 0, the oracle lane level is `+1.4 / -1.6 dB` (saw, unison 1 / 3), `+0.8 / +1.7 / -1.8` (SHAPE triangle, 1 / 2 / 3), `+0.5 / -2.0 / -2.1` (SHAPE square, 2 / 3 / 3); native analog unison sums constant-power voices and falls `4.1 .. 6.6 dB` **below** its single voice on saw, SHAPE triangle and SHAPE saw (SHAPE square ignores unison). Native unison is `+2.4 .. +8.3 dB` too low against the oracle lane on those three (section 4). The D132 `Crunch Synth` `+11.0 dB` is mostly this (`+5.9`) plus the row (`+3.5`) | VERIFIED (measurement) |
| Band shape before the filter | Level-normalised third-octave mean difference, native default against the oracle oscillator path: noise `1.1`, sine `0.6`, SHAPE square `1.6`, pulse `0.4 .. 3.8`, saw `1.4 .. 5.8`, MULTI `2.9 .. 3.7`, SUPER `4.1 .. 8.7`, SHAPE triangle `4.4 .. 12`, SHAPE saw `3.2 .. 21`, SHAPE SINE `3.5 .. 49`, FM `22 .. 44` dB. The waves with a stage in native and a knob that is mapped match to under 4 dB; the SHAPE saw / SHAPE SINE / FM shapes differ by the shape word the native knob law supplies and by the FM stage (opt-in in native) | VERIFIED (measurement), cause ASSUMED |
| Which native constants are wrong? | Rows `Other`, `Fm`, SHAPE square (idle `1.0` against `0.4`): fix behind `--synth-p oracle`. Only SHAPE square has a hardware clip, and it improves in level (section 6). Unison level law: wrong against the oracle, **not fixed here** because the only clip (Crunch) goes the other way | VERIFIED + decision |
| What is incomplete on the oracle side? | The ARM stream has no envelope words (D132), no output stage, and three of the 49 programs run silent in the oracle (`FM Harpies`, `Scream Lead AT`, SUPER 1..4, `Shoreline Whl`, noise: the key lane does not start; cause not diagnosed). WAVE (wavetable / sample) is not measurable here | open |

## 2. Method

1. **Programs.** From the 384 captures, those with ORG core 2 key words for key 60 and an enabled ANALOG or FM layer A: 46 runs, 43 with sound. Layer A only; unison 0 in the file for the baseline of each wave (31 programs), unison 1 .. 3 for the confound (12 programs). Waves covered (`PWave` rows): PURE saw, pulse, noise, sine ("other"), SHAPE triangle, saw, square, SHAPE SINE, MULTI, SUPER wave 0, FM (3 programs with sound).
2. **Oracle.** `tools/synth_osc_gain.py run`: `sharc-sys` on the ORG core 2 image with the recall of the capture and the key words, null B stage poked before every pass (oscillator path only, one run per program, samples cached); channel 0 PEx lane, 4 samples per pass at 86,804.69 Hz (ASSUMED rate, D132), steady RMS from 0.15 s after note on to 0.05 s before note off.
3. **Native.** `synth_e2e_native --no-filter` on layer A with five variants: `default` (program envelope, velocity 100), `unity` (`--unity`: amp velocity 0, attack 1 ms, sustain 1), `verified` (`--unity --synth-stages --synth-p verified`), `g1` (`--unity` and the row of the program's wave at gain 1) and `g1u0` (`g1` with unison 0). `needed_row_gain = 10^((oracle - native_g1) / 20)`: the row gain at which native lands on the oracle. This isolates the oscillator stage from velocity, envelope and the fitted gain.
4. **Clips.** `tools/emu_vs_hw.py` (emulator minus hardware) with `--synth-p oracle` against the default on the 11 synth segments and `pianopad_grand`.

```
uv run python tools/synth_osc_gain.py run CAPTURE.txt.gz PROGRAM.ns4p --sharc-sys target/release/sharc-sys \
    --image DIR/core2_sram.bin --native target/release/synth_e2e_native --workdir target/og > row.json
uv run python tools/synth_osc_gain.py csv row*.json > docs/research/data/synth-osc-gain-oracle.csv
```

## 3. Per wave (unison 0 in the file)

Needed row gain = row gain at which the native oscillator path (filter off, unity amp, times 0.25) equals the oracle stage. `Default row` is the built-in `PTable::FITTED` gain. `Row vs 0.4 x needed` is the row against the saw-anchored level (`D = 0.4`, EMPIRICAL): positive means the native row is louder than the oracle implies.

| Row | Programs (unison 0) | Needed row gain (min / median / max) | Default row | Row vs `0.4 x needed` |
|---|---|---|---|---|
| PURE saw | 3 | 1.003 / 1.006 / 1.472 (`J3 Open Sky` 3 layers, pitch 263.6 Hz native: outlier) | 0.4 | -0.1 dB |
| PURE pulse | 3 | 0.700 / 0.700 / 0.753 | 0.27 | -0.3 dB |
| PURE noise | 1 (`Shoreline Whl` silent) | 0.999 | 0.4 | 0.0 dB |
| ANALOG other (sine) | 1 | 1.001 | 1.0 | **+8.0 dB** |
| FM | 3 | 0.956 / 0.999 / 1.003 (level only: FM band delta 22 to 44 dB) | 1.0 | **+8.0 dB** |
| SHAPE triangle | 3 | 0.994 / 1.000 / 1.001 | 0.6 | +3.5 dB |
| SHAPE saw | 3 | 0.624 / 0.667 / 1.072 (shape knob dependent) | 0.4 | +3.5 dB (median) |
| SHAPE square | 3 | 1.006 / 1.006 / 1.006 | 1.0 | **+7.9 dB** |
| SHAPE SINE | 6 | 0.676 / 0.743 / 1.159 (knob dependent) | 0.5 | +4.5 dB (median) |
| MULTI | 3 | 3.914 / 4.047 / 4.249 | 3.8 | +7.4 dB (words stage reads `P[16]`, verified row 1.6) |
| SUPER 0 | 2 | 4.591 / 4.755 / 4.918 | 2.4 | +2.0 dB (verified row 1.6) |

Reading:

- Seven single-oscillator families land on `1.00 +- 0.01` (saw, noise, sine, SHAPE triangle, SHAPE square, two of three FM) or on the P ratio of their words (pulse `0.175 / 0.25 = 0.70`, MULTI `1.6 / 0.4 = 4.0` within 2.2 dB, SUPER `4.6 .. 4.9 x 0.4 = 1.84 .. 1.97` against the verified `1.6`, +1.2 .. +1.8 dB): **the oscillator stages and the voice constant are the oracle's**. D132's `+0.3 .. +11.4 dB` oscillator offsets were this row structure plus the velocity / envelope / unison terms, not a stage error.
- The voice constant `0.25` in `render.rs` is the single-oscillator `P[16]`. For MULTI and SUPER the words stages carry their own `P[16]` and the row gain is `GAIN_PER_P16` (`p_words.rs`).
- SHAPE saw and SHAPE SINE vary with the program by `-4 .. +0.6` and `-3 .. +1.3` dB at one row gain: the shape word the native knob law builds differs from the word the ARM sent (band delta `3.2 .. 49` dB). That is a knob-to-word question (OQ), not a gain constant; no row is proposed.

## 4. Unison (file value 1 .. 3, native single row)

`Oracle` = oracle level against the unison 0 median of the same wave; `native` = native layer at the file unison against native unison 0 (same row); `gap` = oracle minus native, i.e. how far native is **too low**.

| Wave | Unison | Programs | Oracle (dB) | Native (dB) | Gap (dB) |
|---|---|---|---|---|---|
| saw | 1 / 3 | `Daft Lead` / `OB Fanfare`, `Phil Pad`, `Saw Horn` | +1.4 / -1.6 | -4.6 / -4.1 | +6.0 / +2.5 |
| SHAPE triangle | 1 / 2 / 3 | `Crunch Synth` / `Slow Motion Pad` / `Ballad Layer Whl` | +0.8 / +1.7 / -1.8 | -5.1 / -6.6 / -6.2 | +5.9 / +8.3 / +4.4 |
| SHAPE saw | 1 / 2 | `Replica Lead` / `J3 Warm 80s Brs` | +1.4 / +1.2 | -4.9 / -5.7 | +6.3 / +6.9 |
| SHAPE square | 2 / 3 | `Analog PWM` / `12dB Sweep`, `PWM Strings` | +0.5 / -2.0, -2.1 | 0 / 0 | +0.5 / -2.0, -2.1 |

The oracle changes the lane level by `+- 2 dB` with the unison setting (the `a` / `b` words are a lane rotation and a ratio pair, [synth-p-writer](synth-p-writer.md) s4: a level law of its own, `+1 dB` at 1 and 2, `-1.8 dB` at 3). Native analog unison is a constant-power sum of detuned voices whose sum is `4 .. 7 dB` below one voice. **Matching the voice count** therefore means: oracle lane at the file unison against native **unison 0**, which is what `g1u0` and `needed_row_gain_u0` give (`Crunch Synth` shape `1.099`, i.e. `+0.8 dB` against unison 0). No hardware stat of the real unison level exists: the unison clips are the four synth segments with unison > 0 (`synth_12db` 3, `synth_crunch` 1, `synth_phat3` 1, `synth_leaky` 3).

## 5. What the oracle cannot give

| Item | State |
|---|---|
| Absolute voice gain behind the stage (`D = 0.4`) | Not on the ORG stream (no output stage, D132 s4). The saw row `0.4` is a hardware fit (one clip pair, [synth-p-fit](synth-p-fit.md)); pulse `0.27` and noise `0.4` agree independently. EMPIRICAL |
| Envelope, amp and filter-envelope words | Not in the recall; oracle envelope is the image default (D132 s5). The `unity` variant removes the native envelope, so the comparison here does not depend on it |
| Velocity | Key velocity 100 in the key word; the oracle oscillator path has no amp stage, native `--unity` removes the velocity blend. The amp velocity law (`fw_velocity.rs`) is not tested here |
| Silent oracle programs | `FM Harpies`, `Scream Lead AT` (SUPER 1..4 stage), `Shoreline Whl` (noise): the pass produces no samples for key 60; not diagnosed. SUPER waves 1 .. 4 therefore have no level here |
| WAVE (wavetable), SAMPLE | Not measurable: sample data is not on the ORG stream. `PWave::Sampled` keeps the idle row |
| SYNC, SUB OSC, PURE triangle | No single-layer program with a key word in the capture set that plays them; the `Other` row rests on one sine program plus the words (`P[16] = 0.25`; SYNC pulse types 30, 31: `0.175 / 0.2`) |

## 6. Hardware clips (numbers, `tools/emu_vs_hw.py`, emulator minus hardware RMS dB, band score mean |delta| dB, lower is better)

`--synth-p oracle` (= built-in table with `ShapeSquare`, `Other`, `Fm` rows at `0.4`) against the default:

| Segment | Rows hit | RMS default -> oracle | Score default -> oracle |
|---|---|---|---|
| `synth_12db` (A:57, SHAPE square x2 + saw, LP12) | SHAPE square | **+6.65 -> +0.21** | 1.39 -> 1.50 (presence -0.8 -> -1.3, brilliance -1.1 -> -2.0) |
| `synth_lead` | none | -3.66 -> -3.66 | 0.80 -> 0.80 |
| `synth_crunch` | none | +2.04 -> +2.04 | 0.89 -> 0.89 |
| `synth_filterbank` | none | -1.35 -> -1.35 | 5.39 -> 5.39 |
| `synth_phat3` | none | -3.86 -> -3.86 | 2.38 -> 2.38 |
| `synth_leaky` | none | -5.15 -> -5.15 | 1.95 -> 1.95 |
| `synth_sineplk` | none | +1.51 -> +1.51 | 4.73 -> 4.73 |
| `synth_silkpad` | none | -0.55 -> -0.55 | 1.64 -> 1.64 |
| `synth_horizon` | none | -0.18 -> -0.18 | 1.89 -> 1.89 |
| `synth_megaline` | none | -7.63 -> -7.63 | 2.00 -> 2.00 |
| `synth_80s` | none | +5.02 -> +5.02 | 0.90 -> 0.90 |
| `pianopad_grand` | none | +14.27 -> +14.27 | 2.19 -> 2.19 |

Default output: `render_hash` on the merged tree (default flags, five segments) has SHA-256 `46f4dc64...da506`, equal to `WHOLE` in `tests/golden_render.rs` (test passes); the new rows are idle by default, so the default render does not move.

Rows fitted to hardware that the oracle level would move (`--synth-p verified`, which sets the rows to `1.6 x P[16] x weight`, D81 opt-in), same clips, RMS default -> verified and score:

| Segment | Row | RMS | Score |
|---|---|---|---|
| `synth_lead` | SHAPE triangle 0.6 -> 0.4 | -3.66 -> -7.18 | 0.80 -> 0.80 |
| `synth_crunch` | SHAPE triangle (unison 1) | +2.04 -> -1.37 | 0.89 -> 0.89 |
| `synth_80s` | MULTI 3.8 -> 1.6 | +5.02 -> -3.53 | 0.90 -> 1.46 |
| `synth_megaline` | MULTI + SUPER | -7.63 -> -13.81 | 2.00 -> 3.34 |
| `synth_silkpad` | SUPER 2.4 -> 1.6 | -0.55 -> -4.07 | 1.64 -> 1.64 |
| `synth_horizon` | SUPER | -0.18 -> -3.74 | 1.89 -> 1.68 |
| `synth_sineplk` | SHAPE SINE 0.5 -> 0.4 | +1.51 -> -0.43 | 4.73 -> 4.73 |
| `synth_12db` | SHAPE square | +6.65 -> +0.21 | 1.39 -> 1.50 |

The verified rows worsen 5 of 8 RMS errors: the fitted rows MULTI / SUPER / SHAPE triangle carry about `+2 .. +7 dB` that the oracle stage does not have. The native filter output is about `+5 dB` under the oracle on four of five programs (D132, `cin` hypothesis open), and those fits were made behind that filter: the extra row gain and the missing filter gain look like one error (ASSUMED, not tested: a native filter gain fix and the verified rows together is the test, follow-up 1).

## 7. Decision per finding

| Finding | Native or oracle | Action |
|---|---|---|
| Idle rows `Other`, `Fm`, SHAPE square at 1.0 (+8 dB) | native constant | `--synth-p oracle`: rows at `GAIN_PER_P16 x 0.25 = 0.4`; `PWave::Fm` / `PWave::Sampled` split from `Other` so the sample path stays idle. Clip: `synth_12db` RMS +6.65 -> +0.21 dB, score +0.11, nothing else moves. Not default |
| SHAPE triangle 0.6, MULTI 3.8, SUPER 2.4 above the oracle level | native fits, confounded with filter gain | none; clips worsen with the verified rows |
| Native analog unison level 4 .. 7 dB under its single voice, oracle +1 .. -2 dB | native model | not fixed: the one hold-out clip (`synth_crunch`, unison 1) is already `+2.0 dB` over hardware and a fix would raise it by about 6 dB. Needs the oracle lane rotation (P words `a`, `b`) in the stage, follow-up 2 |
| SHAPE saw / SHAPE SINE level spread with the program | native knob-to-word | none; follow-up 3 |
| Envelope, velocity, output stage | oracle input incomplete | send the program's `Edt_Syn` envelope edits (follow-up 4 of D132); install a mix stage for the absolute gain |

## Implications

- The oscillator stages and the voice constant are right to 0.1 dB on the clean programs (saw 2 of 3, FM 2 of 3; outliers `J3 Open Sky` +3.4 dB, `Necklace Upright` -0.4 dB); D132's "oscillator offset at least as large as the filter offset" was fitted row gains and unison, not a stage error. Neither stage model needs a level change.
- One opt-in fix with a clip number: `--synth-p oracle` (3 idle rows). The owner's rule (D136 proposed) would flip it on RMS (`synth_12db` error `6.65 -> 0.21 dB`); the band score of that clip is `0.11` worse, inside the noise of one lossy clip. No default changed here.
- The fitted MULTI / SUPER / SHAPE triangle rows and the native filter gain are one coupled problem; flipping either alone worsens clips.

## Follow-ups

1. Test the coupling: native filter input gain `1.98` (`cin`, D132 follow-up 2) with the verified rows `1.6 x P[16]`, re-run `synth_80s`, `synth_megaline`, `synth_silkpad`, `synth_horizon`.
2. Port the unison words (`a`, `b`: lane rotation `cos / sin(pi / 8)`, ratio pairs) into the native analog stages and compare the lane level per unison setting with section 4.
3. SHAPE saw / SHAPE SINE: feed the oracle's shape word to native (D132 follow-up 3), re-measure the `0.62 .. 1.07` spread.
4. Diagnose the silent oracle programs (SUPER 1 .. 4, noise `Shoreline Whl`, `FM Harpies`) and measure SUPER 1 .. 4, SYNC, SUB OSC and PURE triangle on programs that play them.
5. GUI: add `Oracle` to the `P_TABLES` list when the owner decides.
