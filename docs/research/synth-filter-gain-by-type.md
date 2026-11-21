# Native synth filter gain by filter type, measured against the oracle (D197)

- **Checked:** 2026-11-21
- **Sources (local, never committed):** ORG core 2 image (`target/synth_env/sram/core2_sram.bin`), OS v1.66 ARM firmware through `tools/arm_dsp_capture.py` (recall plus key 60 velocity 100, four more HighPass programs, [arm-dsp-capture](arm-dsp-capture.md)), `sharc-sys`, native `synth_e2e_native`, `tools/synth_e2e_oracle.py report` (D132 pipeline, unchanged), `tools/emu_vs_hw.py` with the Part A video (`_zksT69H30c`) and the factory backup (16 synth segments of [data/emu_vs_hw_segments.json](data/emu_vs_hw_segments.json)), the two Nord installers named in section 6. Derived numbers only: [data/synth-filter-gain-oracle.csv](data/synth-filter-gain-oracle.csv) (22 programs, 4 new), [data/synth-filter-gain-clips.csv](data/synth-filter-gain-clips.csv) (16 clips x 6 variants). Background: [synth-rows-unison](synth-rows-unison.md) (D194), [synth-osc-gain-oracle](synth-osc-gain-oracle.md) (D140), [synth-e2e-oracle-vs-native](synth-e2e-oracle-vs-native.md) (D132).
- **Question:** (1) What is the gain of the oracle filter over the native one per filter type, including HighPass, which the D194 table had no program for? (2) Does applying it per type (with the plain oracle rows, no fitted workaround) beat the single coupling factor of D194 on the 16 Part A synth clips? (3) Do the Nord Sound Manager / Sample Editor installers help build probe programs?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Gain per type | `Lp12 +4.65 dB` (5 programs), `Lp24 +5.26` (7), `Ladder24 +1.17` (4, bimodal), `BandPass 0.00` (1), **`HighPass +3.62` (4: `+7.3`, `+4.0`, `+3.7`, `-0.6`)**. Table: `synth/filter_gain.rs`; reduction and check: `cargo run --release -p nord-tools --bin synth_filter_gain` (exits non-zero when the compiled table differs from the CSV by more than 0.06 dB) | VERIFIED means (n as listed); HighPass and Ladder24 EMPIRICAL (spread) |
| HighPass is not `0 dB` | The first draft assumed `0 dB` (no program in the D194 set). Four factory layer A HighPass programs measured `+7.3 / +4.0 / +3.7 / -0.6 dB`; the `-0.6` one (`Distorted Whl II`) sits at the `14 Hz` cutoff floor, where the filter is transparent in both engines | VERIFIED (4 runs) |
| Opt-in | `--synth-filter-gain` (scales the filter output by the type gain; also selects `--synth-p oracle-rows` = the oracle rows `0.4 / 1.6 / 1.9` without the `sqrt 2` of D194). Default off | opt-in |
| Does it beat D194 on the clips? | **No.** Mean abs RMS error over 16 clips (current main): default `3.39 dB`, `--synth-oracle` (D194) `2.77`, **`--synth-filter-gain` `4.08`** (`+ --synth-unison-weights` `4.07`). Mean score `2.708 / 2.640 / 2.685 / 2.652` | VERIFIED (run) |
| Why | The type gain is the oracle's filter-stage level (tap `IN0` before the output chain). The hardware clips want about `+3 dB` on the three fitted rows and `0 dB` elsewhere. Applied to every row it overshoots `LP12` / `LP24` clips: `synth_sineplk` (SHAPE sine, LP12, no fitted row) `+3.0 -> +7.5 dB`, `synth_12db` `+0.2 -> +4.6`, `super_lp24_oopad` `+0.7 -> +5.1`, `synth_horizon` `+0.6 -> +2.9` | EMPIRICAL |
| Flip? | **No.** Nothing here supports a default change; `--synth-oracle` stays the better candidate. The per type gain is evidence about the filter stage, not a replacement for `COUPLED_FILTER_GAIN` | proposal |
| Installers | Do not help (section 6) | VERIFIED (inspection) |

## 2. Method

