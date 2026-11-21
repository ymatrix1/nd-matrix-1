# Analog unison level: why native sits 2.4 to 8.3 dB below the oracle lane, and the clip rescore of the synth filter

- **Checked:** 2026-11-20
- **Sources (local, never committed):** [synth-osc-gain-oracle](synth-osc-gain-oracle.md) s4 (oracle lane against native, 12 unison programs, D140), the verified P words of the unison variants ([synth-p-writer](synth-p-writer.md) s4, [data/synth-p-words.csv](data/synth-p-words.csv), type 2), the lane mix of [synth-wave-models](synth-wave-models.md) s2, `crates/nord-core/src/synth/render.rs` (unison sum), `tools/emu_vs_hw.py` with the Part A video (`_zksT69H30c`) and the factory backup, run on this PR's release `ns4`. Derived numbers only.
- **Question:** (1) Why is the native analog unison `2.4 .. 8.3 dB` below the oracle lane at the same file unison, and is the native law wrong? (2) Does `--synth-fw-filter` still score worse than the fitted factors after the 12 clip re-run (D121: 3.36 against 2.18), and does the stage `0x1257B2` change a clip?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Item | Result | Tag |
|---|---|---|
| Cause of the gap | the native unison sum normalises by `1 / sqrt(N)` (`render.rs`), so a channel carries half the power of one voice for every `N` (2 or 4). The firmware lane mix has **no** voice count term: every oscillator enters its lane with the fixed weights `0.92388` (own lane) and `0.38268` (partner lane), unit norm per oscillator. A lane then carries power `1.0` at unison 1 (one oscillator pair) and `2.0` at unison 2 and 3 (two pairs) against the native `0.5`: **`+3.0 dB` and `+6.0 dB` structural gap** (uncorrelated voices) | VERIFIED (words), the power sum is arithmetic |
| Channel split | the native weak channel gain `0.43` (EMPIRICAL, L / R peak ratio) is the firmware ratio `0.38268 / 0.92388 = tan(pi / 8) = 0.41421` within 4 % | VERIFIED (words) against EMPIRICAL (measurement) |
| Rest of the measured gap | the oracle lane level moves by `+- 2 dB` around its unison 0 value with the setting (`+1.2`, `+1.1`, `-1.8 dB` mean at unison 1, 2, 3) while the power sum predicts `0`, `+3`, `+3 dB`: a window RMS of beating voices (2 to 6 cent detune: 0.3 to 1.5 Hz beat at C4) is not an average over phase. The spread (`+2.4 .. +8.3 dB`) is the structural `3 .. 6 dB` plus this beat term | ASSUMED (not isolated) |
| Mapping `kOscUnison` to `(a, b)` | not captured ([synth-p-writer](synth-p-writer.md) s4, s6). The voice counts agree (2 voices at `a = 1`, 4 at `a >= 2`, `UNISON_VOICE_COUNT = [0, 2, 4, 4]`) and the ratio pairs equal `UNISON_VOICES` (`2`, `4 / 1.32`, `6 / 2` cents) | VERIFIED (ratios), ASSUMED (assignment) |
| Fix | **opt-in** `--synth-unison-weights` (`SynthLayer::unison_fw_weights`): per-voice gain `1.0` instead of `1 / sqrt(N)` (`+3.0 / +6.0 dB`); `--synth-unison-gain <x>` is the generic factor. Default unchanged | decision |
| Clip effect | level moves toward the hardware on both clips whose row is not suspect: `synth_leaky` (unison 3) RMS delta `-4.64 -> +1.05 dB`, score `1.90 -> 1.60`; `synth_phat3` (unison 1) `-3.86 -> -0.86 dB`, score `2.38 -> 2.38`. `synth_crunch` (unison 1) gets louder against the hardware (`+2.55 -> +5.36 dB`, score `0.89 -> 0.90`): its SHAPE triangle row is `+3.5 dB` above the oracle level (D140), so the clip needs the row fix together with the unison one. Mean of 12: `2.415 -> 2.392` | VERIFIED (run) |
| `--synth-fw-filter` rescore | **the gap did not shrink**: default `2.415`, `--synth-fw-filter` `3.652` (12 clips, this build); D121 had `2.18` / `3.36`. Ratio `1.51` against `1.54`, difference `1.24` against `1.18` | VERIFIED (run) |
| Raw type 3 stage on the clips | `--synth-stage3` is bit identical to the default on all 12 clips: none of them carries a raw type 3 layer | VERIFIED (run) |

