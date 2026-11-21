# Mod 1 calls per DSP pass: the dispatcher calls a stage once per pass, the ARM never installs the Mod 1 stages in any harness run (OQ38, D194)

- **Checked:** 2026-11-21
- **Sources (local, never committed):** FX image `target/DSP/dsp_image_0_FX.bin` (`python tools/ns4_dsp_ldr.py ... --sram FX`, `core1_sram.bin`), boot script `target/bulk/fx_core1.script` (96,111 ARM messages for all chips, [fx-mod1-call-count](fx-mod1-call-count.md) s3), the ARM harness capture of one program (`tools/arm_dsp_capture.py one`, `70s Phase Mk I`, 15,364 messages: 1,846 to FX core 1, 1,905 to FX core 2), the SHARC oracle `sharc-fxlive` (`--calls-at`). New tool: `tools/fx_mod1_pass_calls.py` (`stage-writes`, `dispatch`, `install`; tests `tools/tests/test_fx_mod1_pass_calls.py`). Builds on [fx-mod1-call-count](fx-mod1-call-count.md) s5b, [fx-chain-install](fx-chain-install.md), [fx-oracle-live](fx-oracle-live.md).
- **Question:** how many times does the firmware call each Mod 1 unit per DSP pass (OQ38, the fact that blocks the `mod1_unit_calls` default flip of D186)?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Item | Result | Tag |
|---|---|---|
| Calls per pass of a stage the dispatcher reaches | **Exactly 1.** With the full ARM state of a real program (boot script plus the program load) one FX core 1 pass reaches the null stub **35 times**, once per stage record. A Mod 1 routine installed in stage 0 is reached **1 time per pass** at block count 2 and at block count 1, for all six routines (A-PAN, TREM, RM, A-WAH, WAH, PUMP) | VERIFIED (`dispatch`, `install`) |
| Frames a call changes in the 4-frame pass | Block count 2: **4 of 4** frame positions; block count 1: **2 of 4** (positions 2 and 3 stay at the pass-through), all six routines | VERIFIED (`install`; same as [fx-mod1-call-count](fx-mod1-call-count.md) s2) |
| Does the ARM install the Mod 1 stages? | **No, in none of the captures.** 0 of 3,499 boot-script lines and 0 of 1,846 program-load messages to FX core 1 touch the stage window (DM `0xB62C0 .. 0xB6520`: head record, 10 group headers, 35 rows). After boot plus program load every one of the 35 stages still holds the null stub, and no Mod 1 routine pc (core 1 `0x123707`, `0x1235AD`, `0x124344`, `0x12480F`, `0x124746`, `0x1236A0`) is reached in the pass, although the loaded program has A-PAN in its piano and synth rows | VERIFIED (`stage-writes`, `dispatch`) |
| What the ARM does write at program load | Per-unit type words (the Fx1 rows of [fx-type-words.csv](data/fx-type-words.csv)) and the Amp unit routine slots (`DM(0xC1A01 .. 0xC1AC3)`, step 8, three routine pointers per unit, writers `0xC021D02C / 0xC021CED0 / 0xC021CD74`). These slots are not reached in the pass either (`0x124D68`, `0x12500A`, `0x124E0B`: 0 calls) | VERIFIED (capture, `dispatch`) |
| Answer to OQ38 (b) | **Not measurable in this harness.** The real call count of a piano or synth Mod 1 unit per pass needs the install the ARM does later or elsewhere than the boot and program-load phases; it is not in the message stream, so neither the oracle nor the ARM harness can count it. What is settled: the dispatcher gives **1 call per stage record per 4-frame pass** | VERIFIED (dispatcher), open (install) |

## 2. What follows for `mod1_unit_calls`