1. **Oracle gain per program.** Unchanged from [synth-rows-unison](synth-rows-unison.md) s2: `tools/synth_e2e_oracle.py report CAPTURE PROGRAM --sharc-sys ... --image core2_sram.bin --native ... --workdir ...`; the gap is `oracle_filter_gain_db - native_filter_gain_db` (filtered / unfiltered RMS ratio per engine, which cancels oscillator, envelope and voice gain).
2. **HighPass probes.** The factory set has 11 programs (14 layers) with `filter_model = HighPass` (`program-sweep.json`); layer A of five of them is an analog or sample oscillator. Captures (about 70 s each) of `HP Splash Whl` (super, unison 2), `Replica Lead` (shape saw, unison 1), `Chip Bass` (saw), `Distorted Whl II` (other, cutoff 14 Hz) and `Gravity Whl` (sampled: the oracle layer is silent, `-600 dB`, excluded like the silent programs of D194).
3. **Reduction.** `crates/nord-tools/src/synth_filter_gain.rs`: group by filter model, drop rows more than 8 dB from the type median (only `Glow Chords Whl`, resonance 0.95, `+15.2` against an LP12 median `+4.9`), mean, min, max, standard deviation.
4. **Clips.** `tools/emu_vs_hw.py` on the 16 synth segments, variants `default`, `unison-weights`, `synth-coupled`, `synth-oracle`, `synth-filter-gain`, `synth-filter-gain+uw` (the last two are new in the spec JSON).

```
cargo build --release -p nord-core --bin ns4 -p nord-tools --bin synth_filter_gain
cargo run --release -p nord-tools --bin synth_filter_gain
uv run python tools/emu_vs_hw.py --video <Part A>.webm --backup "<factory>.ns4b" --ns4 target/release/ns4.exe \
    --spec spec.json --workdir w --out res.json
```

## 3. Oracle gain by filter type

| Type | n | Mean (dB) | Min | Max | Std | Excluded |
|---|---|---|---|---|---|---|
| BandPass | 1 | `0.02` | `0.02` | `0.02` | `0.00` | |
| HighPass | 4 | `3.62` | `-0.60` | `7.30` | `2.81` | |
| Ladder24 | 4 | `1.17` | `-2.20` | `5.30` | `3.15` | |
| Lp12 | 5 | `4.65` | `2.83` | `5.81` | `1.03` | `Glow Chords Whl` |
| Lp24 | 7 | `5.26` | `2.67` | `6.71` | `1.38` | |

New HighPass rows (`data/synth-filter-gain-oracle.csv`): `HP Splash Whl` oracle `+4.17`, native `-3.13`, gap `+7.30`; `Replica Lead` `+4.10`, `+0.06`, `+4.04`; `Chip Bass` `+3.91`, `+0.17`, `+3.74`; `Distorted Whl II` `+0.37`, `+0.97`, `-0.60`.

Reading: the gap is a property of the filter being in the signal path. Wide open or at the cutoff floor (`Phat 3` Ladder24 13 kHz `-2.2`, `Leaky` 2.5 kHz `-1.6`, `Distorted Whl II` HighPass 14 Hz `-0.6`) the oracle gain falls to or below `0`. A type constant cannot express that; a table indexed by type and cutoff could, but the 22 programs do not span it (ASSUMED).

## 4. Clips (current main, 16 Part A synth segments)

`score / RMS` (RMS = emulator minus hardware level, dB), full table in `data/synth-filter-gain-clips.csv`:

| Variant | Mean score | Mean abs RMS (dB) | RMS error (dB, root mean square) |
|---|---|---|---|
| `default` | 2.708 | 3.39 | 4.69 |
| `--synth-unison-weights` | 2.671 | 3.19 | 4.55 |
| `--synth-p coupled` (D194) | 2.676 | 2.97 | 4.45 |
| `--synth-oracle` (D194) | **2.640** | **2.77** | **4.27** |
| `--synth-filter-gain` | 2.685 | 4.08 | 5.02 |
| `--synth-filter-gain --synth-unison-weights` | 2.652 | 4.07 | 5.05 |

The D194 baseline moved since the PR was written (mean abs RMS `4.09 -> 3.39` for the default) because main changed in between (for example `super_lp24_sidechain` `+9.8 -> +2.7`, `synth_filterbank` `+5.5 -> -2.4`); the variants are compared on the same build.

