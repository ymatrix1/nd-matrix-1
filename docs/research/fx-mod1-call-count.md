# Mod 1 routines: frames per call in the running instrument (OQ38)

- **Checked:** 2026-11-21
- **Sources (local, never committed):** the FX Core 1 flat SRAM image (`python tools/ns4_dsp_ldr.py dsp_image_0_FX.bin --sram`), the ARM boot message scripts `target/bulk/fx_core1.script` and `target/bulk/pno_core1.script` (`tools/arm_bulk_tables.py capture`, the 96,111 messages the firmware ARM sends to the DSPs), the SHARC oracle `sharc-fxlive`, [fx-type-words.csv](data/fx-type-words.csv). Tools (new): `tools/fx_mod1_call_count.py` (`counts`, `probe`). Builds on [fx-vs-oracle](fx-vs-oracle.md) s6.2, [fx-mod-types](fx-mod-types.md) s7, [fx-routine-models](fx-routine-models.md).
- **Question:** how many frames per call does the running instrument give the Mod 1 routines (A-PAN, TREM, RM, A-WAH, WAH, PUMP): 2 or 4? `LayerFxChain::mod1_rest_frames` was ASSUMED 2; the oracle pass is 4 frames and the port nulls it at -100 dB only with 4.

## 1. Short answer

| Item | Result | Tag |
|---|---|---|
| What sets the call length | Every Mod 1 routine loops over a **block count word** in its context (`LCNTR`), one block = 2 stereo frames. In the oracle with the count at 1, a 4-frame pass changes only frame positions 0 and 1; positions 2 and 3 stay at the pass-through (deviation 0.0000, all six routines). With the count at 2 all four positions are processed. A call processes exactly `2 x count` frames (s2) | VERIFIED (`tools/fx_mod1_call_count.py probe`) |
| What the ARM writes | The ARM boot messages set the count to **1 for every organ and piano unit row and 2 for every synth unit row**, for A-PAN, TREM, RM, A-WAH, WAH and PUMP (s3) | VERIFIED (`tools/fx_mod1_call_count.py counts`) |
| Answer | **2 frames per call on the organ and piano layers, 4 on the synth layers.** Neither 2 nor 4 for all. The default 2 of `mod1_rest_frames` is right for organ and piano and wrong for synth; the live harness 4 is right for synth only. A shipped instrument cannot leave frames of its own pass unprocessed, so the count word is the call length | VERIFIED (words and loop); the pass structure per unit is ASSUMED (s5) |
| LFO frequency | Not affected by the call length for RM: the phase advance per call is `inc x 2^count` (count 1: 2 inc, count 2: 4 inc, read from the oracle, s4), i.e. `inc` per frame either way. The factor 2 worry of [fx-mod-types](fx-mod-types.md) s7 (OQ21) is not a call-length effect for RM | VERIFIED (RM only) |
| What changes with the call length | Latency (2 or 4 frames) and the per-call smoothers (`K = 0.99078` per call, time constant 108 calls: 2.2 ms against 4.5 ms at 48 kHz), the A-WAH envelope step, the RM / PUMP ramp updates. That is the -16 to -26 dB null of the 2-frame port against the 4-frame oracle | VERIFIED (null table of [fx-vs-oracle](fx-vs-oracle.md) s3.2) |
| Port | `LayerFxChain::mod1_unit_frames` (2 on chains 0 to 3, 4 on the synth chains 4 to 6) and the opt-in `FirmwareFx::mod1_unit_calls` / `--mod1-unit-calls` pick it for RM, A-WAH, WAH, PUMP. **A-PAN and TREM stay at 4 frames** (`FwMod1Stage` is hard 4, follow-up 1). Default unchanged | code, tests `fx::tests_fw_mod1_unit_calls` |

## 2. The loop count in the oracle

`python tools/fx_mod1_call_count.py probe FX/core1_sram.bin TYPE OFFSET` installs the routine through the live chain (the install of `fx_live_install`, 4-frame pass, white noise), once with the count word at 2 and once at 1, and prints the RMS deviation of the output from `0.1866 x input` for each frame position of the pass:

| kFx1Type | Routine | Count word | Deviation at position 0..3, count 2 | Deviation at position 0..3, count 1 |
|---|---|---|---|---|
| 0 | A-PAN | `c[-3]` | 0.2974 0.2909 0.2909 0.2967 | 0.2974 0.2909 **0.0000 0.0000** |
| 2 | TREM | `c[4]` | 0.3154 0.3203 0.3218 0.3152 | 0.3154 0.3203 **0.0000 0.0000** |
| 4 | RM | `c[-16]` | 0.6326 0.6381 0.6332 0.6307 | 0.6326 0.6369 **0.0000 0.0000** |
| 6 | A-WAH | `c[4]` | 0.8838 0.8969 0.8915 0.8900 | 0.8657 0.9031 **0.0000 0.0000** |
| 8 | WAH | `c[7]` | 0.5358 0.5475 0.5418 0.5348 | 0.5475 0.5431 **0.0000 0.0000** |
| 10 | PUMP | `c[-8]` | 0.3418 0.3373 0.3358 0.3418 | 0.3418 0.3373 **0.0000 0.0000** |

(Types 1, 3, 5, 7, 9, 11 share the routine of the type before.) The existing harness sets the count to 2 for every install (`fw_live::from_state`), which is why the earlier oracle comparison could only ever test the 4-frame call.

