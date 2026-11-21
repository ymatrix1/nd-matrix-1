# Amp types 7, 8, 9 (To Rotary, LP24, HP24): module, routine and equations

- **Checked:** 2026-11-21
- **Sources (local, never committed):** FX core 1 and core 2 flat SRAM images (`C:/tmp/h/img_0_FX/core{1,2}_sram.bin`, built with `tools/ns4_dsp_ldr.py` from the Phase 0 firmware unpack), `target/fw_unpacked/bank0_CODE.bin` + `bank0_SRAM.bin` (OS v1.66) with the unicorn ARM harness (`tools/fx_clients.py`, `arm_edit_capture.py`, `arm_steady.py`), `sharc-fx` oracle (`tools/fx_oracle.py`), [fx-type-map](fx-type-map.md), [fx-chain-install](fx-chain-install.md), [arm-param-messages](arm-param-messages.md). Derived numbers only: [data/fx-lp24-oracle-cases.json](data/fx-lp24-oracle-cases.json), [data/fx-lp24-words.csv](data/fx-lp24-words.csv).
- **Question:** Which module handles Amp types 7 (To Rotary), 8 (LP24) and 9 (HP24), which routine runs them, and can the emulator reproduce LP24 / HP24 word for word?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Item | Result | Tag |
|---|---|---|
| Module | the Amp unit's layer switch (`0xC021D064`, [fx-chain-install](fx-chain-install.md) s5) installs **chain rows** for types 7, 8, 9: a routine pc plus link words, then a mode word through `0xC01F496C`; the mode word handler is `0xC01F4844` | VERIFIED (ARM harness capture, `amp_all.csv`) |
| LP24 / HP24 routine | one **state variable filter pair** at `0x124EFA` (FX core 1) / `0x124586` (core 2, relocated copy, same code); the mode word at `CTX - 4` selects LP (0) or HP (1) | VERIFIED (oracle) |
| Equations | section 2; model against `sharc-fx`: worst output error `4.7e-07` over 8 random scenarios (4 LP, 4 HP, 4 frames each), `python tools/fx_model_lp24.py check` | VERIFIED |
| To Rotary | the type 7 install is the **Leslie family** (`0x125371`, `0x121BBA`, `0x124E0B`, `0x125142`, core 2 `0x1247CE` / `0x12478F`, `0x122857`), not one routine; `0x125142` itself is a fixed fs/4 Butterworth bank (PM `0x9437A`, ROM constants); **not modelled, no port**, section 8 (D195) | EMPIRICAL (identity), VERIFIED (PM words are ROM) |
| Organ unit instance of the tone stack `0x1243F4` | PNO core 1 `0x125261` | VERIFIED (pc relocation, [fx-type-map](fx-type-map.md) s2) |
| ARM word laws | cutoff (`kAmpMidFreq` 0..127) writes the cutoff target word at unit DM `0xC1A08` (`CTX - 32`): `6.21e-5` at 0 doubling every 12 steps (`2^(i/12)`, measured ratio `1.05950` against `1.05946`) to `0.0506` at 113 | EMPIRICAL (114 sample points of the capture) |
| Template constants | `L = c[-25] = 1.0`, `c[-24] = 0.5`, `c[-23] = 0.25`, `c[-3] = 2` (FX core 1 unit 0 context `0xC1A28`, DM dump `0xC1A0F..0xC1A11`, `0xC1A25` of the booted image: `sharc-fx core1_sram.bin --call 0x1231CB --until 0x1231CD --dump 0xC1A00:0x48`) | VERIFIED |
| Words | all 128 `kAmpMidFreq` words (`c[-32]` = DM `0xC1A08`) and all 121 `kAmpMid` word pairs (`c[-29]` = `0xC1A0B`, `c[-28]` = `0xC1A0C`) captured on a type 8 unit: [data/fx-lp24-words.csv](data/fx-lp24-words.csv); the filter list `0xC044398C` carries the same words for every Amp type | VERIFIED (capture) |
| Rust port | `fx/fw_lp24.rs` + `fw_lp24_tables.rs`, opt-in `--firmware-fx-ports lp24` (`FirmwareFx::lp24`), oracle golden `tests_fw_lp24_golden.rs` (8 scenarios, both channels, worst deviation under `2e-5`); latency 4 frames; D188 | VERIFIED (golden) |

## 2. LP24 / HP24 equations (VERIFIED)

