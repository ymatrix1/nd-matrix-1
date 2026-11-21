# FX default words: flip evidence for `--fx-defaults firmware`

- **Checked:** 2026-11-21
- **Sources (local, never committed):** the ARM boot log `target/bulk/boot_log.jsonl` (6,264 FX writes of the booted ARM, phases boot and settle 1 to 3), the ARM edit table `docs/research/arm-edit-params.csv`, the firmware port of every ported block (oracle checked word for word, [fx-vs-oracle](fx-vs-oracle.md)). New tools: `tools/fx_defaults_boot.py` (boot log), `cargo run --release -p nord-tools --bin fx_defaults_vs_fw` (render). Data: [fx-defaults-flip.csv](data/fx-defaults-flip.csv). Builds on [fx-defaults](fx-defaults.md) (D101), [fx-compressor-row](fx-compressor-row.md) (D187).
- **Question:** OQ34 / [audio-remaining](audio-remaining.md) row 14: for each of the 8 differing default words, does the firmware default match the firmware better than the engine default, and does the hardware start from it?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Word | Engine / firmware | Start state sent by the ARM boot | Render against the firmware model | Safe to flip alone |
|---|---|---|---|---|
| `comp_amount` | 0 / 0.748 | **`kCmpAmt` = 95 in every compressor row** (= 0.748 x 127) | firmware closer (band error 1.7 dB noise, 34 dB notes, against 0) | yes |
| `mod1_amount` | 0.40 / 0.504 | 6 of 6 (type 0) and 8 of 8 (type 1) amount-64 words written; none of the amount-44 / 51 words | firmware closer (0.3 to 1.0 dB) | yes |
| `mod2_amount` | 0.35 / 0.504 | type 0: 2 of 3 amount-64 words written, 0 of 3 for 44 / 51 | firmware closer (0.8 to 1.5 dB) | yes |
| `reverb_mix` | 0.15 / 0.299 | **wet word of `kRvbAmt` = 38 written** (0.299 x 127), not the one of 19 | **engine closer**: the engine reverb is already louder than the firmware (D91, +9 to +17 dB), a larger amount widens the gap | **no**, not before a reverb port or a level correction |
| `delay_time_ms` | 375 / 250 | no delay words in the boot (delay not installed) | firmware closer (0.4 to 3.6 dB) | yes, start state ASSUMED |
| `delay_mix` | 0.25 / 0.504 | same | firmware closer (7 to 9.6 dB) | yes, start state ASSUMED |
| `delay_feedback` | 0.35 / 0.504 | same | firmware closer (0.7 to 1.6 dB) | yes, start state ASSUMED |
| `eq_mid_freq_hz` | 1000 / 999 | not run | 0.1 % apart (`EQ_FREQ_HZ[64]`) | no effect |

Seven words are safe alone; `reverb_mix` is not. The defaults reach audio only for an engine with no program (`apply_fx_chain` overwrites every field on a load, [fx-defaults](fx-defaults.md) s3).

## 2. Boot log: does the start program send the default words? (`tools/fx_defaults_boot.py`)

The words of a knob are a function of its UI value, so the value the ARM used is recovered from the FX writes of the boot log. Output of the run:

```
kCmpAmt sent by the boot: [95] (firmware default word 95)
kRvbAmt wet words in the boot: [38, 127] (firmware default word 38)
Mod 1 type 0: {44: (6, 0), 51: (6, 0), 64: (6, 6)}
Mod 1 type 1: {44: (8, 0), 51: (8, 0), 64: (8, 8)}
Mod 2 type 0: {44: (3, 0), 51: (3, 0), 64: (3, 2)}
Mod 2 type 4: {44: (5, 0), 51: (5, 0), 64: (11, 0)}
```

`(n, k)`: n amount-dependent words of `fx_live_install` for that amount, k of them written by the boot. The engine defaults (compressor 0, Mod 1 0.40 = 51, Mod 2 0.35 = 44, reverb 0.15 = 19) are never the values sent; the firmware defaults always are. `kRvbAmt` 127 also appears (another reverb row, not traced). Tags: words sent VERIFIED (boot log); that the boot program starts from the registry default and not from a stored program amount is ASSUMED (the compressor amount equals the default word exactly, which a stored program would not do by chance).

Not covered: delay. The delay row is not installed at boot (the time words `c[0x15]` of [fx-delay-time](fx-delay-time.md) never appear), so mix, feedback and time have the default words VERIFIED and the start state ASSUMED.

Not among the 8: the boot writes the Mod 2 **type 0** (PHAS) amount words and none of type 4 (CHOR), so the firmware default `kFx2Type` is 0, not the engine's 4 (D101 lists it as differing and does not apply it).

## 3. Render: engine default against firmware default (`fx_defaults_vs_fw`)