## 3. What the ARM writes

`python tools/fx_mod1_call_count.py counts target/bulk` (rows from [fx-type-words.csv](data/fx-type-words.csv), the word at `I1 + offset` of s2 in the last-value-per-word boot script; the organ rows live in the PNO image and its script):

| Routine | Organ unit row: count | Piano unit row: count | Synth unit row: count |
|---|---|---|---|
| A-PAN | `0xE105C`: 1 | `0xC1582`: 1 | `0xC1528`: 2 |
| TREM | `0xE1098`: 1 | `0xC1630`: 1 | `0xC15BE`: 2 |
| RM | `0xE0A64`: 1 | `0xB04B4`: 1 | `0xB047E`: 2 |
| A-WAH | `0xE11D4`: 1 | `0xC1946`: 1 | `0xC18D4`: 2 |
| WAH | `0xE1184`: 1 | `0xC1884`: 1 | `0xC180C`: 2 |
| PUMP | `0xE10E4`: 1 | `0xC16CA`: 1 | `0xC167C`: 2 |

The other row of a module (`None` or 0 in the tool output) belongs to another routine, it is not a count.

## 4. Phase advance per call (RM)

`sharc-fxlive` with the RM stage, `--dump` of the phase word after 3200, 3201 and 3202 passes, increment word `0x7A4100`: count 1 advances `0x00F48200` per call (2 x inc), count 2 advances `0x01E90400` (4 x inc). The RM routine shifts the increment by the count. WAH and PUMP use other phase words (the dump of this run read a word that does not move for them); their advance per call was not measured.

## 5. What is not settled

- **Where each unit runs, and the pass size there.** The organ rows are written to the PNO image (2-frame pass, count 1 = one call per pass) and the piano and synth rows to the FX image (4-frame pass in the harness). A piano row with count 1 in a 4-frame FX pass would leave two frames unprocessed, so either the FX chip calls it twice per pass or the piano FX sit elsewhere. The count word is what the DSP loops on, so the call length stands; how many calls make a pass is not traced (the live link `sharc-live` has no FX preset, and the ARM install of the 35 chain records is not in the boot log: `sharc-fxlive` with the full boot script gives silence).
- **Rate tables per unit.** The synth rows hold half the piano phase increments for RM (`0x1C200 .. 0x7DB1D00` against `0x38400 .. 0xFB63B00`). With `inc` per frame independent of the call length (s4) a synth layer would run at half the piano Hz. The port uses the piano words for every layer (ASSUMED, [fx-mod-types](fx-mod-types.md) s7.1); this was not changed and `mod1_unit_calls` keeps the piano words.
- **A-PAN / TREM at 2 frames, PUMP and WAH phase per call:** see the follow-ups.

## 5b. Why `mod1_unit_calls` is not the default yet (the missing measurement)

The default flip of D186 needs one fact this note does not have: **how many Mod 1 calls make one pass on each unit** (organ, piano, synth). The words prove the length of one call (2 or 4 frames); they do not prove that the chip's pass is built from those calls the way the port builds it (one call per layer per host frame batch). Tried without success: `sharc-live` has no FX preset; the ARM install of the 35 chain records is not in the boot log; `sharc-fxlive` with the full boot script gives silence.

Exact measurement that settles it (any one of):

1. Run the FX chip (`sharc-fxlive`, FX core 1 and the PNO core) with the ARM's chain-install records for one program (the 35 records the firmware writes when a program is recalled), and count the calls of each Mod 1 pc (`0x122D93`, `0x122C39`, `0x1239D0`, `0x123E9B`, `0x123DD2`, `0x122D2C`) per output block, per unit instance (`I1` of the unit rows in s3). Needed: calls per block and the block size of the PNO lane and of the FX lane.
2. Or a hardware clip of RM / WAH on a synth layer next to the same on a piano layer, to read the latency and smoothing time constant (2.2 ms against 4.5 ms per call, s1).

**Update 2026-11-21 (D194, [fx-mod1-pass-calls](fx-mod1-pass-calls.md)):** the dispatcher gives 1 call per stage per pass (VERIFIED), but the ARM never writes the stage records in the boot or program-load message streams, so option 1 could not be run; the count per unit stays open.

Until then the opt-in `--mod1-unit-calls` stays (default off); the 384-program ranking cannot judge it (mean 2.12 to 2.12, 1 program worse by more than 0.5, Time Warp Saw 5.87 to 6.83).

## Implications

- The call-count half of OQ38 is answered per unit kind: 2 (organ, piano) and 4 (synth). The proposed default is D186 (owner decides).
- A hardware clip of RM or WAH on a synth layer would be needed to separate the latency / smoothing effect from the rate-table question.

## Follow-ups

1. A 2-frame option for `FwMod1Stage` (A-PAN, TREM): the routine ports take whole blocks of any count, only the adapter is hard 4.
2. Measure the phase advance per call for WAH and PUMP; decide the synth unit rate words (use the synth rows with the synth call, or keep the piano words).
3. Trace the number of calls per pass per unit once an FX preset exists in the live link.

## Reproduce

```bash
python tools/ns4_dsp_ldr.py dsp_image_0_FX.bin --sram FX
cargo build --release -p nord-sharc -p nord-tools
python tools/fx_mod1_call_count.py counts target/bulk
python tools/fx_mod1_call_count.py probe FX/core1_sram.bin 4 -16   # types and offsets of s2
```