Context floats at `CTX + o` (`CTX = I1`): `c[-32]` cutoff target `Ta`, `c[-31]` cutoff state `A`, `c[-29]` resonance target `Tb`, `c[-27]` resonance state `B`, `c[-28]` / `c[-26]` the pre-gain target and state, `c[-25] = L` (clip limit), `c[-24]`, `c[-23]` template constants, states at `c[-16, -14, -12, -10, -8, -6]`, mode word `c[-4]`. `K = 0.99078` (`0x3f7da39e`, the smoothing factor of the other FX filters).

```text
A2 = Ta + K (A - Ta)        B2 = Tb + K (B - Tb)       X = clamp(16 A2, -4, 4)
C  = c[-28] + K (c[-26] - c[-28])       g2 = 2 C
s  = L + 4 X B2 + X^2       inv = 1 / sqrt(s)          a1 = 2 X inv      b = 4 B2 inv
```

Per sample (clip `clip(v) = clamp(v, -L, L)`), mode 0 (LP24: two cascaded stages, the first scaled by `g2`):

```text
stage(x, g, s1, s2, s3):  F = x g;  t3 = s3 + s1 a1;  e = (F + s2) / 2 - t3 - s1 b
                          s3' = t3;  s2' = F;  s1' = clip(s1 + e a1);  return (t3 + s3) / 2
y = stage(stage(x, g2, c[-16], c[-14], c[-12]), 1, c[-10], c[-8], c[-6])
```

Mode 1 (HP24: two cascaded two state stages, `h = L - (b a1 c[-24] + a1^2 c[-23])`):

```text
st2(x, s1, s2):  F13 = s2 + s1 a1;  F9 = x h - F13;  e = F9 - s1 b
                 s2' = F13;  s1' = clip(s1 + a1 e);  return (F9, e)
(_, e1) = st2(x, c[-16], c[-14]);   (y, _) = st2(e1, c[-10], c[-8])
```

The two SIMD lanes (words `k`, `k + 1`) run the same recursion on left and right with shared coefficients. The first model run missed `C` (a 0.9 % gain error at `g2`); the smoothing of `c[-26]` toward `c[-28]` fixed it, after which outputs and all six final states match to about `1e-6`. The right channel uses the odd state offsets (`c[-15, -13, ...]`) with the same coefficients; the model and the oracle agree on both channels (worst error `6.5e-07`). The template values `L`, `c[-24]`, `c[-23]`, `c[-3]` are in section 1 (read from the booted FX core 1 image; the pass-boot of `tools/fx_oracle.py` stops at the step limit on that image, a plain `--call` without the pass arguments does not). The unit context is on **core 1** (`CTX = 0xC1A28`, mode word `c[-4]` = `0xC1A24`, written by pc `0xC01F496C`); the chain rows `0xC1A00..0xC1A23` seen in the capture are the core 2 copy.

## 3. How a type switch reaches the routine (VERIFIED capture, EMPIRICAL reading)

`Edt_Effects_kAmpTypeNew` = 7 / 8 / 9 on unit 0 sends the layer switch's DSP words (`amp_all.csv`, one row per `(base + 4 off, 0, value)` word with the ARM pc that wrote it): the chain rows (routine pc, links), the mode word and the template. Types 8 and 9 install the same routine; only the mode word differs (`0` / `1`), so LP24 and HP24 share state and coefficient code. `kAmpMidFreq` and `kAmpMid` (resonance, -60..60) then write the target words `c[-32]` and `c[-29]` that the routine smooths with `K`. Target DSP is PNO core 2 (EMPIRICAL, [arm-param-messages](arm-param-messages.md)); DM word `w` of that core is `0x28800000 + 4 w`.

## 4. Closure of the other OQ31 items

| Item | Result |
|---|---|
| Synth filter type 3 "LP+HP" | The stage is `0x1257B2` and is already ported behind `--synth-stage3` ([synth-filter-t3](synth-filter-t3.md), 1840 / 1840 oracle words). The firmware has no string table for the type names ([oled-strings-vs-firmware](oled-strings-vs-firmware.md) lists the filter type names as not in the string pool); the display words were not located. |
| Piano Clav EQ (`kEq` 6, 7) | **Superseded 2026-11-21 (D206).** The consumer is the voice-frame builder `0xC00AA520`: the `kEq` handler stores a timbre mode, modes `0x10..=0x1A` become a per-note gain (VERIFIED 108 of 108, [piano-timbre-gain](piano-timbre-gain.md)), opt-in `--firmware-piano-timbre`. The Clav class maps `kEq` to modes below `0x10` (unity there); the rocker audio stays open. |

## 5. Rust port (opt-in)