Reference: the firmware port of the block (`FirmwareFx::ALL`) driven by the firmware word. Candidates: the default chain (`FirmwareFx::DEFAULT` = parity, D173) with the engine word and with the firmware word. Each word is varied alone, the rest at firmware defaults. Stimuli: noise, held notes, impulse (1.5 s at 43.4 kHz). Band error = rms over the octave band gains re the model; null = aligned null depth (dB re the model).

| Word / variant | Engine / firmware value | rms delta engine (noise, dB) | Band error engine (noise / notes, dB) | Null engine (noise / notes / impulse, dB) | Firmware default | Closer |
|---|---|---|---|---|---|---|
| `mod1_amount` type 0 | 0.400 / 0.504 | 0.59 | 0.72 / 0.95 | -20.4 / -20.9 / -23.3 | 0.00, null -320 | firmware x3 |
| `mod1_amount` type 1 | 0.400 / 0.504 | 0.28 | 0.28 / 0.80 | -22.7 / -22.0 / -23.3 | 0.00, null -322 | firmware x3 |
| `mod2_amount` PHAS | 0.350 / 0.504 | 0.61 | 1.10 / 1.26 | -15.9 / -6.7 / -57.0 | 0.00, null -324 | firmware x3 |
| `mod2_amount` CHOR | 0.350 / 0.504 | -0.38 | 0.79 / 1.47 | -13.8 / -12.0 / -14.6 | 0.00, null -324 | firmware x3 |
| `comp_amount` | 0.000 / 0.748 | -1.67 | 1.68 / 34.17 | -15.1 / -10.9 / 0.0 | 0.00, null -325 | firmware x3 |
| `delay_time_ms` | 375 / 250 | -0.05 | 0.43 / 3.58 | 0.6 / -0.2 / 0.6 | 0.00, null -327 | firmware x3 |
| `delay_mix` | 0.250 / 0.504 | -3.32 | 7.03 / 9.60 | -4.9 / -5.4 / -4.9 | 0.00, null -327 | firmware x3 |
| `delay_feedback` | 0.350 / 0.504 | -0.38 | 0.72 / 1.63 | -16.7 / -16.8 / -16.6 | 0.00, null -327 | firmware x3 |
| `reverb_mix` type 2 | 0.150 / 0.299 | -0.05 | 0.24 / 2.95 | -19.3 / -22.1 / +6.0 | 0.53 / 5.48 band, null -17.6 | engine x3 |
| `reverb_mix` type 5 | 0.150 / 0.299 | -0.05 | 0.43 / 3.39 | -19.1 / -19.2 / +5.8 | 0.64 / 1.98 band | engine, firmware (notes), engine |
| `reverb_mix` type 7 | 0.150 / 0.299 | -0.05 | 0.21 / 2.03 | -19.0 / -19.2 / +6.1 | 0.34 / 5.09 band | engine x3 |

Reading, with the limit stated:

- For the ported blocks (Mod 1, Mod 2, compressor, delay) the firmware default nulls to float noise (-290 dB or lower) by construction: the chain with the firmware word is the model with the firmware word (that the port equals the SHARC oracle is [fx-vs-oracle](fx-vs-oracle.md)). The table therefore measures the distance the flip moves the audio (0.3 to 9.6 dB band error); what says the firmware word is the right one is the boot log of s2, not this render. Tags: port against oracle VERIFIED, word to unit conversion VERIFIED ([fx-defaults](fx-defaults.md) s1), start state VERIFIED for compressor, reverb, Mod 1 and Mod 2, ASSUMED for delay.
- For reverb the default chain runs the engine reverb (the reverb ports are out by owner decision, [firmware-flag-audit](firmware-flag-audit.md)); engine and model differ by the known level gap, so the engine amount that sounds quieter lands closer to the model. The word is right (the boot sends 38); the model gap is not the default's to fix. Flip it together with a reverb port or a level correction. Tag: ASSUMED that the gap, not the word, explains the render.
- The compressor row measures the amount only; with `comp_enabled` false in the default chain a bare engine stays inaudible there either way.
- `eq_mid_freq_hz` (999 / 1000 Hz) is 0.1 %: not rendered.

## Implications

- Flip proposal: D199. Seven of the eight words may flip alone; `reverb_mix` stays at 0.15 until the reverb path matches the firmware level.
- OQ34 narrows from "does the hardware start from the words" to "does the delay start from them" (no boot words); no hardware init-patch clip is needed for the other words, one with the delay on settles the rest.
- `kFx2Type` default 0 (PHAS) is a candidate ninth word, not applied.

## Follow-ups

1. Owner decides the flip (D199); `render_hash` needs a re-baseline only if a bare-engine hash exists.
2. Trace what writes `kRvbAmt` 127 in the boot.
3. A delay-on init-patch clip, or the delay install path of the boot, for the three delay words.

## Reproduce

```bash
python tools/fx_defaults_boot.py target/bulk/boot_log.jsonl --install target/release/fx_live_install
cargo run --release -p nord-tools --bin fx_defaults_vs_fw -- --out docs/research/data/fx-defaults-flip.csv
```
