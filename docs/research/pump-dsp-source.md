# PUMP clock word: where `DM(0xC0230)` comes from (OQ36, D152)

- **Checked date:** 2026-11-17
- **Sources (local, never committed):** FX DSP image `target/DSP/dsp_image_0_FX.bin` (flat SRAM and L2 by `tools/ns4_dsp_ldr.py --sram`, VISA listing by `tools/adi_dis.py`, byte = 2 pc), SMP image `dsp_image_4_SMP.bin` (`tools/ns4_dsp_raw.py --sram`, listing `0x1C0300..0x1C1590`), PNO and ORG L2 descriptor words, an all-chip watch of the ARM harness (same method as `tools/arm_tick_probe.py fxwatch`, every chip instead of FX only) on A:25 `Sidechain Whl II` for 30,000 timer ticks after an 800 tick settle, `sharc-live`. Builds on [fx-mod-types](fx-mod-types.md) s7.2, [piano-receiver-scale](piano-receiver-scale.md), [dsp-rx-protocol](dsp-rx-protocol.md), [arm-tick-timers](arm-tick-timers.md). Numbers and addresses only.
- **Question:** who writes `DM(0xC0230)` (the word FX core 1 copies to `DM(0xC0C70)`, read by the PUMP routine `0x122D2C` through `c[-29]`), from which tempo state and at what rate; can the span and zero point of the duck be derived from the firmware instead of from the 2.10 Hz target?

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Where is the word? | `DM(0xC0453)` = strip base + `0x74` (`0xC01BC + 0x74 = 0xC0230`, or `0xB003C + 0x74` in the other ping-pong half). FX core 1 copies words 0 and 1 at that address to `DM(0xC0C70)` / `DM(0xC0C72)` every pass (listing `0x1213FB..0x121405`, setter `0x1212F2..0x12130A`). | VERIFIED (listing) |
| Which DMA owns it? | **Not the SMP 10-word frame.** SMP frames land at `0xC0234` (A) and `0xC023E` (B) (descriptors `0x283008D0`, `0x283008F8`); `0xC0230` is 4 words **before** frame A. The strip `0xC01BC..` is filled by three 2D receive descriptors (`0x283006F0`, `0x28300790`, `0x28300830`, CFG `0x4064223`, X 2, Y 40, bases 40 words apart); `0xC0230` lies in the third (offset 36 of the block at `0xC020C`) if the block is read contiguously, which is **not decoded** (the XMOD field reads `0x800000`, YMOD `0xFF800004`). No descriptor targets `0x283008C0`. This corrects s7.2 / D149 ("head of the SMP serial frame buffer"). | VERIFIED (descriptor words); block geometry ASSUMED |
| Does the SMP image write it? | **Not found in the paths run.** The SMP ISR post stage stores ring words at `i2 + 0, 2, 0xA, 0xC` (L/R pairs of two frames); the listing has no other store to `0xB2000..0xB2027` and none to ring words 4 to 9. | EMPIRICAL (listing grep, negative) |
| Does the ARM send tempo or clock words to any DSP? | **Not found in the paths run.** A:25 (Master Clock on, 126 BPM, PUMP rate 73), 30,000 ticks (1.25 s, 2.6 beats): the only DSP messages are 119 writes to three PNO core 1 words (`0xB2F25`, `0xB2F26`, `0xB2FA1`; first at tick 52; one per about 250 ticks = 96 Hz, not the 0.476 s beat). No word to FX, ORG, SY or SMP in the window. | EMPIRICAL (run) |
| Span of the clock word (beats per wrap) | **Not derivable.** No writer found, so no increment, tempo link or reset. 32 beats stays ASSUMED. | ASSUMED |
| Zero point | **Not derivable.** Duck on the beat at word 0 stays ASSUMED. | ASSUMED |
| Phase reset at note-on or program load | Not found: under MST CLK the routine reads an external word and never resets it (own phase and counter do not reach the output). Any reset would sit on the writer's side. | VERIFIED (routine), open (writer) |

## 2. What was traced

