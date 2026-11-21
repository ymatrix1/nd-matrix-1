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
| To Rotary | chain row routine `0x125142` (partner `0x125103`; core 2 copies `0x1247CE` / `0x12478F`) plus a null stub; coefficients read through `PM 0x9437A`; **not modelled**, source of its words not found | EMPIRICAL (identity), open (words) |
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
| Piano Clav EQ (`kEq` 6, 7) | **Negative result.** `Edt_Pno_kEq` (`0x15C`) is stored by the ARM at the layer cell with one store (pc `0xC00CE5A8`) and sends only the 1.0 reload trigger to the SMP path ([piano-dsp-path](piano-dsp-path.md) s5); no EQ coefficient word follows from the edit. The firmware table `0x2F730C` is not the EQ ([fx-eq-words](fx-eq-words.md) s5). Search narrowed to: the consumer of the stored cell (a read at an address the capture did not reach) and the `0x1be108` index math of bean `ns4-98f3`. No routine or coefficient words found, so no port. |

## 5. Rust port (opt-in)

`fx/fw_lp24.rs` runs the routine on four frames per call (smoothing once per call, then the 4 frames, two channels), `fw_lp24_tables.rs` holds the captured words. `LayerFxChain::process_fw_lp24` (`fx/amp_eq.rs`) replaces the two engine biquad pairs for `AmpCabinet::LpFilter` / `HpFilter` when `FirmwareFx::lp24` is on (`--firmware-fx-ports lp24`; not in `ALL`, `PARITY` or `PORT_NAMES`). The cutoff index is the nearest `EQ_FREQ_HZ` step of the engine's Mid Freq, the Mid counts are `res / 10 * 120 - 60`. Real time: no allocation, no lock. Checks: oracle golden (both channels), shape tests (LP / HP direction, bounded states at full resonance, the glide of the smoother), chain tests (opt-in, latency, other amps untouched). No hardware clip of an LP24 / HP24 program exists yet, so the flag stays opt-in (D22 style).

## Implications

- Amp LP24 / HP24 are a state variable pair, not a biquad: the engine's biquad stand-ins differ in damping, clipping and the smoothing law. The port is available as `--firmware-fx-ports lp24`.
- To Rotary stays a pass-through in the engine until `0x125142`'s words are found.

## Follow-ups

1. A hardware clip of an LP24 / HP24 program to confirm the cutoff unit (rate scale 1.106 at 48 kHz) and decide the default.
2. Model `0x125142` (To Rotary) and find its word source.
3. Clav EQ: find the reader of the `kEq` cell.

## Reproduce

```bash
python tools/ns4_dsp_ldr.py target/DSP/dsp_image_0_FX.bin --sram C:/tmp/h/img_0_FX   # flat images
python tools/fx_model_lp24.py check --image C:/tmp/h/img_0_FX/core2_sram.bin --binary target/release/sharc-fx.exe
python tools/fx_model_lp24.py record --image C:/tmp/h/img_0_FX/core2_sram.bin --binary target/release/sharc-fx.exe
python tools/fx_lp24_golden.py tables > crates/nord-core/src/fx/fw_lp24_tables.rs
python tools/fx_lp24_golden.py golden > crates/nord-core/src/fx/tests_fw_lp24_golden.rs
cargo test -p nord-core --lib lp24
python -m pytest tools/tests/test_fx_model_lp24.py
```