`fx/fw_lp24.rs` runs the routine on four frames per call (smoothing once per call, then the 4 frames, two channels), `fw_lp24_tables.rs` holds the captured words. `LayerFxChain::process_fw_lp24` (`fx/amp_eq.rs`) replaces the two engine biquad pairs for `AmpCabinet::LpFilter` / `HpFilter` when `FirmwareFx::lp24` is on (`--firmware-fx-ports lp24`; not in `ALL`, `PARITY` or `PORT_NAMES`). The cutoff index is the nearest `EQ_FREQ_HZ` step of the engine's Mid Freq, the Mid counts are `res / 10 * 120 - 60`. Real time: no allocation, no lock. Checks: oracle golden (both channels), shape tests (LP / HP direction, bounded states at full resonance, the glide of the smoother), chain tests (opt-in, latency, other amps untouched). No hardware clip of an LP24 / HP24 program exists yet, so the flag stays opt-in (D22 style).

## 6. Type 9 words and the multi-call golden (VERIFIED)

- **Type 9 words.** The ARM sweep of `kAmpMidFreq` (0..127) and `kAmpMid` (-60..60) was repeated on a type 9 (HP24) unit: all 249 indices send the same words as type 8 (`data/fx-lp24-words-type9.csv` against `data/fx-lp24-words.csv`, `tools/tests/test_fx_lp24_capture.py::test_type_9_words_equal_type_8_at_all_249_indices`). The D188 assumption "type 9 equals type 8" is now VERIFIED.
- **Multi-call golden.** `tools/fx_lp24_capture.py golden` runs the real template (cutoff and resonance smoothers 0, pre-gain smoother 1.0, filter states 0; `template` checks the words against the FX core 1 image) with the table words through `sharc-fx` for 6 scenarios (LP at `kAmpMidFreq` 100 / 110 / 95, HP at 64 / 90 / 50; `kAmpMid` 0 / -60 / 40) of 24 calls with the state carried and seeded noise input. The stateful Python model and the Rust stage (`tests_fw_lp24_calls.rs`) agree with the oracle to better than 1e-5 on every output word (peaks 0.26 to 1.4, so not quiet). The single-call cases of D188 start from random states; this one covers the smoother glide and the state write-back (c[-31], c[-27], c[-26], six states per channel).

## 7. Program ranking: what the flip would change, and what it proves

`program_rank --amp-filters --flip fx_amp_lp24` renders the 26 factory programs with an enabled LP24 or HP24 amp (key 60, velocity 100, as [program-ranking](program-ranking.md)) with the engine default and with `lp24` on, and scores both against the all-firmware render (which now carries `lp24`). `tools/program_rank_amp.py` joins them ([data/program-ranking-amp-lp24.csv](data/program-ranking-amp-lp24.csv)).

| | Mean score | Median | Improved by more than 0.5 | Unchanged | Worse |
|---|---|---|---|---|---|
| Default (engine biquads) | 5.29 | 2.46 | | | |
| With `lp24` | 1.59 | 1.33 | 13 | 13 | 0 |

Largest moves: Unclocked Pad 46.45 to 0.87, Dance Lead 27.93 to 0.87 (both HP24), Ghost Melody 8.72 to 3.44, Bell Strings 5.06 to 2.50, HP Splash Whl 3.62 to 0.00. The residual (Ghost Melody 3.44, Marimba Pad 3.46 unchanged) comes from other groups in which the reference differs from the default (reverb ports, organ tone source, synth filter engine); not analysed per group here.

**What this proves and does not.** The score is the distance of one of our renders to another render of the same engine with every verified port on. It proves the wiring (LP24 / HP24 layers now carry the verified routine) and how far the engine biquads sit from it. It does **not** prove the sound matches the hardware: the oracle verifies the routine word for word (D188, s6), but no hardware clip of an LP24 / HP24 program exists, and three inputs stay ASSUMED: the Mid knob to `kAmpMid` counts mapping, the engine Mid Freq to table index, and the rate scale (the stage runs at the engine rate here, the firmware at 43,402 Hz; the rate match of D180 is applied through the same path as the drive stage).

**Proposed, not made:** switch `FirmwareFx::lp24` on by default (into `PARITY`) once one hardware clip of an LP24 or HP24 program, e.g. Unclocked Pad or Dance Lead (largest before score), confirms the cutoff unit and the Mid mapping. Until then it stays opt-in (D193).

## 8. To Rotary (type 7): assessment, no port (D195)

Checked 2026-11-21. Sources: `amp_all.csv` (ARM harness capture of the type switch, local), FX core 1 image listing (`sharc-disasm C:/tmp/h/img_0_FX/core1_sram.bin 0x24A206 0x24A290`; byte offset = 2 x pc), [fx-dsp-map](fx-dsp-map.md) s6, [b3-leslie-install-hunt](b3-leslie-install-hunt.md).