## 2. Hardware lane mix (VERIFIED words, type 2 = PURE saw, [data/synth-p-words.csv](data/synth-p-words.csv))

Lane x output (the model of [synth-wave-models](synth-wave-models.md) s2): `own(osc1) wB1 + partner(osc1) wA1 + own(osc2) wA2 + partner(osc2) wB2`, scaled by `P[16] = 0.25`.

| Variant | `P[-16]` / `P[-14]` (osc 1 own / partner) | `P[-12]` / `P[-10]` (osc 2) | Oscillator ratios (lane x / lane y) | Power per lane (uncorrelated), relative to unison 0 |
|---|---|---|---|---|
| `a = 0` (unison 0) | 1 / 0 | 0 / 0 | 1 / 1 | `1.0` (0 dB) |
| `a = 1`, `b = 1` | 0.92388 / 0.38268 | 0 / 0 | osc 1: 1.001156 / 0.998845 (`+-2` cents) | `0.92388^2 + 0.38268^2 = 1.0` (0 dB) |
| `a = 2, 3`, `b = 2, 3` | 0.92388 / 0.38268 | 0.92388 / 0.38268 | osc 1 `+-4 / +-6` cents, osc 2 `+-1.32 / +-2` cents | `2.0` (+3.0 dB) |

Native (`render.rs`): voice gain `1 / sqrt(N)`, pan `(norm, norm 0.43)` with `norm = 1 / sqrt(1 + 0.43^2) = 0.9185`: power per channel `(1/N) (N/2) (norm^2 + norm^2 0.43^2) = 0.4998`, i.e. **-3.0 dB** against one voice for every `N`. The difference between the two columns (hardware `1.0` / `2.0`, native `0.5`) is the `+3.0 / +6.0 dB` of the table in s1. Correlated voices (the first beat of a held note) raise both sides in the same way.

## 3. Measured gap (D140, [synth-osc-gain-oracle](synth-osc-gain-oracle.md) s4)

| Unison | Gap oracle minus native (dB), D140 | Structural gap | Left over |
|---|---|---|---|
| 1 (2 voices) | `+6.0`, `+5.9`, `+6.3` | `+3.0` | `+3.0 .. +3.3` |
| 2 (4 voices) | `+8.3`, `+6.9` | `+6.0` | `+0.9 .. +2.3` |
| 3 (4 voices) | `+2.5`, `+4.4` | `+6.0` | `-3.5 .. -1.6` |

