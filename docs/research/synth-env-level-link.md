# Level link of the firmware envelope law (D236)

- **Checked:** 2026-11-23 (`main` @ `2b8560b7`)
- **Sources (local, never committed):** ORG core 2 oracle `sharc-sys` with the captures of Smooth Lead and Crunch Synth (`target/synth_env_cap/`), `tools/synth_env_level_link.py`, the Part A clips and the factory backup (`tools/emu_vs_hw.py`). Committed data: [synth-env-level-link.csv](data/synth-env-level-link.csv), [synth-env-level-clips.csv](data/synth-env-level-clips.csv). Background: [synth-env-oracle-trace](synth-env-oracle-trace.md), [synth-velocity](synth-velocity.md), [default-flip-audit](default-flip-audit.md), D200, D207, D223.
- **Question:** D223 kept `--synth-fw-env-law` opt-in because `synth_lead`, `synth_silkpad` and `synth_horizon` get 2 to 3 points worse and named the ASSUMED link between the verified envelope word and the engine level as the likely cause. Is that link wrong, and if so where?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Result

| Link | Result | Tag |
|---|---|---|
| Engine `level` = filter chain level times the velocity factor | Oracle peak of the cutoff index equals `amount_knob * F` semitones, `F = G / 128` (`G = 128` at `g = 127`) with `kFltEnvVel` on, `amount_knob` with it off; 20 rows (2 programs x 5 key bytes x 2 settings), max error 0.002 semitone, chain peak 1.0 in every row | VERIFIED |
| Key tracking before the law | additive in semitones at the oracle (`m0`, D207, [synth-cutoff-voice-law](synth-cutoff-voice-law.md)); a Hz multiply before the law is the same sum below the `R` clip. The three clip programs have `kb_track` 0 | VERIFIED (equivalence), not exercised by the clips |
| Cutoff the law hands to the filter | **Not the oracle `R`.** The strip path multiplies the law output by the EMPIRICAL row and type factors (`p_table.rs`, `p_type.rs`: SHAPE 1.75, SUPER 0.45 x LP12 1.4, SUPER x LP24 1.98, fitted on these very clips under the 96 semitone default law). The firmware has no such factor (D108, [synth-filter-links](synth-filter-links.md) s1) | EMPIRICAL, the real remaining link |

So the level link of the audit is correct; the regression comes from the fitted cutoff factors that were tuned on `synth_lead` (SHAPE, "Smooth Lead clip"), `synth_silkpad` and `synth_horizon` while the default sweep was 31 semitones short.

## 2. Method

`python tools/synth_env_level_link.py measure WORKDIR --sharc-sys ... --image ... --program NAME --amount-knob N --out CSV`: the scripts of `synth_env_trace.py report` (recall, program ADR, cutoff and amount words, key), plus the words the ARM writes for `kFltEnvVel` (strip 0 word 19 and strip 6 word 19 = `0.0` / `1.0` on, `1.0` / `0.0` off), the key byte `0x80 + g` replaced, taps of the filter chain `F-8`, `F-4`, `DM(0xC2723)` and `R`. Peak index over the 0.5 s hold minus the index before the key. Unit tests: `tools/tests/test_synth_env_level_link.py`, `fw_cutoff_env.rs::level_link_rows_equal_the_oracle_peak` (engine `fw_env_cutoff_hz` with `level = chain * filter_factor(g)` against the oracle peak).

## 3. Clip scores (`tools/emu_vs_hw.py`, velocity 100, 3 segments)

| Clip | default | law | `--synth-fw-filter` | law + fw-filter | law + `--synth-filter-law 1.0` |
|---|---|---|---|---|---|
| `synth_lead` | 0.80 | 3.52 | 3.50 | 1.47 | 1.57 |
| `synth_silkpad` | 1.36 | 3.95 | 4.28 | 6.70 | 6.63 |
| `synth_horizon` | 2.31 | 4.16 | 3.13 | 0.76 | 0.69 |

With the law, 3 to 8 kHz is +13.3 dB (lead), +7.1 (pad), +11.8 (horizon). Removing the fitted factors (the oracle unit) brings horizon to 0.69 (better than default 2.31) and lead to 1.5 (still worse than 0.80), and makes the pad much worse (+14 dB at 3 to 8 kHz). Velocity does not rescue it (law is 6 to 12 dB brighter at 40, 64, 100, 127; `synth_lead` has no velocity dependence).

## 4. Implications

- No engine change: the level link was right. The ASSUMED tag is replaced by VERIFIED with a test.
- `--synth-fw-env-law` plus `--synth-fw-filter` is the combination without an EMPIRICAL link; it is better on 1 of 3 clips and clearly worse on the SUPER LP12 pad. The pad needs a source side explanation (SUPER oscillator spectrum, filter input gain +5 dB of [synth-e2e-oracle-vs-native](synth-e2e-oracle-vs-native.md), unit 1.0147 ASSUMED) before the fitted 0.63 can go.
- **No flip proposed.** Not every clip segment is equal or better, and the unit `2 / 1.971` stays ASSUMED.

## 5. Follow-ups

- Pad: oracle audio of Silk Pad (SUPER wave 0, LP12) against native with the program words injected, to split oscillator spectrum, filter gain and unit.
- Direct capture of the hardware corner to settle `UNIT_2F`.
