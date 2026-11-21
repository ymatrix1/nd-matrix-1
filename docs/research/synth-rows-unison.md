# Synth rows and the unison default flip: why the fitted SHAPE triangle, MULTI and SUPER rows are coupled to the filter gain (D189)

- **Checked:** 2026-11-21
- **Sources (local, never committed):** ORG core 2 image (`tools/ns4_dsp_ldr.py target/DSP/dsp_image_2_ORG.bin --sram DIR`), OS v1.66 ARM firmware through `tools/arm_dsp_capture.py` (recall plus key 60 velocity 100 of 18 factory programs, [arm-dsp-capture](arm-dsp-capture.md)), `sharc-sys`, native `synth_e2e_native`, `tools/synth_e2e_oracle.py report` (D132 pipeline, unchanged), `tools/emu_vs_hw.py` with the Part A video (`_zksT69H30c`) and the factory backup, new `synth_census` (384 factory programs). Derived numbers only: [data/synth-filter-gain-oracle.csv](data/synth-filter-gain-oracle.csv) (18 programs), [data/synth-rows-unison-clips.csv](data/synth-rows-unison-clips.csv) (16 clips), [data/synth-rows-unison-census.csv](data/synth-rows-unison-census.csv) (351 programs with a synth layer). Background: [synth-osc-gain-oracle](synth-osc-gain-oracle.md) (D140), [synth-unison-level](synth-unison-level.md) (D178), [synth-e2e-oracle-vs-native](synth-e2e-oracle-vs-native.md) (D132), [synth-filter-t3](synth-filter-t3.md) (D177), [synth-sineplk-drift](synth-sineplk-drift.md) (D179).
- **Question:** (1) Can the fitted rows SHAPE triangle `0.6`, MULTI `3.8` and SUPER `2.4` be replaced by oracle-derived values, or why are they coupled to the filter gain? (2) With those rows fixed, do the firmware unison lane weights (`--synth-unison-weights`, D178) improve the 16 synth clips and the 384 factory programs? (3) What is the evidence for a default flip, for the owner to decide?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Why are the three rows `+3.5 / +7.4 / +2.0 dB` above the oracle level? | They are the oracle row times the filter gain the native filter does not have. The oracle filter (ORG core 2, real ARM stream) reads **`+2.7 .. +6.7 dB` above the native filter on 14 of 18 programs** (LP12 `+4.6` mean, LP24 `+5.3`, two of four Ladder24 `+5.3 / +3.1`), the band pass `0.0`, the other two Ladder24 programs `-2.2 / -1.6`, one resonance 0.95 outlier `+15.2`. The three rows were fitted on LP24 (Smooth Lead), Ladder24 (80s Sci-Fi) and LP12 (Silk Pad) clips, the saw anchor (`0.4`) on a band pass and a wide-open Ladder24 clip, where the gap is about `0 dB`. The row excess (`+4.3 dB` mean) and the filter gap (`+5.0 dB` mean on those types) agree in sign and size; the per row residual is `+- 3 dB` | VERIFIED (18 oracle runs), the attribution EMPIRICAL |
| Can the rows be replaced by oracle values alone? | **No.** At the oracle level (`0.4`, `1.6`, `1.9`) the clips lose level (`synth_lead -3.7 -> -7.2`, `synth_80s` flips from `+5.4` to `-3.1`, `synth_megaline -0.9 -> -7.4` dB RMS): the filter gain has to ride with them. `--synth-p verified` (the old attempt) worsened 5 of 8 clips for the same reason (D140) | VERIFIED (clips) |
| What is the replacement? | **One free factor instead of three fitted rows**: `--synth-p coupled` = oracle rows (`0.4 x 1.0` SHAPE triangle, `0.4 x 4.047` MULTI, `0.4 x 4.755` SUPER) times `sqrt 2` (`+3.0 dB`, `COUPLED_FILTER_GAIN`), plus the D140 idle rows. The ratios between the rows are the oracle's; the factor is swept on the clips (section 4): `sqrt 2` minimises the mean absolute RMS error. Rows `0.566 / 2.263 / 2.687` against the fitted `0.6 / 3.8 / 2.4` | EMPIRICAL (one factor), VERIFIED (ratios) |
| Unison weights on top | `--synth-unison-weights` (D178) is better than the default on the 16 clips (mean score `2.784 -> 2.747`, mean abs RMS `4.09 -> 3.89 dB`) and better than the fitted rows with it; with the coupled rows (`--synth-oracle`) the means are `2.714` / `3.41` dB. It moves 21 of 351 synth programs by more than 1 dB (mean `+4.3 dB`, all on analog unison layers) | VERIFIED (run) |
| Is a flip justified? | **Not by the clip score.** The clip readings are mixed (section 6): the mean score gain (`0.07`) is within the noise of one lossy clip, and `synth_crunch` (SHAPE triangle, unison 1) goes `+0.5 -> +2.8 dB` loud. The level evidence (mean abs RMS error `4.09 -> 3.41 dB`, `synth_80s` `+5.4 -> 0.0`, `synth_12db` `+6.6 -> +0.2`) supports it under the owner's D136 rule (RMS, not band score). Recommendation and the risks are in section 6; **no default changed here** | proposal |

