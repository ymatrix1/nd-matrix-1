# Research: why 59 of 119 SAMPLE programs stop at the synth voice gate (D143)

- **Checked:** 2026-11-17
- **Sources (local, never committed):** OS v1.66 `target/fw_unpacked/bank0_CODE.bin` and `bank0_SRAM.bin`, the factory restore EEPROM image `target/oled/eeprom.bin`, the 119 SAMPLE programs of `factory_programs/Program/`, one stand-in sample bound by the real loader (as in [arm-dsp-capture](arm-dsp-capture.md)). Disassembly with capstone over the firmware image (addresses and numbers only, no contents). Tool: `tools/arm_voice_gate.py` (`tools/tests/test_arm_voice_gate.py`); data [voice-gate-summary.json](data/voice-gate-summary.json) and [voice-gate-programs.json](data/voice-gate-programs.json) (the raw rows the tool writes, one per program, merged into one JSON list; `arm_voice_gate.py summary` reads it directly; program names, counts, one SPI word each).
- **Question:** What holds the synth voice update `0xC01E00AC` for 59 of the 119 SAMPLE programs of the D124 capture, and is it related to the missing pitch entry `e`?

Tags: **VERIFIED** = reproduced by the command in "Reproduce", **EMPIRICAL** = measured in this harness, **ASSUMED** = reading.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Related to `e` or to the sample DMA? | **No.** Harness artefact: the scheduler `0xC01DEAFC` defers the voice event because of a value read through the SPI1 data register `0x01E12040`, which the harness serves as plain RAM. | VERIFIED (run, section 3) |
| What does the gate wait for? | The slot counter `[0x90639A50 + 4]` must be `<= 1`. It is `\|signed byte\|` of the first half-word of the table `0xC05A4AB0`, refilled on every note-on by `0xC01B1840` (SPI read routine `0xC00119A4`, store `strhi ip, [r5], #4` at `0xC0011AEC`). | VERIFIED (listing and hook run) |
| What does the harness return there? | Leftover test patterns (`0x0, 0x3, 0x1, 0x3F, 0x10001, 0x80000000, 0xF0F0F0F, 0xFFFFFFFF`, an 8 value cycle the firmware writes in the SPI1 page); the value at the note-on depends on how many ticks ran. 79 programs read `3`, 31 read `1`, 9 never read. | EMPIRICAL |
| Cheap fix? | **Yes, in the harness:** zero `r12` at `0xC0011AEC` / `0xC0011930` (SPI read returns 0). `arm_voice_gate.install_spi_neutral`, default on in `arm_dsp_capture.py` (`--no-spi-neutral` restores the old behaviour). | VERIFIED (run) |
| How many pass then? | Key 60 from one saved state, gated against neutral: gated voiced 32, stopped 87; neutral voiced 110, stopped 9. **78 of 87 stopped programs fixed**, 0 voiced programs break. | VERIFIED (batch, 119 programs, 0 failed) |
| The other 9? | They never reach the SPI refill, so a different cause: no voice is queued for key 60 (6 scene II only, 3 scene I only; not diagnosed). | EMPIRICAL |
| Flash / SPI / DMA model needed? | **Not for this gate.** The hardware value is unknown (ASSUMED small, neutral 0 is enough). The gate is not the sample data path; `e` is untouched. | ASSUMED |

D124 counted 59 stopped (voiced = any of keys 36, 60, 84); this run counts 87 (key 60 only, another phase of the test pattern). Both are the same artefact (section 4).

## 2. The gate in the code

Key 60 note-on, `String Quintet` (stopped) against `Acoustic Guitar` (voiced), hooks on entry pcs and a pc trace of `0xC01E72C4`:

| Step | Same in both | Differs |
|---|---|---|
| voice event `0xC01EC690 -> 0xC01E72C4`, path to `0xC01E7544`, event post | identical 331 pcs | none |
| drain `0xC01E623C`, event class 3, `0xC01DEAFC(handler, event)` | same arguments (`r2 = 2`) | return value: 0 (Quintet, 4 calls in 40 ticks) against nonzero (Guitar, then `0xC01E00AC` runs) |
| `0xC01DEAFC` after `0xC01B1840` | | `0xC01DED20..28`: `ldr r3, [r6, #4]; cmp r3, #1; ble 0xC01DEC90`; counter 3 in Quintet (`mov r0, #0` at `0xC01DED2C`), 1 in Guitar |