| Hypothesis | Needs | Against the measurement |
|---|---|---|
| H1: one call per stage per pass, on a 2-frame pass (count 1) or a 4-frame pass (count 2) | organ and piano units on a 2-frame pass, synth on a 4-frame pass | Call count 1 per pass fits. The organ rows are in the PNO image (pass assumed 2 frames, [org-system-run](org-system-run.md)). The **piano rows are in the FX image** ([fx-mod1-call-count](fx-mod1-call-count.md) s3, `0xC1582` ...) whose live pass is **4 frames**: a piano unit with count 1 called once leaves positions 2 and 3 unprocessed (s1), so a piano unit cannot be called once per 4-frame FX pass |
| H2: the piano unit is called twice per 4-frame pass (two stage records, second input pointer 2 frames on) | two stage records for one unit | Not excluded; no install word is seen. This is what `mod1_unit_frames` = 2 on chains 0 to 3 builds (all frames processed in 2-frame calls) |
| H3: the FX pass is 2 frames in the running instrument | pass size of the real chip | Not excluded: `FRAMES_PER_PASS` = 4 of `fx_run.rs` is the harness choice ([fx-oracle-live](fx-oracle-live.md) s1) |

The flip has to choose between H2 and H3 for the piano rows and to know the synth rows. Neither is decidable from the boot, load, idle and key message stream.

## 3. Result for D186

No evidence for a default flip and none against it: **the flip stays proposed, not made**, and OQ38 (b) stays open with a narrower question: *where and when does the ARM install the 35 stage routine pointers, and what are the input pointers of the piano and synth units?* Next measurements, in order:

1. Find the ARM writer of word 0 of the stage records (`0xC021D928`, `0xC021DA80`, `0xC021DC14` of [fx-chain-install](fx-chain-install.md) s3) at run time: it did not fire in the `load`, `idle` or key phases of the capture. Try an effect edit or the type-switch event of the firmware and log the FX window.
2. If found, rerun `dispatch` with those messages: calls per Mod 1 pc per pass, and `--regs-at` for the unit's input pointer (H2 against H3).
3. Or a hardware clip of RM / WAH on a synth layer next to a piano layer ([fx-mod1-call-count](fx-mod1-call-count.md) s5b item 2).

## Implications

- The "calls per pass" factor of the port is 1 per stage record. The port's 2-frame call on organ and piano chains is the H1 / H2 reading, unverified for piano.
- A 4-frame FX pass and count-1 piano rows disagree unless H2 or H3 holds: a concrete test for the next capture.

## Follow-ups

1. ARM run that triggers the stage install (OQ38 b), then rerun `tools/fx_mod1_pass_calls.py dispatch`.
2. Count the stage records the install gives to one unit (H2) and read their input pointers.
3. The same dispatch check on the PNO image for the organ rows (pass size and calls).

## Reproduce

```bash
python tools/ns4_dsp_ldr.py target/DSP/dsp_image_0_FX.bin --sram FX
cargo build --release -p nord-sharc -p nord-tools
uv run --with unicorn python tools/arm_dsp_capture.py one target/fw_unpacked/bank0_CODE.bin \
    target/fw_unpacked/bank0_SRAM.bin --eeprom target/oled/eeprom.bin --out target/m1cc \
    "factory_programs/Program/Bank A/70s Phase Mk I.ns4p"
python tools/fx_mod1_pass_calls.py stage-writes target/bulk/fx_core1.script \
    target/m1cc/Bank_A__70s_Phase_Mk_I.txt.gz
python tools/fx_mod1_pass_calls.py dispatch FX/core1_sram.bin target/bulk/fx_core1.script \
    --capture target/m1cc/Bank_A__70s_Phase_Mk_I.txt.gz
python tools/fx_mod1_pass_calls.py install FX/core1_sram.bin 4 -16   # types and offsets of fx-mod1-call-count s2
```

Output of this run:

```text
boot script lines 3499, stage-window writes 0
capture messages to FX core 1 1846, stage-window writes 0
null stub 35 | A-PAN 0 | TREM 0 | RM 0 | A-WAH 0 | WAH 0 | PUMP 0   (calls in one pass)
install, all six types: block count 2: 1 call per pass, 4 positions; block count 1: 1 call, 2 positions
```
