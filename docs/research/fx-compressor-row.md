# Compressor gain-law words: the firmware row against the ASSUMED ones

- **Checked:** 2026-11-21
- **Sources (local, never committed):** the FX Core 1 flat SRAM image (`tools/ns4_dsp_ldr.py dsp_image_0_FX.bin --sram`), the ARM boot log `target/bulk/boot_log.jsonl` and script `fx_core1.script`, the SHARC oracle `sharc-fxlive`, `fx_live_native comp`. Tool (new): `tools/fx_compressor_row.py` (`row`, `scan`, `chain`). Builds on [fx-type-map](fx-type-map.md) s7, [fx-contexts](fx-contexts.md) s5.5, [program-ranking-fx](program-ranking-fx.md) s6.2, [fx-vs-oracle](fx-vs-oracle.md) s6.5.
- **Question:** the compressor row of the FX image holds SLOPE 0.75, LOG_GAIN 6, REDUCTION_SCALE 1/96 and TABLE1 2^-16, while `compressor_for_amount` used ASSUMED 0.05, 1.0, 0.5, 0.5. Are the image words what the running instrument uses, and does a port with them match the oracle chain?

## 1. Short answer

| Item | Result | Tag |
|---|---|---|
| The words are the same in every instance | Five compressor rows per FX core (`I1 = 0xB0168, 0xB0180, 0xB0198, 0xB01B0, 0xB01C8`); the gain-law words are identical in all five (SLOPE pair `(0.75, 1.0)`, REDUCTION_SCALE `0x3C2AAAAA`, TABLE1 `0x37800000`, LOG_GAIN `6.0`, SMOOTH `0x3F7DA3C2`) | VERIFIED (`tools/fx_compressor_row.py row`) |
| Nothing rewrites them at runtime | The ARM boot log (boot and every settle phase, both FX cores, all ten instances) writes only offsets `-0x20 .. -0x1B`, `-0x13`, `-0x12`, `-0x8`, `-0x7`: none of the gain-law offsets `-0x17`, `-0x16`, `-0x15`, `-0x14`, `-0x0B`, `-0x09`. The routine model stores back only LEVEL, EXCESS and MAKEUP (`fw_compressor`, word-for-word against the oracle on 2,100 scenarios, including the changed words). `kCmpResponse` writes no word with a value ([fx-reverb-leslie](fx-reverb-leslie.md) s4) | VERIFIED (`tools/fx_compressor_row.py scan`; model check of the earlier work) |
| Port with the row against the oracle chain | The oracle chain with the image row (image words, ARM boot words of the row, the `kCmpAmt` words of the port) against `fx_live_native comp --comp-row firmware`: level delta **0.00 dB**, null depth **-102 to -108 dB**. Against the ASSUMED words: level delta -4.5, -11.5, -26.0 dB and null only -36 to -48 dB (s2) | VERIFIED (`tools/fx_compressor_row.py chain`) |
| Consequence for the default | `FirmwareFx::compressor` is on since D173 with the ASSUMED words: at `kCmpAmt` 127 the port is **26 dB louder** than the firmware on a noise bed that crosses the threshold; D158's "compressor matches the oracle" proved the routine and the `kCmpAmt` words, not the gain law | VERIFIED |
| Port | `fw_params::compressor_for_amount_row` (words `CMP_ROW_*`), opt-in `FirmwareFx::compressor_row` / `--compressor-row` (layer compressor), `MasterBus::use_fw_compressor_amount_row`, `fx_live_native comp --comp-row firmware`, `fx_live_install 3 1`. Default unchanged | tests `fw_params`, `fx::tests_fw_mod1_unit_calls`, `fx_live_native` |

## 2. Oracle chain against the port

`python tools/fx_compressor_row.py chain FX/core1_sram.bin target/bulk/fx_core1.script` (1 s of noise with a 4,000-frame amplitude envelope, 3,000 passes lead, FX core 1 live pass, stage at the image's own row `I1 = 0xB0168`, `0x120630`). Lag is the 4-frame latency of the port stage.

| `kCmpAmt` | Port words | Lag (frames) | Level delta oracle / port (dB) | Null depth after the fit (dB) |
|---|---|---|---|---|
| 32 | ASSUMED | -4 | -4.53 | -36.1 |
| 32 | **firmware row** | -4 | -0.00 | **-108.2** |
| 64 | ASSUMED | -4 | -11.53 | -39.0 |
| 64 | **firmware row** | -4 | -0.00 | **-104.9** |
| 127 | ASSUMED | -4 | -25.97 | -48.2 |
| 127 | **firmware row** | -4 | -0.00 | **-101.9** |

Data: [fx-compressor-row.csv](data/fx-compressor-row.csv).

## 3. What the row is (and is not)

The row words are the DSP's initial data. The oracle run uses them with the ARM's own writes and the `kCmpAmt` words, which is the state the instrument is in after a recall settles (the smoothed makeup and threshold words are set to their targets, as the port does with `settle = true`). Not covered: the glide of a live knob move (the port keeps its smoother), a hardware clip with an audible compressor (none exists), `kCmpResponse` with a value-dependent effect (ignored by the firmware, [fx-reverb-leslie](fx-reverb-leslie.md) s4). The right-channel threshold word `c[-0x18]` is `0.0` in the image and is settled to the `kCmpAmt` threshold by the port; the null of s2 includes that choice.

## Implications

- The ASSUMED gain-law words were wrong by far more than the 1.7 to 10.9 dB that [fx-vs-oracle](fx-vs-oracle.md) s4.4 attributed to a missing makeup gain: the port was louder than the firmware at every amount, by up to 26 dB at 127. The engine compressor was lower than the ASSUMED port, so the engine and the firmware row may be closer than either is to the old port.
- Flipping `compressor_row` is the owner's decision (D187): 384-program before / after in [program-ranking-fx](program-ranking-fx.md) s9.

## Follow-ups

1. A hardware clip with an audible layer compressor (none of the six clips has one) would settle level and attack / release by ear.
2. The master compressor is not wired in the engine (`MasterBus::use_fw_compressor_amount*` only in tests and `fx_stage_response`); wire the row there when it is.
3. Per-unit rows: the synth and organ unit compressor rows were not compared with the piano row (all five rows of core 1 are the same, ASSUMED equal in the PNO image).

## Reproduce

```bash
python tools/ns4_dsp_ldr.py dsp_image_0_FX.bin --sram FX
cargo build --release -p nord-sharc -p nord-tools
python tools/fx_compressor_row.py row   FX/core1_sram.bin
python tools/fx_compressor_row.py scan  target/bulk/boot_log.jsonl
python tools/fx_compressor_row.py chain FX/core1_sram.bin target/bulk/fx_core1.script
```
