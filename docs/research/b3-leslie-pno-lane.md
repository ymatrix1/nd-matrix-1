# Research: B3 presence and Leslie ring DMA on the live PNO core 1 lane (D126)

- **Checked:** 2026-11-17
- **Sources (local, never committed):** PNO core 1 SRAM image `core1_sram.bin`, ARM boot message log `pno_core1.script` (29,760 lines), program load and key scripts `docs/research/data/org-system-{load,key8}-pno1.script`, Part A demo video (`NEW NORD STAGE 4 - PROGRAMAS PRESETS DEMO (PART A)`, segment `organ_b3`). Tools: `sharc-sys` (`--pno --epdma --ring-dma --census --lane 2:0 --wav`), `sharc-ring` (new, `runner/ring_trace.rs`), `tools/dsp_lane_vs_hw.py`. Background: [dsp-peripherals-audio](dsp-peripherals-audio.md), [organ-preamp-search](organ-preamp-search.md) s6 / s10, [fx-leslie](fx-leslie.md) s9–s11, [audio-remaining](audio-remaining.md) row 9, [D120](../DECISIONS.md#d120-dsp-peripherals-for-the-system-run-an-opt-in-range-register-block-a-census-and-a-lane-capture-the-pno-lane-is-live-org-core2-stays-silent).
- **Question:** How are the pass-input / ping-pong region (`DM(0xB0078..)` / `DM(0xB00E4..E5)`) and the Leslie history ring (`DM(0xE31E8..)` / `DM(0xE35E0..)`, `DM(0x310C3000)`) filled and consumed on a real B3 pass of the live PNO core 1 lane, does an opt-in ring/DMA model change the 8-key lane output against the `organ_b3` hardware clip, and what blocks porting to the native engine?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by the commands shown, **EMPIRICAL** = measured on this host against the single clip, **ASSUMED** = reading without hardware register docs. Decision number **D126** is a placeholder (decisions up to D125 are taken on other branches).

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Does the live PNO lane read `DM(0xB0078..)`? | **No.** `sharc-ring --trace 0xB0000:0x100` (passes 40..42, 8 keys down) shows `0x1210F0` / `0x1210F3` writing `DM(0xB00E4..E5)` and reads/writes in `0xB00E1..0xB00FF`, with **0 reads and 0 writes** to `0xB0078..0xB008B` or `0xB00C8..0xB00D3`. | VERIFIED (`sharc-ring`) |
| Does the Leslie processor run on the live PNO lane? | **No.** `sharc-sys --probe` over 76 passes (16 warm-up + 60): B3 processor `0x121E2B`, chain loop `0x1213E9`, and null stub `0x12637B` run in **76 of 76 passes**; Leslie processor `0x1256AD`, cabinet `0x1237F8` / `0x1236F8`, and `0x1250CF` / `0x125371` / `0x1235AA` run in **0 of 76 passes**. Chain heads `DM(0xC2F48, 0xC2F82, 0xC2FBC, 0xC2FD6)` all hold `0xC2E4A` (null stub `0x12637B`). | VERIFIED (`--probe`, `--dump`) |
| What does the pass routine do to the Leslie ring? | Every 8th pass (`DM(0xC00DC)` countdown, 5,506 of 44,048 passes), `0x12112C..0x1211A6` re-bases `c[-10]` / `c[-8]` from `0xE31E8..` / `0xE35E0..` and writes `7`, then `0x107` to `DM(0x310C3000)` (`--census`). On the other 7 passes it advances `c[-10]` and `c[-8]` by `c[-6] = 4` words. | VERIFIED (`sharc-ring`, `--census`) |
| What does `--ring-dma` (`core::periph::ringdma`) do? | On `7` then `0x107` to `DM(0x310C3000)`, copies `count` (`0x40`) words from `src` (`0x96634` / `0x967FE` in `0x0A00_0000` DMA view) into `dst` (`0xE31A6` / `0xE359E` second buffers), filling 60 and 25 nonzero words respectively. | VERIFIED (copy, `--dump`), transfer semantics ASSUMED |
| Does `--ring-dma` change the 8-key PNO lane or `organ_b3` deltas? | **No.** `base.wav` and `ring.wav` (44,048 passes, 88,096 frames) have identical `peak 0.360115`, `rms 0.060318`, pass-buffer hash `38ca895586a8e448`, and `dsp_lane_vs_hw.py` deltas: sub `+27.54 dB`, mid `+1.04 dB`, brilliance `+15.41 dB`, gap score `11.02 dB`. | VERIFIED (exact match), clip deltas EMPIRICAL |
| Should anything be ported to `crates/nord-core`? | **No — stopped at Step 3.** Nothing in the live PNO lane consumes the Leslie second buffer or `0xB0078` until the chain installer writes `0x1256AD` (or the FX chip / real chain head) into the active chain. | VERIFIED (blocker) |

## 2. Access trace on the live PNO lane

Commands (`CARGO_TARGET_DIR=C:/tmp/agy-b3-target`):

```bash
sharc-ring $PNO --script-flat $BL --script-flat $S/org-system-load-pno1.script \
    --script-at 16:$S/org-system-key8-pno1.script --trace 0xB0000:0x100 --from 40 --to 43
sharc-ring $PNO --script-flat $BL --script-flat $S/org-system-load-pno1.script \
    --script-at 16:$S/org-system-key8-pno1.script --trace 0xE2E00:0x800 --from 40 --to 43
```

1. **`0xB0000..0xB00FF`:** Every pass writes `DM(0xB00E4)` (`0x1210F0`) and `DM(0xB00E5)` (`0x1210F3`), reads `0xB00E6..0xB00FF`, and writes `0xB00EC..0xB00F1` and `0xB00FE..0xB00FF`. Zero accesses hit `0xB0078..0xB008B` (the 20-word audio-input range) or `0xB00C8..0xB00D3`.
2. **`0xE2E00..0xE35FF`:** On pass 40 (re-base pass), `0x121140..0x12115F` reads `0xE31F1`, `0xE31ED`, `0xE35E9`, `0xE35E5` and writes `0xE319C`, `0xE319E`, `0xE3594`, `0xE3596` (`c[-10]` and `c[-8]` of both Leslie contexts). On passes 41 and 42, `0x12116F..0x1211A4` reads `c[-28]`, `c[-27]`, `c[-20]`, `c[-19]`, `c[-10]`, `c[-8]`, `c[-6]` and writes the modified `c[-10]` and `c[-8]`.
3. **Probe counts (passes 0..75):** `0x121E2B` (76/76), `0x1213E9` (76/76), `0x12637B` (76/76); `0x1256AD` (0/76), `0x1237F8` (0/76), `0x1236F8` (0/76), `0x1250CF` (0/76), `0x125371` (0/76), `0x1235AA` (0/76).
4. **Chain heads after boot + load + 8-key script (`--dump`):** `DM(0xC2F48)`, `DM(0xC2F82)`, `DM(0xC2FBC)`, `DM(0xC2FD6)` all equal `0xC2E4A` (whose routine pointer is `0x12637B`). `0xB2F29..0xB2F2C` hold `[0x1237F8, 0xE359E, 0x1237F8, 0xE31A6]` from `pno_core1.script`, and `0xB0479..` / `0xB0481..` hold `[0x1250CF, 0x125371, 0x121BBA]` and `[0x1250CF, 0x1235AA, 0x1236F8]` from `org-system-load-pno1.script`, but none of them are reached by the active chain.

## 3. Opt-in `--ring-dma` model and 8-key lane comparison

`crates/nord-sharc/src/core/periph/ringdma.rs` (`RingDma`, opt-in via `sharc-sys --ring-dma`, off by default) watches `DM(0x310C3000)`:

- Writing `0x7` arms the restart; a following `0x107` runs the first 4-word record `[modify, src, dst, count]` at `0xE31E8` (left) and `0xE35E0` (right).
- When `modify == 1`, both `src` and `dst` have tag `0x0A00_0000`, and `1 <= count <= 0x400`, it copies `count` words from `src & 0x00FF_FFFF` (`0x96634`, `0x967FE`) to `dst & 0x00FF_FFFF` (`0xE31A6`, `0xE359E`).
- Over 216 passes (`--dump`), `DM(0xE31A6..)` goes from 0 to 60 nonzero words and `DM(0xE359E..)` from 0 to 25 nonzero words; `0xB0000..0xB2FFF` and `0xC0000..0xC2FFF` hashes are unchanged (`38ca895586a8e448` and `f3e7c8fbcbbbb35f`).

44,048-pass lane runs (16 warm-up + 44,032 measured, 88,096 frames) and `tools/dsp_lane_vs_hw.py --segment organ_b3 --slice-s 1.8`:

| Run | Peak | RMS | hw RMS dBFS | emu RMS dBFS | RMS delta | Centroid hw / emu (Hz) | sub (20-120) | low (120-400) | mid (400-1.5k) | presence (1.5k-3k) | brilliance (3k-8k) | air (8k-20k) | Gap score |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 0.360115 | 0.060318 | -24.35 | -24.39 | -0.04 | 338.2 / 361.1 | +27.54 | -3.69 | +1.04 | +7.42 | +15.41 | -18.53 | 11.02 |
| `--ring-dma` | 0.360115 | 0.060318 | -24.35 | -24.39 | -0.04 | 338.2 / 361.1 | +27.54 | -3.69 | +1.04 | +7.42 | +15.41 | -18.53 | 11.02 |

Census of `--ring-dma` (44,048 passes): `0x310C3000` is touched in 5,506 passes (every 8th pass, 2 writes per pass); the other 4 per-pass registers (`0x28AC0388`, `0x31022330`, `0x3108900C`, `0x3108945C`) are touched in all 44,048 passes.

## 4. Implications (what blocks)

1. **The blocker is not the ring DMA model.** `RingDma` fills `c[-8]` (`0xE31A6` / `0xE359E`) on every 8th pass as the descriptor chains specify, yet the PNO lane output is bit-identical because the Leslie processor `0x1256AD` and its cabinet routines `0x1237F8` / `0x1236F8` never execute in the live PNO lane (`0xC2F48`..`0xC2FD6` stay at the null stub `0xC2E4A`).
2. **Where the Leslie runs or how the chain head is installed remains open.** Either the ARM `organ_b3` capture (`org-system-load-pno1.script`) lacks the message that links `0x1256AD` into `0xC2F48..` (or another chain), or the Leslie runs on another core (FX) for this program.
3. **No native port.** Per Step 3, nothing is ported to `crates/nord-core/src/organ/` until a live lane exercises `0x1256AD` / `0x1237F8` and shows a verified band improvement.

## 5. Follow-ups

1. Find which ARM message or recall phase links `0x1256AD` (row 6 of `0xC05450C8`) or `0xB0476` / `0xB047E` into an active chain head on PNO core 1, or check the FX cores (`fx_core1.script` / `fx_core2.script`) for the active Leslie path of `B3 Chords n Solo.ns4p` (checked in [b3-leslie-fx-chip](b3-leslie-fx-chip.md), D135: no Leslie processor in the static FX images and no Leslie row in the boot logs; run-time loaded code and an amp type 7 recall untested).
2. Verify the SHARC TCB layout of `0xE31E8..` / `0xE35E0..` (including the second record `2, 0xA0E31A6, 0xA0E2E08, 0x32A`) against CCES documentation / simulator once `0x1256AD` is active on a lane.