Clips that move most against the hardware with `--synth-filter-gain` relative to `--synth-p coupled`: `synth_sineplk` `+2.99 -> +7.48`, `super_lp24_oopad` `+0.24 -> +4.80`, `synth_12db` `+0.21 -> +4.57`, `synth_horizon` / `super_lp24_horizon` `+0.60 -> +2.85`, `synth_silkpad` `+0.48 -> +2.12`. Better: `synth_lead` `-4.16 -> -1.92`, `synth_megaline` `-4.42 -> -2.74`, `synth_phat3` (with weights) `-0.56 -> +0.60`.

## 5. Why the type gain does not beat the single factor

- `COUPLED_FILTER_GAIN = sqrt 2` (`+3.0 dB`) minimised the clip error in D194 and sits `1.6 .. 2.3 dB` below the oracle type means. The clips, not the oracle, set the level: the oracle tap `IN0` is the stage record, upstream of everything the native strip chain adds after the filter (ASSUMED: the output chain accounts for the missing `1.5 .. 2 dB`; not verified).
- `COUPLED_FILTER_GAIN` only touches the three rows that were fitted behind the quiet filter. A type gain touches every row, including those D140 / D194 already put at the oracle level without a filter gain (SHAPE square `synth_12db`, SHAPE sine `synth_sineplk`, PURE, SUB): those clips go `+4 .. +5 dB` loud. This is evidence that the per row anchoring, not only the filter, carries a level (ASSUMED).
- The type means have a wide spread (`+- 1 .. 3 dB`) and no model for the cutoff dependence (section 3).

## 6. Nord Sound Manager 9.22 and Sample Editor 4.36: not useful for probes (do not repeat)

Both setups are NSIS installers; `7z x` unpacks them without running anything (extracted to git-ignored `.reference/`; no vendor file is committed).

| Installer | Finding | Use |
|---|---|---|
| Sound Manager 9.22 (`Nord Sound Manager v9.22.exe`, 16 MB, MSVC C++) | Converter classes only: `CStage4SynthConverter`, `CStage4OrganConverter`, `CStage4PianoConverter`, `CStage4EffectsConverter`, `CStage4MultiConverter` (`CConverterBaseStrict<EBuffer<stage4>, EParam, stage3...>`: **Stage 3 to Stage 4**, plus `electro6/7`, `piano5`, `grand2` to Piano 6 converters), `CBinInput/OutputNordFile`, and the `Edt_Syn_kFlt*` / `Param_Syn_kFlt*` names (the same parameter ids as `arm-edit-params.csv`). It is not a program editor | none; a program with a chosen filter type can only be reached through a Stage 3 conversion, which our own CBIN writer does without it |
| Sample Editor 4.36 | `.nsmp4` / `.nsmp` sample tooling only; the `Invalid filter type specified` string is libpng, not a Nord filter | none |

The HighPass probes came from the factory backup instead: `ns4 inspect --json` over the 384 programs finds the layers with `filter_model = HighPass`.

## Implications

- The native filter is `+3.6 .. +5.3 dB` below the oracle filter on LP12, LP24 and HighPass, `+1.2 dB` on Ladder24 (bimodal) and equal on band pass. This is measured and now has a table, a tool and a test.
- It does not replace `COUPLED_FILTER_GAIN`: the hardware clips prefer a smaller factor on three rows only. D194's `--synth-oracle` remains the best candidate for the owner's flip decision.
- The oracle filter level and the clip level disagree by `1.5 .. 2.3 dB` per type; that gap, not the filter, is the open item.

## Follow-ups

1. Find what the native chain after the filter lacks or adds against the oracle (output chain, voice gain) so the `1.5 .. 2.3 dB` between the oracle type gain and the clip optimum is explained instead of fitted.
2. A cutoff dependent gain (Ladder24 and HighPass are bimodal) needs more oracle programs per type and cutoff band; the HighPass set still has `Heart Strings II`, `Syn Flute`, `Ghost Melody`, `Slow Octaves`, `Chrome Layer` (layer B) and `ScissorSplit Whl` (layer C), which this tap (layer A) does not read.
3. `Gravity Whl` (HighPass, sampled layer A) needs the sample bank to be audible in the oracle.