`0xC01DEAFC` also keeps a 64 bit deadline per slot (`[slot + 0x198]`, `now + 0xF0`, 16 per tick); with the counter above 1 the event is dropped in either case. The counter is stored by `0xC01B1840` (`0xC01B19B4..0xC01B1A3C`: `abs(ldrsb [table])` into the slot) from the table `0xC05A4AB0`, which that function fills with three calls of the SPI read `0xC00119A4` (call `0xC01B1CEC`, flash address argument `0x28B07770`, 4 bytes). The routine writes the address halves to `[spi + 0x38]` and reads the answer at `[spi + 0x40]`, `spi = 0x01E12000` (SPI1, [arm_display_probe](../../tools/arm_display_probe.py) lists the window). No store to `0x01E12040` exists (hooked, 0 hits); the auto-mapped page serves what firmware code stored elsewhere in the page last. Table bytes after the run: `03 00 03 00 ...` Quintet, `01 00 01 00 ...` Guitar.

## 3. Evidence

| Check | Result |
|---|---|
| Neutralise the read on `String Quintet` | `0xC01DEAFC` returns nonzero, `0xC01E00AC` runs, 17 SY words per voice |
| Batch, key 60, gated against neutral (`arm_voice_gate.py batch`, 119 programs, 0 failed) | gated voiced 32, stopped 87, neutral voiced 110, fixed 78; `defers(spi word)` agrees with `stopped` in 109 of 119 |
| The 10 disagreements | 9 never read the SPI table (no voice queued), 1 voiced although the word is 3 |
| SPI word histogram | `3`: 79, `1`: 31, none: 9 |
| Split by program | the stop does not follow program fields: fixed 78 over layer masks `S..` 23, `SS.` 23, `SSS` 21, `.S.` 4, `.SS` 4, `S.S` 3; all 78 have scene I samples. Unfixed 9: scene II only 6, scene I only 3 |

Program fields (layer count, scene, mask) do not predict the stop. The one feature that matters is whether a voice is queued for the key at all (scene II only programs: the recall puts scene I on).

## 4. Why it varies between runs

The word read is wherever the firmware's test pattern cycle stands, so a few ticks more or less change it. D124 (three keys, each from the state after 900 ticks) saw 60 voiced, this run (one key, same state) 32. With the neutral read no program that reaches the refill stays gated.

## 5. Implications

- The D124 "59 stop at the voice gate, undiagnosed" is answered: SPI1 stand-in RAM, not `e`, not a DMA, not polyphony, zone or channel.
- With the fix 110 of 119 programs run the voice update with the stand-in sample. The D124 `e` check was not re-run (70 s per program); the SY words per voice are the same, so the 0-hit result is expected to hold, but only the 60 voiced programs of D124 were checked for `e`.
- SPI1 is still not modelled. If hardware returns a non-zero value, a stub returning it is all the gate needs; no flash image, no DMA.
- The writer of `e` stays open.

## 6. Follow-ups

1. Identify the SPI1 device behind `0xC00119A4` (three reads of 4 bytes, address word `0x28B07770`): likely a panel or pedal chip.
2. Diagnose the 9 programs that queue no voice for key 60 (other keys, scene switch).
3. Re-run `arm_dsp_capture.py batch` with the neutral read (default now) to capture the SY stream of the extra 50 programs and redo the `e` check on them.

## Reproduce

```bash
C=target/fw_unpacked
cd tools && uv run --with unicorn python arm_voice_gate.py batch ../$C/bank0_CODE.bin ../$C/bank0_SRAM.bin \
  --eeprom ../target/oled/eeprom.bin --out ../target/arm_voice_gate --workers 8 \
  --ns4b "../.reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b" ../factory_programs/Program
uv run python arm_voice_gate.py summary ../target/arm_voice_gate
uv run pytest tests/test_arm_voice_gate.py
```

`--only <file>` (a plain whitespace-separated list of `Bank_X__Name` stems, as in the `program` column of the CSV) restricts the batch to the 119 SAMPLE programs of [arm-dsp-capture-programs.csv](data/arm-dsp-capture-programs.csv). The pc traces of `0xC01E72C4` and `0xC01DEAFC` were scratch scripts, not committed.
