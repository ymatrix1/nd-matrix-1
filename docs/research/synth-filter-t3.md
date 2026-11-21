# Raw synth filter type 3 (stage `0x1257B2`): bit-exact model and opt-in port

- **Checked:** 2026-11-20
- **Sources (local, never committed):** ORG core 2 image (`python tools/ns4_dsp_ldr.py target/DSP/dsp_image_2_ORG.bin --sram DIR`), ADI disassembly of bytes `0x24AF64..0x24B226` (`tools/adi_dis.py`), `sharc-org` (`tools/synth_stage_oracle.py` driver, pass control stores patched out), [synth-filter-links](synth-filter-links.md) (type to stage link, 61 of 988 factory filter layers use raw type 3), [synth-adr-and-strip-law](synth-adr-and-strip-law.md) s5 (impulse fit), [synth-stage-equations](synth-stage-equations.md). Derived numbers only: [data/synth-filter-t3-tables.csv](data/synth-filter-t3-tables.csv), [data/synth-filter-t3-oracle-cases.json](data/synth-filter-t3-oracle-cases.json).
- **Question:** The engine plays raw `kFltType` 3 (the panel's "LP+HP") as `Lp12` (`filter_model_of_type`). What does the firmware stage `0x1257B2` compute, can the Rust engine reproduce it word for word, and how far is the stand-in from it?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Item | Result | Tag |
|---|---|---|
| Equations | the stage is a **coupled second order form with five state words per lane**; the prologue builds six coefficient words from the cutoff word `R` (frame `-30`) and the position word `W` (frame `-28`) through three 129 word tables, then 4 samples per call run the recursion of section 2 | VERIFIED |
| Oracle match | model against `sharc-org`: **1840 of 1840 words equal** (outputs, final states and coefficient words, both lanes, 40 random trials of 3 calls, random `R`, position, states and inputs), `python tools/synth_filter_t3.py --image SRAM_ORG/core2_sram.bin` | VERIFIED |
| Lanes | the two SIMD lanes (words `k`, `k + 1`) are two independent channels with identical coefficients; the `rA<->sB` swaps in the prologue only move table entries between lanes. The prologue reads `R` and `W` of lane x; poking different words in lane y changes nothing the model needs | VERIFIED (equal inputs), ASSUMED for unequal `R` per lane (never occurs in the pass) |
| Shape | **not a low pass**: unity gain at DC and Nyquist, a peak (up to +6 dB) at low position index or a notch (down to -15 dB) at high index, around a cutoff set by `R` (section 3). The `Lp12` stand-in is wrong in kind, not only in corner | VERIFIED (model run) |
| Rust port | `synth/fw_stage3.rs` (`Stage3Coefs`, `FwStage3`), tables `fw_stage3_tables.rs`, goldens `tests_fw_stage3_golden.rs` (5 rows x 64 samples, worst deviation under `2e-5`) | VERIFIED (golden test) |
| Engine switch | **opt-in** `--synth-stage3` (`SynthLayer::fw_stage3`), read only on the strip chain for layers decoded with raw type 3 (`SynthLayer::filter_raw3`). Default unchanged (D22) | decision |
| Cutoff unit | unchanged: the stage takes the strip coefficient `R = tan(pi fc / fs)` like the other stages, and the engine feeds it the same `R` (uniform `2 F` reading not applied here) | ASSUMED |

## 2. Equations (VERIFIED, `tools/synth_filter_t3.py`)

Tables (`DM(0xB0A9F)` `T0`, `DM(0xB0C0D)` `T1`, `DM(0xB0C8E)` `T2`, 129 float words each; `T1[0] = 0`, `T2[0] = 2`, `T0` runs `0.0007 .. 1.99`). `lerp(T, i, f) = T[i] + (T[i+1] - T[i]) f`, every operation rounded to the 40 bit register format, every stored word truncated to 32 bit, as in [sharc40.py](../../tools/sharc40.py).

```text
P  = FIX(R / 2, 31)             i = P >> 24            f  = (P & 0xFFFFFF) / 2^24
U  = lerp(T1, i, f)             V = lerp(T2, i, f)     g  = 2 U            (frame -22; V frame -20)
Wv = lerp(T0, W >> 24, (W & 0xFFFFFF) / 2^24)
Q  = FIX(Wv / 2, 31)            i2 = Q >> 24           f2 = (Q & 0xFFFFFF) / 2^24
X1 = lerp(T1, i2, f2)           X2 = lerp(T2, i2, f2)
kc = CLIP(4.5 g + 0.25, 1)      (frame +10)    2 X1 (frame +12)    X2 (frame +14)
h  = 1 - (X1^2 + X1 X2)         (frame -24)    kh = kc h (register only)
```

Per sample, states `A` (-10), `D` (-8), `B` (-6), `C` (-4), `E` (-2) read back from the frame (32 bit):

```text
p  = B + g A                     B' = p
e  = E + 2 X1 C                  E' = e          D' = x
n  = (x kh - e) - X2 C
m  = ((D + x) / 2 - p) - V A
y  = n + (B + p) / 2             (output)
C' = CLIP(C + 2 X1 n, 1)         A' = CLIP(A + g m, 1)
```

Constants (`DM(0xC28FA + k)`): `4.5` at `-20`, `0.25` at `-32`, `1.0` at `-28` (clip limit and `cin`, frame `-26` is stored as `1.0`). The cutoff index is `R / 2` (as noted in [synth-stage-equations](synth-stage-equations.md) s1); the resonance position goes through `T0` and then through `T1` / `T2` again, so the position changes both `X1`, `X2` and the shape word.

How it was found: the listing is 154 short instructions (a 90 instruction prologue plus a 24 instruction loop body unrolled 4 times). A small SIMD interpreter of the ADI listing (analysis only) reproduced the oracle with one rule, `rA<->sB` exchanges exactly the two named registers (not a lane swap of both pairs), and a load or store in SIMD mode moves word `a` for PEx and `a + 1` for PEy (the tables are read as `T[i]` / `T[i + 1]` pairs). The closed form above was then read off the interpreter trace.

## 3. Frequency response of the model (VERIFIED run, gain in dB, 3000 sample impulse)

| `R` | position | 0.002 | 0.01 | 0.03 | 0.06 | 0.1 | 0.2 | 0.3 | 0.45 cycles/sample |
|---|---|---|---|---|---|---|---|---|---|
| 0.3 | 0 | +5.9 | +5.9 | +4.8 | +2.1 | -1.6 | -1.5 | -0.1 | 0.0 |
| 0.3 | 64 | -0.4 | +0.3 | +2.9 | +0.7 | -2.5 | -1.4 | -0.1 | 0.0 |
| 0.3 | 100 | 0.0 | -0.2 | -2.2 | -8.6 | -13.3 | -2.1 | -0.3 | 0.0 |
| 0.3 | 126 | 0.0 | -0.1 | -1.0 | -3.3 | -6.3 | -5.8 | -1.2 | 0.0 |
| 0.8 | 100 | 0.0 | -0.1 | -1.2 | -3.6 | -8.1 | -9.1 | -1.2 | +0.1 |

The shape is a bump or a notch of a few dB to 15 dB with unit gain at both ends: an equalizer like block (an "LP+HP" read as a band reject at high position). It is not close to `Lp12` (a 12 dB per octave roll off), so layers with raw type 3 play wrong today by `10 .. 40 dB` above the cutoff. 61 of 988 factory filter layers use it (D121).

## 4. Port and tests

| Piece | File | Check |
|---|---|---|
| Model | `tools/synth_filter_t3.py` | oracle 1840 / 1840; recorded cases (6 seeds) replay word for word without firmware, `tools/tests/test_synth_filter_t3.py` |
| Tables | `fw_stage3_tables.rs` (generated by `tools/synth_filter_t3_golden.py tables`) | shape test (`T1[0] = 0`, `T2[0] = 2`) |
| Stage | `fw_stage3.rs` | golden words (coefficients within `2e-6` relative, outputs under `2e-5`), bounded states (50,000 samples at the top position), non finite input and a non finite state reset, cutoff zero equals the table start |
| Switch | `fw_filter.rs` `FwFilterType::Stage3`, `render.rs`, `render_opts.rs` (`--synth-stage3`), `synth_params.rs` (`filter_raw3` from the raw `kFltType`) | `raw_type3_runs_the_stage_only_with_the_strip_chain_and_the_opt_in`, `stage3_and_unison_gain_are_opt_in_and_reach_the_layers` |

Real time: the Rust stage keeps five `f32` states, no allocation, no lock; the prologue (two `T1` / `T2` interpolations, one `T0`) goes through the per-voice `FwCoefMemo` like the other kinds, so it is recomputed only when cutoff, resonance or sample rate change.

## Implications

- The stand-in for raw type 3 can be replaced by the real stage word for word; the remaining error of a type 3 layer is then the cutoff unit and the oscillator, not the filter shape.
- The `--synth-stage3` switch stays opt-in: none of the 12 Part A synth clips was verified to carry a raw type 3 layer, so there is no clip evidence for the flip (D22). The clip table is in [synth-unison-level](synth-unison-level.md) s4.

## Follow-ups

1. A hardware clip of a raw type 3 program (61 factory layers) with a long held note to confirm the bump or notch and the cutoff unit.
2. The type 3 position law `W` assumes the engine's resonance map; it follows `TablePos::from_resonance` (settled `128 knob / 127` behind `--synth-fw-resonance`, D121).

## Reproduce

```bash
export NS4_CCES="C:/Analog Devices/CrossCore Embedded Studio 2.12.1"
python tools/ns4_dsp_ldr.py target/DSP/dsp_image_2_ORG.bin --sram SRAM_ORG
python tools/adi_dis.py SRAM_ORG/core2_sram.bin 0x24AF64 0x24B228          # listing
python tools/synth_filter_t3.py --image SRAM_ORG/core2_sram.bin --sharc-org target/release/sharc-org.exe
python tools/synth_filter_t3.py --image SRAM_ORG/core2_sram.bin --emit-tables docs/research/data/synth-filter-t3-tables.csv
python tools/synth_filter_t3.py --image SRAM_ORG/core2_sram.bin --record docs/research/data/synth-filter-t3-oracle-cases.json
python tools/synth_filter_t3_golden.py tables > crates/nord-core/src/synth/fw_stage3_tables.rs
python tools/synth_filter_t3_golden.py golden > crates/nord-core/src/synth/tests_fw_stage3_golden.rs
cargo fmt -p nord-core && cargo test -p nord-core --lib fw_stage3
```