## 2. Method

1. **Filter gain per program.** `tools/synth_e2e_oracle.py report` (D132 method, unchanged) on 18 programs whose layer A is a SHAPE triangle, MULTI, SUPER, saw or SHAPE saw voice, one run per program: ARM stream to ORG core 2 in `sharc-sys`, filter stage on and null stage, native `synth_e2e_native` with and without the filter. The gap is `(oracle filtered / unfiltered RMS) - (native filtered / unfiltered RMS)` in dB: the ratio form cancels the oscillator level, the envelope and the voice gain, so it isolates the filter. Cutoff, resonance and type come from the program recall; the oracle's filter envelope is the image default (D132 s5), which makes the gap a property of the filter plus that confound. Driver wrapper only (`scratchpad`), no tool change; the 18 captures took 70 s of ARM CPU each.
2. **Clips.** `tools/emu_vs_hw.py` on the 16 Part A synth segments of [data/emu_vs_hw_segments.json](data/emu_vs_hw_segments.json) (`synth_lead`, `synth_crunch`, `synth_silkpad`, `synth_filterbank`, `synth_sineplk`, `synth_megaline`, `synth_horizon`, the four `super_lp24_*` clips, `synth_12db`, `synth_phat3`, `synth_80s`, `synth_leaky`, `pianopad_grand`; D121's twelve plus the four SUPER LP24 fit clips). Variants: `default`, `unison-weights`, a sweep of the oracle rows times `G = 1.0 / 1.414 / 1.778 / 2.0` with and without the weights (`--synth-p table:shape=...;multi=...;super=...`), then the real flags `--synth-p coupled` and `--synth-oracle`. The three new variants are in the spec.
3. **384 programs.** New `synth_census` (`crates/nord-tools`, tests in `synth_census.rs`): RMS of key 60 velocity 100 (2.08 s hold, mono mean) of every program with an enabled synth layer (351 of 384), engine default against each variant. A tool convention (ASSUMED), the same for every variant.

```
cargo build --release -p nord-tools --bin synth_census
target/release/synth_census "<factory>.ns4b" --jobs 4 --variant "uw=--synth-unison-weights" \
    --variant "coupled=--synth-p coupled" --variant "oracle=--synth-oracle" > census.jsonl
python tools/emu_vs_hw.py --video <Part A>.webm --backup "<factory>.ns4b" --ns4 target/release/ns4.exe \
    --workdir w --only synth_lead synth_crunch ... --out res.json
```

## 3. The oracle filter reads +3 .. +7 dB above the native one

Per program (layer A, unity amp, key 60): `Gap` = oracle filter gain minus native filter gain in dB (`data/synth-filter-gain-oracle.csv`).

| Program | Row | Filter | Res | Cutoff (Hz) | Unison | Oracle gain | Native gain | Gap |
|---|---|---|---|---|---|---|---|---|
| Smooth Lead | shape | Lp24 | 0.516 | 370 | 0 | +6.0 | +0.1 | **+5.9** |
| Crunch Synth | shape | Lp12 | 0.404 | 622 | 1 | +6.0 | +0.1 | **+5.8** |
| Ballad Layer Whl | shape | Lp24 | 0.0 | 1000 | 3 | +6.5 | -0.2 | **+6.7** |
| Slow Motion Pad | shape | Lp24 | 0.0 | 4200 | 2 | +6.6 | -0.1 | **+6.7** |
| 80s Sci-Fi | multi | Ladder24 | 0.456 | 1500 | 0 | -4.2 | -9.5 | **+5.3** |
| Mega Line Whl | multi | Lp12 | 0.127 | 1200 | 3 | +4.2 | -0.1 | **+4.4** |
| Funk Hit Split | multi | Lp12 | 0.0 | 7000 | 0 | +2.8 | -0.1 | **+2.8** |
| Statement Syn Bs | multi | Ladder24 | 0.539 | 370 | 0 | -5.6 | -8.8 | **+3.1** |
| Silk Pad | super | Lp12 | 0.097 | 294 | 3 | +4.0 | -1.4 | **+5.3** |
| Horizon Whl | super | Lp24 | 0.0 | 277 | 0 | +3.0 | -0.9 | **+3.9** |
| Sidechain Whl II | super | Lp24 | 0.097 | 1600 | 3 | +5.0 | -0.3 | **+5.4** |
| Oo Pad Whl | super | Lp24 | 0.606 | 208 | 2 | +5.3 | +2.6 | **+2.7** |
| Believer | super | Lp24 | 0.0 | 1000 | 3 | +5.0 | -0.6 | **+5.6** |
| Glow Chords Whl | super | Lp12 | 0.95 | 277 | 0 | -6.8 | -22.0 | +15.2 (outlier) |
| 12dB Sweep | shapesq | Lp12 | 0.726 | 147 | 3 | +4.3 | -0.7 | **+4.9** |
| Filter Bank Ld | saw | BandPass | 0.756 | 165 | 0 | -1.9 | -1.9 | **0.0** |
| Phat 3 Osc MiniM | saw | Ladder24 | 0.0 | 13000 | 1 | -2.3 | -0.1 | -2.2 |
| Leaky Stabs II | shapesaw | Ladder24 | 0.037 | 2500 | 3 | -3.5 | -1.9 | -1.6 |

By type: LP24 `+5.3` mean (7 programs, `+2.7 .. +6.7`), LP12 `+4.6` (5 programs without the outlier, `+2.8 .. +5.8`), Ladder24 `+1.2` (4 programs, two near `0`: the filters of `Phat 3` and `Leaky` run wide open, `13000` / `2500` Hz), band pass `0.0` (1 program). The gap does not follow the resonance word (`+5.6` at 0.0, `+5.3` at 0.6), the unison (`+5.9` at 0, `+5.8` at 1) or the oscillator row. The previous D132 reading (`20 log10 1.98 = +5.93 dB`, the table A input gain at resonance 0) fits the top of the LP range and nothing else: the native firmware filter port (`--synth-fw-filter`) carries `cin` and still shows `+4.6 .. +5.4` (D132), so the gap is not the input gain alone (ASSUMED: it includes the cutoff the native strip law puts on the filter, which differs from the oracle's by the fitted per row factors).

Reading against the rows:

| Row | Fitted | Oracle row (D140) | Excess | Fit clip filter | Oracle gap of that type |
|---|---|---|---|---|---|
| SHAPE triangle | 0.6 | 0.4 | `+3.5 dB` | LP24 (Smooth Lead) | `+5.3` (LP24) |
| MULTI | 3.8 | 1.62 | `+7.4 dB` | Ladder24 (80s Sci-Fi), LP12 (Mega Line) | `+5.3` / `+4.4` |
| SUPER | 2.4 | 1.90 | `+2.0 dB` | LP12 (Silk Pad), LP24 (Horizon) | `+5.3` / `+3.9` |
| saw (anchor) | 0.4 | 0.4 | 0 | band pass (Filter Bank), wide open Ladder24 (Phat 3) | `0.0` / `-2.2` |

The fitted rows were each fitted behind a filter that is `+4 .. +5 dB` too quiet; the saw, which fixes the absolute voice gain (`D = 0.4`), was fitted behind filters that are not. The relative offsets of the three rows therefore come out `+2 .. +7 dB`, the same order as the filter gap, with a `+- 3 dB` residual per row that is the per clip error (velocity, envelope and arp differences between the video and the oracle; `synth_crunch` and `synth_lead` disagree by 7 dB on the same SHAPE triangle row).

## 4. Clips: oracle rows times a filter gain, with and without the unison weights

Score = `emu_vs_hw` gap score (lower is better), RMS = emulator minus hardware level (dB). `ORx` = SHAPE triangle / MULTI / SUPER at `0.4 / 1.6 / 1.9` times `x` (cutoff factors unchanged), `uw` = `--synth-unison-weights`.

| Clip (rows hit) | default | uw | OR 1.0 + uw | OR 1.414 + uw | OR 1.778 + uw |
|---|---|---|---|---|---|
| `synth_lead` (tri) | 0.80 / -3.66 | 0.80 / -3.66 | 0.80 / -7.18 | 0.80 / -4.16 | 0.80 / -2.19 |
| `synth_crunch` (tri, unison 1) | 2.07 / +0.50 | 2.08 / +3.31 | 2.07 / +0.01 | 2.08 / +2.84 | 2.08 / +4.63 |
| `synth_silkpad` (super) | 1.36 / -0.50 | 1.36 / -0.50 | 1.36 / -2.53 | 1.36 / +0.48 | 1.36 / +2.47 |
| `synth_megaline` (multi, super) | 2.17 / -0.90 | 2.17 / -0.90 | 1.42 / -7.39 | 1.61 / -4.42 | 2.03 / -2.37 |
| `synth_horizon` (super, multi) | 2.30 / -0.35 | 2.30 / -0.35 | 1.94 / -2.41 | 1.93 / +0.60 | 1.94 / +2.59 |
| `super_lp24_horizon` | 2.30 / -0.35 | 2.30 / -0.35 | 1.94 / -2.41 | 1.93 / +0.60 | 1.94 / +2.59 |
| `super_lp24_sidechain` | 3.88 / +9.79 | 3.88 / +9.79 | 3.69 / +9.74 | 3.98 / +9.83 | 4.22 / +9.92 |
| `super_lp24_oopad` | 4.88 / +0.25 | 4.57 / +0.82 | 4.74 / +0.40 | 4.63 / +0.74 | 4.56 / +1.11 |
| `super_lp24_believer` | 1.66 / +7.46 | 1.66 / +7.46 | 1.66 / +7.23 | 1.65 / +7.62 | 1.63 / +8.03 |
| `synth_80s` (multi) | 1.16 / +5.44 | 1.16 / +5.44 | 1.78 / -3.10 | 1.65 / +0.04 | 1.50 / +2.25 |
| `synth_12db` (shape square) | 1.38 / +6.62 | 1.38 / +6.62 | 1.38 / +6.62 | 1.38 / +6.62 | 1.38 / +6.62 |
| `synth_phat3` (unison 1) | 2.28 / -3.55 | 2.28 / -0.56 | 2.28 / -0.56 | 2.28 / -0.56 | 2.28 / -0.56 |
| `synth_leaky` (unison 3) | 1.90 / -4.64 | 1.60 / +1.05 | 1.60 / +1.05 | 1.60 / +1.05 | 1.60 / +1.05 |
| `synth_filterbank`, `synth_sineplk`, `pianopad_grand` (no row hit) | unchanged | unchanged | unchanged | unchanged | unchanged |
| **mean score of 16** | 2.784 | 2.747 | **2.693** | 2.706 | 2.733 |
| **mean abs RMS error (dB)** | 4.09 | 3.89 | 4.50 | **3.81** | 4.25 |

(Cells are `score / RMS`.) Reading:

- The score is flat in `G` (`2.69 .. 2.74`, below the default `2.78`): it is dominated by clip specific band errors that a level factor does not touch (`synth_filterbank` 7.1, `synth_sineplk` 7.5, `super_lp24_oopad` 4.6, `super_lp24_sidechain` 3.9). The level error is not: it has its minimum at `G = sqrt 2` (`3.81 dB`, against `3.89` with the weights alone and `4.09` for the default). The oracle rows alone (`G = 1`) are the worst on level (`4.50`), as D140 found for `--synth-p verified`.
- `G = sqrt 2` is `+3.0 dB`, `1.6 .. 2.3 dB` under the oracle filter gap of the types the rows were fitted on (`+4.6 .. +5.3`, section 3). The remainder is the per clip scatter, not a second effect (ASSUMED).
- The rows disagree between clips of the same row by more than `G` can move: SHAPE triangle wants `G = 1` on `synth_crunch` (with the weights) and `G = 2.4` on `synth_lead`; MULTI wants `1.4` on `synth_80s` and `2.2` on `synth_megaline`. One factor is the compromise, not a fit of every clip.
- `synth_12db` and the `synth_oracle` / `--synth-p coupled` variants also carry the D140 idle rows (`+6.62 -> +0.21 dB`, score `1.38 -> 1.50`), which the sweep columns above do not include.

Real flags, 16 clips (`score / RMS`; `--synth-oracle` and `--synth-p coupled` include the D140 idle rows, `--synth-oracle` adds the unison weights):

| Clip | default | `--synth-unison-weights` | `--synth-p coupled` | `--synth-oracle` |
|---|---|---|---|---|
| `synth_lead` | 0.80 / -3.66 | 0.80 / -3.66 | 0.80 / -4.16 | 0.80 / -4.16 |
| `synth_crunch` | 2.07 / +0.50 | 2.08 / +3.31 | 2.07 / +0.02 | 2.08 / +2.84 |
| `synth_silkpad` | 1.36 / -0.50 | 1.36 / -0.50 | 1.36 / +0.48 | 1.36 / +0.48 |
| `synth_filterbank` | 7.09 / +5.54 | 7.09 / +5.54 | 7.09 / +5.54 | 7.09 / +5.54 |
| `synth_sineplk` | 7.48 / +2.99 | 7.48 / +2.99 | 7.48 / +2.99 | 7.48 / +2.99 |
| `synth_megaline` | 2.17 / -0.90 | 2.17 / -0.90 | 1.61 / -4.42 | 1.61 / -4.42 |
| `synth_horizon` | 2.30 / -0.35 | 2.30 / -0.35 | 1.94 / +0.60 | 1.94 / +0.60 |
| `super_lp24_horizon` | 2.30 / -0.35 | 2.30 / -0.35 | 1.94 / +0.60 | 1.94 / +0.60 |
| `super_lp24_sidechain` | 3.88 / +9.79 | 3.88 / +9.79 | 3.98 / +9.83 | 3.98 / +9.83 |
| `super_lp24_oopad` | 4.88 / +0.25 | 4.57 / +0.82 | 4.92 / +0.24 | 4.63 / +0.74 |
| `super_lp24_believer` | 1.66 / +7.46 | 1.66 / +7.46 | 1.65 / +7.62 | 1.65 / +7.62 |
| `synth_12db` | 1.38 / +6.62 | 1.38 / +6.62 | 1.50 / +0.21 | 1.50 / +0.21 |
| `synth_phat3` | 2.28 / -3.55 | 2.28 / -0.56 | 2.28 / -3.55 | 2.28 / -0.56 |
| `synth_80s` | 1.16 / +5.44 | 1.16 / +5.44 | 1.65 / +0.05 | 1.65 / +0.05 |
| `synth_leaky` | 1.90 / -4.64 | 1.60 / +1.05 | 1.90 / -4.64 | 1.60 / +1.05 |
| `pianopad_grand` | 1.84 / +12.92 | 1.84 / +12.92 | 1.84 / +12.94 | 1.84 / +12.94 |
| **mean score / mean abs RMS (dB)** | **2.784 / 4.09** | **2.747 / 3.89** | **2.751 / 3.62** | **2.714 / 3.41** |

## 5. The 384 factory programs (`synth_census`, key 60 velocity 100, 351 programs with a synth layer)

RMS change against the engine default, dB (`data/synth-rows-unison-census.csv`):

| Variant | Programs moved by more than 1 / 3 dB | Mean abs | Range | Note |
|---|---|---|---|---|
| `--synth-unison-weights` | 21 / 15 | 0.26 | `0 .. +6.0` | analog unison layers only; `+4.3` dB mean of the movers |
| three coupled rows only (`table:shape,multi,super`) | 24 / 19 | 0.31 | `-5.6 .. +1.0` | SHAPE triangle / MULTI / SUPER programs; `-3.7` dB mean of the movers |
| D140 idle rows only (`--synth-p oracle`) | 47 / 40 | 0.82 | `-8.0 .. +0.2` | SHAPE square / PURE sine / SUB / SYNC / FM at `-8.0 dB`: PWM and sync programs |
| `--synth-p coupled` (rows + idle rows) | 70 / 60 | 1.14 | `-8.0 .. +1.0` | |
| `--synth-oracle` (coupled + weights) | 89 / 73 | 1.36 | `-8.0 .. +6.0` | `-3.2` dB mean of the movers |
| `--synth-stage3` (for the record) | 11 / 6 | 0.16 | `-6.7 .. +11.4` | raw type 3 layers only |

Most of the movement of `--synth-p coupled` is the idle rows (already measured by D140: the oracle puts them `+8 dB` under the engine default); the three rows move 24 programs.

## 6. Proposal for the owner (not applied)

`--synth-oracle` (`--synth-p coupled --synth-unison-weights`) as the default synth level path; the opt-out flag would be added with the flip (`--legacy-synth-level`, not implemented here).

For:
- Every number in the three row gains is now oracle-derived or one named factor (`COUPLED_FILTER_GAIN`), instead of three clip fits; the unison law matches the firmware lane words (D178, VERIFIED words).
- Level error on 16 synth clips `4.09 -> 3.41 dB` mean absolute (`synth_80s` `+5.4 -> 0.0`, `synth_12db` `+6.6 -> +0.2`, `synth_leaky` `-4.6 -> +1.1`, `synth_phat3` `-3.6 -> -0.6`); the mean gap score is `2.784 -> 2.714`.
- It is the only way the unison weights can become the default without `synth_crunch` getting louder against the hardware (D178 blocked on that).

Against:
- The clip score improvement is `0.07` points on a mean of `2.78`: below the noise of one lossy clip; the D121 / D178 rule asks for a clear win on the clips.
- `synth_crunch` (unison 1): `+0.50 -> +2.84 dB` (loud), `synth_megaline` `-0.90 -> -4.42 dB` (quiet), `synth_12db` band score `1.38 -> 1.50`.
- 89 of 351 synth programs move by more than 1 dB (73 by more than 3 dB; 40 of those are the D140 idle rows, which the owner can accept separately).
- `COUPLED_FILTER_GAIN` is EMPIRICAL. The honest fix is a native filter input gain by type (`+5 dB` LP12 / LP24, `0` band pass, per program for Ladder24), which needs the cutoff mapping fixed first: `--synth-fw-filter` still scores `3.65` against `2.42` (D178).

My recommendation: flip **only** `--synth-unison-weights` plus the three coupled rows together if the owner accepts the level criterion; otherwise keep the default and use `--synth-oracle` for listening tests. The two clips that would move against the hardware (`synth_crunch`, `synth_megaline`) are the first listening check.

## Implications

- The fitted SHAPE triangle / MULTI / SUPER rows are explained: oracle row times the missing filter gain, within `+- 3 dB` per clip. They are not wrong levels, and replacing them with the oracle rows alone is wrong.
- The oracle gives the row ratios, the clips give one factor. A native filter gain by type would make the factor derivable; it is blocked by the cutoff mapping (`--synth-fw-filter` gap).
- `--synth-stage3` stays out of the preset: `super_lp24_oopad` carries a raw type 3 filter and scores `4.63 -> 5.29` (RMS `+0.74 -> +3.63 dB`) with the stage, the first clip that exercises it.

## Follow-ups

1. Native filter input gain by type from the oracle table (`+4.6` LP12, `+5.3` LP24, `0` band pass, Ladder24 per program) together with the cutoff mapping fix: would replace `COUPLED_FILTER_GAIN`.
2. A Part A clip of a raw type 3 program (`Oo Pad Whl` is one; its clip got worse with `--synth-stage3`): decide `--synth-stage3` separately.
3. A phase averaged oracle unison level (D178 follow-up 1) to separate the beat term from the weights.
4. Silent oracle programs and the Ladder24 outliers (`Phat 3`, `Leaky Stabs II` gap `-2`): the Ladder24 stage gain looks level dependent.