1. **FX reader.** `0x1212F2..0x12130A` selects the buffer set from `DM(0xC0450)` against `0x28300830` and stores `DM(0xC0453) = base + 0x74`. `0x1213FB..0x121405`: `i1 = DM(0xC0453)`, `DM(0xC0C70) = DM(i1, 0)`, `DM(0xC0C72) = DM(i1, 1)`. The PUMP routine reads `[c[-29]] = DM(0xC0C70)`.
2. **FX descriptors** (L2 `0x20004000..`, words `{next, addr, cfg, xcnt, xmod, ycnt, ymod}`): receive 2D blocks at `0x283006F0 / 0x28300790 / 0x28300830` and the core 2 twins `0x282C00F0 / 0x282C0190 / 0x282C0230`; SMP frames at `0x283008D0 / 0x283008F8` and `0x28B008D0 / 0x28B008F8`.
3. **Transmit side.** PNO (core 2 sets) and ORG (core 1 and core 2 sets) carry 2D transmit descriptors (CFG `0x4A64225` / `0x4064225`, X 2, XMOD `0xA0`, Y `0x28`, YMOD `-0x9C`: two words 40 apart, 40 rows). Three FX receive blocks against PNO plus two ORG transmit sets is the likely pairing (ASSUMED). Not run: `sharc-live` runs one core of one chip; PNO core 1 shows zero at `0xC0000..0xC004F`, and the ORG preset halts at pass 1 without its own start-up.
4. **SMP.** Table row 3.
5. **ARM.** Table row 4. The three PNO words are not named here.

## 3. Rate at several tempos

The firmware gives the **form** (VERIFIED): ducks per second = `N` times the wrap rate of the clock word, `N = c[-28]` the ARM multiplier (2 to 256 over the 15 rate steps). The wrap rate is **not** derivable. Under the ASSUMED 32 beat span and a tempo driven word, ducks per second = `bpm / 60 * N / 32`:

| BPM | N = 32 (A:25, rate 73) | N = 16 | N = 8 |
|---|---|---|---|
| 60 | 1.000 Hz | 0.500 Hz | 0.250 Hz |
| 90 | 1.500 Hz | 0.750 Hz | 0.375 Hz |
| 126 | 2.100 Hz | 1.050 Hz | 0.525 Hz |
| 150 | 2.500 Hz | 1.250 Hz | 0.625 Hz |

Only 126 BPM, N = 32 has a measurement (hardware envelope line 2.098 Hz). The other cells are the formula under the assumption, **not** oracle output: the oracle cannot give them while the writer is open.

## 4. Native engine over the same inputs

New tests in `fx/tests_fw_pump_sync.rs`: `the_duck_rate_is_bpm_over_60_per_division_at_the_four_check_tempos` (60, 90, 126, 150 BPM x 2, 1, 0.5 beat divisions, within 3 percent + 0.15 Hz of `bpm / 60 / beats`) and `the_first_duck_falls_on_the_clock_start` (gain below 0.5 within 240 frames at 120 BPM). Both pass without engine changes: they pin the engine's own law, not the firmware. No difference to fix was found (D152).

## 5. State of the flip (PR #491 untouched, not flipped)

| Condition for default-safe firmware PUMP | Status |
|---|---|
| Routine bit-exact against the oracle | done (500/500) |
| ARM pointer and `N` under MST CLK | done (128/128) |
| Free run rate (6.108 Hz at rate 73) | done |
| Clock word writer, span, zero point | **open** (not found in the paths run) |
| Hardware clip with a known downbeat or a second tempo | **open** (one clip: A:25, 126 BPM) |
| Phase at note-on / program load | **open** (follows from the writer) |

## Implications

- `0xC0230` is outside the SMP frame; the producer is DMA into the strip from another DSP, or code there. The engine clock word (`fw_pump_clock.rs`) stays an ASSUMED stand-in.
- A beat locked word needs a tempo input. The ARM sends none to any DSP in the A:25 run, so the tempo was either sent before the watch (at load) or the word is not tempo derived (a free running DSP counter would make the rate independent of BPM). The single 2.10 Hz match cannot tell these apart.

## Follow-ups

1. Run the watch from program load (messages before tick 800) and across a tempo change; the `Edt_Com` store is not among `find_stores` results.
2. Decode XMOD / YMOD of the strip descriptors, map block 3 to its transmit set, and run the ORG and PNO core 2 transmit buffers for a counter-like word at that row.
3. A second hardware clip at another tempo (for example 90 BPM, rate 73) settles tempo dependence.