| Finding | Evidence | Tag |
|---|---|---|
| Type 7 vs type 8 differ in 42 / 54 captured words (type 7 only / type 8 only), all of them chain rows and element constants, none of the filter words | set difference of `(dm_word, word)` over the capture | VERIFIED (capture) |
| Type 7 rows carry `0x125371` (nonlinear resonant filter) and `0x121BBA` at `0xB047A / 0xB0482`, `0x1247CE` / `0x12478F` with `0x122857` at `0xC1A02..0xC1A23`, `0x124E0B` + `0x125142` + the identity `0x1231CB` at `0xC1AA1..0xC1AA3` | same | VERIFIED (capture) |
| `0x125103` (63 instr.): averages the input with the previous frame, multiplies by `(DM[I6 - 29] + S) / 2` and a state product, `MAX(., 0)` clamp, stores the pair | listing | EMPIRICAL (reading) |
| `0x125142` (216 words): average the pair, then three 4-frame TDF II biquads with PM `0x9437A..0x9437E` | listing; coefficients `0.292893 0.585786 0.292893 0 0.171573` verified in fx-dsp-map s6 (`fx_biquad_check.py`, bit identical) | VERIFIED (words are ROM, same table as `0x120FDB`) |

**Result.** The words of `0x125142` are not missing: they are the fixed fs/4 Butterworth ROM table that `fx-dsp-map` already verified, so the "word source not found" of the first note is closed. But the routine is the resampling filter bank around a modulated delay, not the effect. To Rotary is the Leslie chain (nonlinear resonant filter `0x125371`, table LFO `0x1235AA`, cabinet `0x1237F8`, the `0x125103` modulator), the same family D94 / D135 / D171 / D172 / D190 could not run: the element words, state block and ring feed are unknown, a hand-made install gives junk (D172) and the live ARM never installs it (D190). A bank port would add a filter with no stage to feed it and no clip to check it against (no To Rotary recall hardware clip exists). **No port.** The single remaining unlock is the D190 / OQ42 install (what the ARM writes into the chain region on a type 7 recall with the read-back served); when that lands, `0x125142` / `0x125103` port as part of the rotary chain, bank first (oracle-testable now: ROM words, `0x120FDB` style golden).

## Implications

- Amp LP24 / HP24 are a state variable pair, not a biquad: the engine's biquad stand-ins differ in damping, clipping and the smoothing law. The port is available as `--firmware-fx-ports lp24`.
- To Rotary stays a pass-through in the engine: its coefficient words are ROM (found), but the effect is the Leslie chain whose element words and state are unobtainable (D190, section 8).

## Follow-ups

1. A hardware clip of an LP24 / HP24 program to confirm the cutoff unit (rate scale 1.106 at 48 kHz) and decide the default.
2. To Rotary: only the Leslie install of D190 / OQ42 (element words) can unblock it; `0x125142` alone is not an effect (section 8).
3. Clav EQ: done (D206, [piano-timbre-gain](piano-timbre-gain.md)); the Clav-class rocker modes stay open.

## Reproduce

```bash
python tools/ns4_dsp_ldr.py target/DSP/dsp_image_0_FX.bin --sram C:/tmp/h/img_0_FX   # flat images
python tools/fx_model_lp24.py check --image C:/tmp/h/img_0_FX/core2_sram.bin --binary target/release/sharc-fx.exe
python tools/fx_model_lp24.py record --image C:/tmp/h/img_0_FX/core2_sram.bin --binary target/release/sharc-fx.exe
python tools/fx_lp24_golden.py tables > crates/nord-core/src/fx/fw_lp24_tables.rs
python tools/fx_lp24_golden.py golden > crates/nord-core/src/fx/tests_fw_lp24_golden.rs
cargo test -p nord-core --lib lp24
python -m pytest tools/tests/test_fx_model_lp24.py tools/tests/test_fx_lp24_capture.py
python tools/fx_lp24_capture.py template --image C:/tmp/h/img_0_FX/core1_sram.bin
python tools/fx_lp24_capture.py golden --image C:/tmp/h/img_0_FX/core2_sram.bin
cargo run --release -p nord-tools --bin program_rank -- <backup.ns4b> --amp-filters --flip fx_amp_lp24 > rank_amp.jsonl
python tools/program_rank_amp.py rank_amp.jsonl --csv docs/research/data/program-ranking-amp-lp24.csv
```
