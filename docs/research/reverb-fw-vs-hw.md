# Firmware reverb against the engine reverb and the hardware (types 2..11)

- **Checked:** 2026-11-17
- **Sources (local, never committed):** the FX Core 2 flat SRAM image (`python tools/ns4_dsp_ldr.py target/DSP/dsp_image_0_FX.bin --sram <dir>`), the oracle `sharc-fx`, `sharc-disasm`; derived numbers only. Repo tools: `cargo run --release -p nord-tools --bin fx_reverb_types` (new, [reverb_measure.rs](../../crates/nord-tools/src/reverb_measure.rs)), `python tools/fx_rvbmulti.py golden`. Builds on [fx-reverb-leslie](fx-reverb-leslie.md) s13, s14, [fx-reverb-colors](fx-reverb-colors.md), [fx-reverb0-chor-oracle-diff](fx-reverb0-chor-oracle-diff.md), [firmware-flag-audit](firmware-flag-audit.md). The hardware reference WAV `C:\tmp\ref\white-grand-cathedral3.wav` did not exist; no hardware audio was used.
- **Question:** why does `--firmware-reverb` (A:21 White Grand, Cathedral, knob 3, one held key, velocity 88) sound worse by ear than the engine FDN against the hardware video, and is the +21 / +26 / +36 dB (sub / brilliance / air) excess of a firmware Cathedral pad render a port or harness bug?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Item | Result | Tag |
|---|---|---|
| Port against the oracle over the whole tail | `FwReverbMulti` (40-bit) equals the oracle word for word over **30,000 calls** (2.76 s, longer than every tail), impulse at amount 127 and noise at 63, all nine types: hash of all output words, non-zero count. The old window (3,600 calls) ended before the Hall / Cathedral tails began (first output at call 3,025). The CHOR bug class of D133 (a wrapped store beyond the golden window) is excluded for the 3,140 word ring and the 20,480 word line | VERIFIED (`tests_fw_rvbm_long.rs`, [data/fx-rvbmulti-golden-long.csv](data/fx-rvbmulti-golden-long.csv), regenerated locally and equal) |
| Gain of the routine | Linear in level (power gain equal within 0.1 dB between amplitude 0.01 and 1.0), **no gain above unity**: wet-only power gain of noise, left channel, per band (dB): below 60 Hz -4.6 to -7.7 (type 6 is not ported), 60-250 Hz -3.8 to -7.5, 0.5-2 kHz -7.9 to -10.0, 4-8 kHz -20 to -24, 8-16 kHz -34 to -38 | VERIFIED (port = oracle) for the routine, MEASURED (`fx_reverb_types`, `tests_fw_rvbm_levels.rs`) |
| The +21 / +26 / +36 dB excess | **Not a gain bug.** The routine cannot add more than -4.6 dB (sub) or -34 dB (air) of its input. A noise floor of the packed 16-bit delay words is **consistent with** the excess: the routine is not time-invariant, and a pure 220 Hz tone leaves, relative to its power, sub -51 / brilliance -61 / air -70 dB at mix 1 and -68 / -77 / -83 dB at mix 0.3 for Cathedral (Room reaches -37 dB in the sub band at mix 1). A pad whose dry spectrum is 90 to 115 dB below its peak in those bands would read such a floor as +21 to +36 dB "relative to dry". The pad render was not reproduced and the floor is asserted only loosely by `tests_fw_rvbm_levels.rs`, so this is a hypothesis, not a proof; whether the floor is audible depends on the dry signal (it is well below the tone at -50 dB or lower, but Room's sub floor is not negligible) | EMPIRICAL (consistent with the figures, not proven) |
| Why it sounds worse (by number) | At the same type and knob the firmware reverb is **quieter, later, shorter and darker** than the engine FDN: wet energy -15 dB at knob 0.3 (Hall), first echo 180 to 252 ms (Hall, Cathedral) against 60 to 77 ms, T60 1.2 s against 3.4 s and 7.9 s, 8-16 kHz -34 dB against -1.7 dB. Hall and Cathedral have **the same firmware tail** (1.20 and 1.19 s) | MEASURED (section 2) |
| Parameter feed | Type, colour, amount words and the 4-frame block call are consistent with the oracle harness; no feed error found. Two items cannot be checked here and are the open candidates for the audible gap (section 4): the stage input gain of the reverb record in the real dispatcher, and the second routine `0x1211C1` of the same module | open |
| Fix | **None needed in the port.** No default flips (D22). New: the long-window parity test, the level test, the measurement tool | - |

Limits of the parity test: it checks the E40 (40-bit) port only, noise runs only at amount 63 and impulse at 127, and the golden CSV needs the local firmware FX image (`tools/ns4_dsp_ldr.py --sram` plus `sharc-fx`), so CI cannot regenerate it; `tests_fw_rvbm_long_data.rs` is checked against the committed CSV by a test, the CSV against the oracle only locally.

## 2. Per-type measurements (48 kHz, `fx_reverb_types`, deterministic noise and impulse)

Wet only (mix 1.0, `kRvbAmt` 127) of the left channel, firmware routine (`fw`) against the engine FDN (`fdn`). First echo = first sample above 5 % of the peak; E = energy of the response after the dry impulse, dB re input energy; T60 = Schroeder backward integral, -5 to -25 dB, times 3 (total, then per octave band 60-250, 250-500, 500-1k, 1-2k, 2-4k, 4-8k Hz).

| Type | Path | First echo (ms) | E (dB) | T60 (s) | T60 per band (s) | Noise gain below 60 / 60-250 / 0.5-2k / 4-8k / 8-16k Hz (dB) |
|---|---|---|---|---|---|---|
| Booth | fw | 81.0 | -20.8 | 0.56 | 0.58 0.60 0.53 0.56 0.55 0.55 | -5.9 -5.2 -10.0 -23.7 -37.4 |
| Booth | fdn | 16.9 | -12.0 | 0.18 | 0.23 0.28 0.22 0.23 0.21 0.16 | -1.7 -1.5 -1.7 -1.7 -1.8 |
| Booth Predelay | fw | 169.6 | -20.8 | 0.56 | 0.56 0.60 0.55 0.55 0.55 0.55 | -6.2 -5.2 -10.0 -23.7 -37.6 |
| Room | fw | 102.2 | -18.9 | 0.75 | 0.69 0.77 0.75 0.71 0.74 0.72 | -4.6 -3.8 -7.9 -22.0 -35.8 |
| Room | fdn | 29.1 | -11.4 | 0.60 | 0.80 0.81 0.75 0.66 0.54 0.36 | -1.9 -1.4 -1.4 -1.7 -1.7 |
| Room Predelay | fw | 226.9 | -18.9 | 0.69 | 0.68 0.68 0.68 0.71 0.72 0.71 | -4.9 -3.9 -7.9 -21.8 -35.6 |
| Stage (type 6) | fw = fdn | 43.1 | -10.9 | 1.55 | 1.99 2.04 1.93 1.59 1.13 0.67 | not ported, engine FDN |
| Stage Chorale | fw | 101.7 | -18.3 | 0.90 | 0.91 0.91 0.89 0.89 0.91 0.91 | -5.3 -4.1 -7.5 -20.4 -34.0 |
| Hall | fw | 179.9 | -19.4 | 1.20 | 1.20 1.18 1.25 1.15 1.19 1.19 | -7.5 -7.0 -8.9 -20.5 -34.2 |
| Hall | fdn | 59.7 | -10.4 | 3.44 | 4.64 4.50 3.96 3.17 2.05 1.09 | -0.5 -0.9 -0.5 -1.6 -1.7 |
| Hall Chorale | fw | 179.9 | -19.4 | 1.19 | 1.14 1.16 1.20 1.14 1.18 1.20 | -7.3 -7.3 -8.8 -20.4 -34.0 |
| Cathedral | fw | 252.2 | -19.4 | 1.19 | 1.20 1.18 1.23 1.17 1.18 1.18 | -7.7 -7.4 -9.1 -20.7 -34.3 |
| Cathedral | fdn | 77.2 | -9.7 | 7.97 | 10.11 9.67 8.40 5.98 3.51 1.61 | -0.1 -0.5 -0.7 -1.6 -1.7 |
| Cathedral Chorale | fw | 252.2 | -19.6 | 1.21 | 1.17 1.19 1.24 1.20 1.20 1.21 | -7.7 -7.5 -9.2 -20.7 -34.3 |

(The remaining fdn rows repeat the model: the FDN has no variation delay except Chorale modulation; Room fdn T60 0.60 s, Stage 1.55 s.) EMPIRICAL: numbers of the ported routine on the synth unit L6 row at the engine rate 48 kHz; the routine runs at 43,402 Hz in the firmware, so its times are 10.6 % too short at 48 kHz (`fw_rate_match` is not applied here).

At the knob of the listening test (mix about 0.3, `kRvbAmt` 38: send word 0.153, dry word 0.988) the wet energy is -35.8 dB re the input energy for the firmware Hall and -20.8 dB for the engine Hall: **15 dB less wet**; `tests_fw_rvbm_levels.rs` asserts 12 to 18 dB. This is why "mix on / off barely changed it": the firmware wet at knob 3 is below -35 dB of the dry signal.

Shape (EMPIRICAL): the firmware tail is flat across bands (no frequency-dependent decay, unlike the FDN's damping), and the wet spectrum falls 5 to 7 dB below 250 Hz, 8 to 10 dB at 0.5 to 2 kHz, 20 to 24 dB at 4 to 8 kHz and 34 to 38 dB at 8 to 16 kHz (a fixed low-pass of the fs/4 biquads and the tap table), so the reverb is dark and the tail is dull rather than bright and long.

## 3. What was checked in the parameter feed

| Item | Finding |
|---|---|
| Type | `fw_multi_type`: model `t / 2`, variation `t & 1`, VERIFIED names; type 6 (Stage, no variation) falls back to the FDN (fw = fdn rows above) |
| Colour | `set_color` writes the ARM delta words; Dark / Bright move the tail by under 15 % (type 7 up to 36 %), [fx-reverb-colors](fx-reverb-colors.md) |
| Amount | `kRvbAmt` words `reverb_send(ui)` / `reverb_dry(ui)`: 127 is wet only, 38 is send 0.153 and dry 0.988 (VERIFIED words, ASSUMED role) |
| Call count and block | one call of 4 stereo frames per 4 engine frames, 4 frames of latency; the oracle golden calls the routine the same way (`IN` 8 words, in place). In the live chain the harness once called the stage 35 times per pass (D133); the real dispatcher calls each stage once |
| Sample rate | the routine runs at 48 kHz instead of 43,402 Hz: times 10.6 % short, the three fs/4 biquads at 12 kHz instead of 10.85 kHz. Not the cause of a 6 times tail gap |
| Input gain | 1.0 in the port. In the real chain the stage record carries a gain word (`clip(level + step) + 0.5`, [fx-oracle-live](fx-oracle-live.md) s2) written by the ARM per layer and reverb mode; its value for the reverb stage is **not known** (candidates: the bus scale cells 2.5 and 3.25 of [piano-receiver-scale](piano-receiver-scale.md) s6 with a global reverb) |
| Mono / stereo | the block layout is the oracle's (words 0, 2, 4, 6 one channel, 1, 3, 5, 7 the other); the hardware's channel assignment of those words is not independently verified (only the type 0 live chain, D133, agrees on it with the harness) |
| Second routine | the module has two routines in the ARM directory: `0x120FDB` (the port) and `0x1211C1` (size 1, 17 instructions, [data/fx-chain-install.csv](data/fx-chain-install.csv) rows `0xc0549e98`, `0xc0549ea8`): it reads two input words, calls the same core `0x120D8A`, mixes `dry * c[-28] + wet`, scales by `c[42]`, `c[43]` and writes two words back. When it runs (reverb mode, layer, or per pass in addition to `0x120FDB`) is not known; the port never runs it |

## 4. Reading

1. **No port or harness bug explains the by-ear gap.** The routine equals the oracle over its full tail, adds no gain and has a noise floor below -50 dB re signal. The +21 / +26 / +36 dB are the floor against a spectrally empty dry pad (EMPIRICAL, same algorithm on the hardware).
2. The audible differences are structural: firmware Hall and Cathedral are one 1.2 s tail with 180 to 252 ms pre-delay and 15 dB less wet at knob 3; the engine is 3.4 and 7.9 s with 60 to 77 ms. The owner hears the native version as closer to the hardware video, which for a Cathedral with a tail of seconds is plausible. It would be **wrong** to conclude that the hardware tail is the FDN's: the engine tails are ASSUMED (OQ21c), and a firmware that stored a 1.2 s Cathedral would be unusual. The more probable reading is that the ported routine on the **synth unit L6 instance** is not what the piano unit runs, or is not the only part of the reverb that runs (open items below).
3. The older piano evidence agrees with a long hardware tail: the default emulator (engine FDN) matches the hardware piano tail median 1.00 over nine octave groups ([piano-decay-attack](piano-decay-attack.md)), but that measures the string decay mostly, not the reverb.

## Implications

- `--firmware-reverb` stays opt-in (D22, D31, D136): no evidence that it is closer to the hardware, and the by-ear verdict is against it. The wet level at knob 3 and the tail are the two measurable reasons.
- The next step to settle the hardware tail is not more port work but a single-note recording with a clean release (Cathedral, amount 127) and the real stage gain.

## Follow-ups

1. Trace the stage record of the reverb module in the real dispatcher: the ARM writes of the gain words (`S - 0x1C`, `S - 0x1B`) for the piano and synth reverb stage (boot log `target/bulk/boot_log.jsonl`, `tools/fx_chain_install.py`), and the condition that picks `0x1211C1`. Run the multi-tap family through `tools/fx_oracle_live.py` (add a `rvbm` family: instance row of core 1 `0xC1AC8`, type words, `fx_live_native` counterpart) to compare the full chain.
2. A hardware recording of a held key with release into Cathedral at knob 127 (and 38): T60 per band, first echo, wet level. Then decide between the firmware tail (1.2 s) and an engine retune.
3. The wet level law: the same knob gives 15 dB less wet through the firmware words than through the engine; once the stage gain is known, re-run `fx_reverb_types --mix 0.3`.
4. Type 6 (Stage), the organ rows and the 43,402 Hz time scale stay as in OQ21c.

## Reproduce

```bash
cargo run --release -p nord-tools --bin fx_reverb_types            # mix 1.0, add -- --mix 0.3
python tools/ns4_dsp_ldr.py target/DSP/dsp_image_0_FX.bin --sram <dir>
python tools/fx_rvbmulti.py golden <dir>/core2_sram.bin --calls 30000 --bin target/release/sharc-fx > long.csv
cargo test --release -p nord-core --lib tests_fw_rvbm
```