The left over is the beat term of the oracle window (unison 3 detunes by `+-6` cents: 1.5 Hz beat at C4, so an RMS window of a few periods lands in a destructive phase, `-1.8 dB` against unison 0) plus the native start phases (`UNISON_START_PHASES`, ASSUMED). No phase averaged oracle run exists: closing it needs a free running voice phase poke in the oracle ([follow-up 1](#follow-ups)).

## 4. Clip rescore (this build, 12 synth and layered clips of the Part A video)

`python tools/emu_vs_hw.py --video <Part A .webm> --backup <factory .ns4b> --ns4 target/release/ns4.exe --only synth_lead ... pianopad_grand` (spec variants `default`, `synth-fw-filter`, `stage3`, `uni1414` = `--synth-unison-gain 1.414`, `uni2` = `--synth-unison-gain 2.0`; then `--synth-unison-weights` on four clips). Gap score, lower is better; `RMS` = emulator minus hardware level (dB).

| Clip | default | `--synth-fw-filter` | `--synth-stage3` | unison x1.414 | unison x2 | `--synth-unison-weights` | RMS default / weights |
|---|---|---|---|---|---|---|---|
| `synth_lead` | **0.80** | 3.50 | 0.80 | 0.80 | 0.80 | | -3.66 |
| `synth_crunch` (unison 1) | **0.89** | 0.90 | 0.89 | 0.90 | 0.90 | 0.90 | +2.55 / +5.36 |
| `synth_silkpad` | **1.47** | 4.28 | 1.47 | 1.47 | 1.47 | | -0.57 |
| `synth_filterbank` | **5.40** | 7.65 | 5.40 | 5.40 | 5.40 | | -1.07 |
| `synth_sineplk` | **7.48** | 8.51 | 7.48 | 7.48 | 7.48 | | +2.99 |
| `synth_megaline` | **2.17** | 5.30 | 2.17 | 2.17 | 2.17 | | -0.90 |
| `synth_horizon` | **2.34** | 3.12 | 2.34 | 2.34 | 2.34 | | -0.32 |
| `synth_12db` | **1.39** | 2.72 | 1.39 | 1.39 | 1.39 | 1.39 | +6.65 |
| `synth_phat3` (unison 1) | 2.38 | 2.40 | 2.38 | 2.38 | 2.39 | 2.38 | -3.86 / -0.86 |
| `synth_80s` | **0.91** | 1.44 | 0.91 | 0.91 | 0.91 | | +5.03 |
| `synth_leaky` (unison 3) | 1.90 | 2.16 | 1.90 | 1.71 | **1.60** | **1.60** | -4.64 / +1.05 |
| `pianopad_grand` | 1.85 | 1.85 | 1.85 | 1.85 | 1.85 | | +12.92 |
| mean of 12 | **2.415** | 3.652 | 2.415 | 2.400 | 2.392 | | |

Reading:

- The default baseline moved from D121 (2.18) to 2.415 because `main` moved (Mod 2 default D163 and other merged work: `synth_sineplk` 4.73 to 7.48, `pianopad_grand` 2.19 to 1.85). The same build runs every variant of the table, so the rows compare with each other, not with D121. The `synth_sineplk` change is outside this PR (follow-up 3).
- `--synth-fw-filter` loses on every clip except `pianopad_grand` (equal): the largest losses are `synth_megaline` (+3.1), `synth_lead` (+2.7), `synth_silkpad` (+2.8), `synth_filterbank` (+2.3). Same picture as D121: the per row and per type cutoff factors of `p_table.rs` / `p_type.rs` are not replaced by a uniform `2 F` factor. The raw type 3 stage does not touch this, no clip uses it.
- `synth_12db` carries unison 3 in the file but plays SHAPE square, which ignores unison natively (and, per D140, the oracle lane `-2.0 dB`): its level error (`+6.65 dB`) is the row and the filter, not the unison.
- The unison weights remove most of the level error where the unison is the explanation (`synth_leaky` `-4.64 -> +1.05 dB`, `synth_phat3` `-3.86 -> -0.86 dB`: both within the `+- 2 dB` beat term). `synth_crunch` needs its SHAPE triangle row lowered by `3.5 dB` (D140) at the same time: `+2.55 + 3.0 - 3.5 = +2.0 dB`.

## Implications

- The native unison law is wrong against the firmware words by `3 .. 6 dB` per channel; it is fixed behind `--synth-unison-weights`. The flip waits for a clip pair that fixes the row of SHAPE triangle / MULTI / SUPER together (D22), because the fitted rows were tuned with the low unison level.
- The filter type factors stay EMPIRICAL: the rescore does not close the gap, and the stage `0x1257B2` is not on the clips.

## Follow-ups

1. A phase averaged oracle level for the unison words: poke the voice phases of the oscillator stage at several offsets and average the lane RMS (the native start phases are ASSUMED).
2. Capture `kOscUnison` to `(a, b)` from a loaded program (OQ22, [synth-p-writer](synth-p-writer.md) s6).
3. Bisect the `synth_sineplk` default score `4.73 -> 7.48` against the merged PRs since D121 (not this PR: every flag of this PR is off by default).
4. Lower the SHAPE triangle, MULTI and SUPER rows (D140) together with a unison default flip.
