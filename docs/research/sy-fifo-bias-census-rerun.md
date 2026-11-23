# SY pitch entry `e`, eighth hunt: the SY census rerun with the FIFO index bias (negative, 2026-11-22)

- **Checked:** 2026-11-22
- **Sources (local, never committed):** OS v1.66 `target/fw_unpacked/bank0_CODE.bin` / `bank0_SRAM.bin`, `target/DSP/dsp_image_3_SY1SY2.bin` (`target/oracle/sy.bin`), `target/oled/eeprom.bin`, the factory backup `.reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b`, program `Program/Bank A/Acoustic Guitar.ns4p` (SAMPLES layer flagged, stand-in sample bound by `--ns4b`); new `tools/sy_script_window_census.py` (tests `tools/tests/test_sy_script_window_census.py`). Follows [sy-pitch-writer-static-census](sy-pitch-writer-static-census.md) s6 / s7 lead 2 (D219), [smp-interrupt-rate](smp-interrupt-rate.md) s4 (D211), [smp-page-refill](smp-page-refill.md) s4 (D199).
- **Question:** The SY chip model replayed stale FIFO pairs until D211 added the opt-in index bias (`SY:0=sy=IMAGE=epchain+paced`). Did the replay hide an ARM message that writes the pitch window `DM(0x93500..0x935A0)`, or does a faithful consumer write it?

Tags follow `.agents/rules/provenance-in-code.md`. Addresses and counts only.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Does the bias change what the ARM sends? | **No.** Same program, same 2,700 ARM ticks, two runs (`--chip SY:0=sy=...=epchain` and `=epchain+paced`, the second passes `--fifo-pace 1 --index-bias 0x80000`): `SY0.script` is byte identical, 300 messages, 72 distinct addresses `0x921A6 .. 0xB6510`; 7 more messages fall outside the DSP window and are skipped in both (`skipped.txt` identical) | VERIFIED (run, `python tools/sy_script_window_census.py`) |
| Does any message name the window? | **No**, 0 of 300 in either run (opcode bits ignored) | VERIFIED (same run) |
| Does a faithful consumer write the window? | **No write seen.** All 160 watched words `0x93500..0x9359F` keep their first value over 2,441 passes in both runs (`watch`, one sample per tick batch) | VERIFIED (run, `tools/arm_dsp_live.py --out`) |
| Verdict | **Lead 2 closed: negative.** The FIFO replay did not hide a window writer; the ARM stream does not depend on the model, and the DSP side applies the same 300 words with or without it. `--sample-pitch fw` stays opt-in | closed |

## 2. Limits of this run

| Item | Status |
|---|---|
| The chip stayed idle: `idle_passes` equals `passes` (2,441) and `lane_peak` is 0.0 in both runs. The SAMPLE voice did not start, so a writer that fires only inside a playing voice (a ring refill, an SPI DMA with a real flash) is not exercised. The watch is therefore a statement about the ARM message path only | EMPIRICAL (one program; playing SY runs of D153 / D185 need `--poll-advance` and a fed return path) |
| The watch samples once per tick batch; a word written and restored between samples is invisible. The window is zero filled in the image and a stored entry would persist, so a transient write is unlikely | ASSUMED |
| One program. The 119 programs of D124 gave the same stream without the bias; the ARM stream is independent of the DSP model here (identical scripts), so a rerun per program would not change the message census | ASSUMED |

## Implications

- Of the five leads of the static census, lead 2 is closed. Open: lead 1 (flash directory model, high cost), lead 3 (hardware clip to test the premise that `e` is read non zero), lead 4 (interprocedural comm resolution; its value is now low: the live stream has no window address), lead 5 (SPI DMA opcode 9).
- Next cheapest: lead 3. It needs a recorded hardware clip of one sample note, not more emulation.

## Follow-ups

Run a playing SY voice (`--poll-advance`, return path) with the bias and the window watch; lead 3.

```bash
C=target/fw_unpacked; M=target/oracle
uv run --with unicorn python tools/arm_dsp_live.py $C/bank0_CODE.bin $C/bank0_SRAM.bin --eeprom target/oled/eeprom.bin \
    --program "factory_programs/Program/Bank A/Acoustic Guitar.ns4p" --ns4b "$B" \
    --chip "SY:0=sy=$M/sy.bin=epchain+paced==0x93500,0x93501,...,0x9359F" --live target/release/sharc-live --log runB --out runB.json
python tools/sy_script_window_census.py runA/SY0.script runB/SY0.script --json-a runA.json --json-b runB.json
uv run pytest tools/tests/test_sy_script_window_census.py
```
