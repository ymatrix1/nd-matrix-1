# Research: the SY (sample) chip live on the ARM-DSP link (D153)

- **Checked:** 2026-11-17
- **Sources (local, never committed):** SY1SY2 image `target/oracle/sy.bin`, OS v1.66 `target/fw_unpacked/bank0_CODE.bin` and `bank0_SRAM.bin`, factory restore EEPROM `target/oled/eeprom.bin`, factory programs `Acoustic Guitar`, `String Quintet`, `Mm-Oh Choir Whl` (Bank A), factory backup `.reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b` (stand-in sample binding only). Numbers and addresses only; no firmware, sample or audio content is quoted. Run data: [data/sy-live-*.json](data/).
- **Question:** Can the SY chip run as a live chip behind `sharc-live` (interrupt driven, with the DSP to ARM return source), so SAMPLE programs run end to end with the real ARM program? Does the voice play past the first ring, and how do voice busy, free and key-down timing compare with the message driven stub of D147?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by a run or listing named here, **EMPIRICAL** = measured in this harness, **ASSUMED** = reading.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| What runs the SY pass? | Interrupts, not a poll loop: vector 17 (`P6I`, a `LIRPTL` slot) is the audio ISR `0x1C05B9`, one stereo frame per ISR; vector 13 (`P2I`) is the FIFO reset handler `0x1C03FD`, which sets `I0 = 0x93046`. One pass = 2 ISRs (ARM tick 24 kHz, SY frame 43,402.34 Hz, 0.9042 passes per tick) | VERIFIED handler addresses and the 2-frame pass (`sy-smp-audio.md`, `sharc-ref` `sy_isr_refill.scn`); ASSUMED that vector 13 is the ARM SPI-receive DMA completion (the SPI block is the ARM link, D141) |
| What feeds the FIFO? | ARM messages are `[opcode:4 \| addr:28], value` pairs, opcode 0 = store; ring base `0x93046`, index `DM(0x2400)`, `n = min((DM(0x2400) + R3 - I0) >> 1, DM(0x92180))`, `DM(0x92180) = 0x40` | VERIFIED by oracle dump |
| What does the ring refill DMA do? | The real SY refills the 128-frame staging ring by external-port DMA chains (AMI banks 2 and 3, offset table `DM(0xB2000..)`, D141). No external image exists, so the stand-in is the D111 synthetic header fill of the staging words of every prepared voice, every frame. **The audio is meaningless; only control flow is tested.** `--epchain` (D141 model) exists and is off | EMPIRICAL (run) / ASSUMED (stand-in) |
| Does the live chip run without faults on the real ARM program? | Yes, 3 of 3 programs: 4,882 to 7,422 ISRs per run, 0 errors, 0 halts | EMPIRICAL |
| **Why did D147's stub never send `go`?** | The ARM sends a state store and **polls its effect inside one ARM tick**: all 40,682 reads of `DM(0x9219A)` happen in tick 901, before the harness delivers that tick's messages to the chip, so the polled bit can never rise (stub: the bit is never set; live chip: it never saw `prepare`). The count is the same 40,682 with the stub, with the live chip and with `--poll-advance 0`. D147 attributed these reads to the wait at `0xC01E0694`; they are a second site, return `0xC01E26FC` | VERIFIED (read log, `wait_ticks`, run 4 below) |
| With the poll fed (`--poll-advance 64`)? | The second wait passes after 192 reads (1 of 192 tests) instead of timing out at 40,682 (0 passes); all 3 chord notes send `prepare` and `go` (stub: 1 `prepare`, 0 `go`) | EMPIRICAL |
| Does the ring stay fed? | Yes in the sense of the model: refills 9,726 (Guitar, Quintet, 3 voices) and 19,560 (Choir, 6 voices) with the chip alive to the end of the run (2,700 ticks) | EMPIRICAL |
| Does the voice play past the first ring? | **Not shown.** Choir voice 0 reaches the playing state `0x1C0851` at tick 901 and stays there for the whole run with non-silent output in 1,627 ticks, but the step word `DM(0x9242D)` reads 0, so the source frames consumed cannot be counted (0.0) and "past the first 128 frame ring" is **not established** (the 0.054 peak is the synthetic header fill, not a sample). Guitar and Quintet voice 0 never leave the waiting/release alternation | EMPIRICAL |
| Voice free timing | Not observed: the ARM sends no SY state store after the last note-on in the 300 ticks after note-off (state stores stop at tick 941), live or stub. `voice0_free_after_off_ticks` is `None` in all 6 runs | EMPIRICAL |

## 2. Interrupt evidence (what the model does)

`crates/nord-sharc/src/runner/live_sy.rs`, opt-in chip spec `SY:0=sy=<image>` (`sharc-live --sy`, `--isrs N`, default 2 per pass). Per pass the model:

1. feeds the queued command pairs into the FIFO at `0x93046` and raises vector 13 (`run_irq`: temporarily unmasks the vector, raises it, steps until the handler returns), as the D141 SPI completion;
2. refills the staging words of every prepared voice (record `0x92400 + 0x38 k`, staging field `+0x2F`, 128 words, wrap at `0x93FF7`) with the D111 synthetic header;
3. raises vector 17 `N` times (`ISR_MAX_STEPS = 2,000,000` guards a runaway handler);
4. reports `sy=isrs:..,resets:..,pairs:..,queued:..,refills:..,staged:..` in the `P` reply.

Voice state words (record `+0x14`, `0x92414 + 0x38 k`): `0x1C0794` reset, `0x1C07B0` idle, `0x1C07CD` prepare, `0x1C0811` waiting, `0x1C083C` release (sets bit `k` of `DM(0x9219A)`), `0x1C084C` go, `0x1C0851` playing; idle mask `DM(0x92196)`, release mask `DM(0x9219A)`; waiting turns into release when `+0x10 + +0x1A > 0x52`. In the return source the live chip answers before the message driven mask (`ReturnSource` layers: live, SY mask, shadow, image, zero).

## 3. Runs

Programs: Acoustic Guitar and String Quintet (Bank A, SAMPLE), Mm-Oh Choir Whl (Bank A, 6 voices for the chord). Chord 60, 64, 67 at ticks 900, 920, 940, off at tick 2400, 1,500 key ticks, stand-in sample bound after the idle phase (`--ns4b`). Live = `SY:0=sy=...` with `--poll-advance 64`; stub = the D147 message driven mask (`PNO:2=org=...` run, no live SY).

| Program | Mode | prepare / go sent | key-down delay (ticks) | voice 0 busy | voice 0 state | second wait (`0x9219A`) tests, passed | chip ISRs / refills |
|---|---|---|---|---|---|---|---|
| Guitar | stub | 1 / 0 | 1, none, none | 901 | none | 40,682, 0 | none |
| Guitar | live, poll 64 | 3 / 3 | 1, 1, 1 | 901 | waiting/release alternation from 901 | 3 x 192, 1 each | 4,918 / 9,726 |
| Quintet | stub | 1 / 0 | 1, none, none | 901 | none | 40,682, 0 | none |
| Quintet | live, poll 64 | 3 / 3 | 1, 1, 1 | 901 | waiting/release alternation from 901 | 3 x 192, 1 each | 4,918 / 9,726 |
| Choir | stub | 2 / 1 | 1, none, 563 | 901 | none | 40,682 + 2,452,938, 0 | none |
| Choir | live, poll 64 | 6 / 6 | 1, 1, 1 | 901 | **playing `0x1C0851` from 901, held to the end** | 6 x 192, 1 each | 4,954 / 19,560 |

Other numbers:

- The first wait (`0xC01E072C`, idle mask `DM(0x92196)`, descriptor in `r5`) passes at its single test in every run.
- Guitar and Quintet give identical numbers: the live voice behaviour depends on the staged header, not on the program; the Choir header lets `+0x10 + +0x1A` stay under `0x52`, the others do not, so they alternate waiting and release (ASSUMED reading: a property of the synthetic stand-in, not of the real data).
- Without the feed in the poll (`--poll-advance 0`, and with the poll but no message feed): the live chip also times out (0 of 40,682), 635 extra passes made no difference. The feed is what matters.
- The 2,452,938 Choir stub tests are the same wait spinning for 600 ticks.

## 4. What still blocks (counts)

| # | Blocker | Count / evidence | Next step |
|---|---|---|---|
| 1 | Poll inside one ARM tick: the harness delivers messages and runs passes only between ticks. `--poll-advance N` is a **workaround** (it feeds and runs 1 pass per `N` reads, so those passes are not in the tick's audio report, and `consumed()` undercounts) | 40,682 reads in tick 901 without it, 192 with it | Run the chip on a real-time clock inside the instruction loop instead of per tick |
| 2 | The ring refill stand-in: synthetic header, no real sample data (D141 AMI/EP chains need an external image) | refills 9,726 / 19,560, content meaningless | `--epchain` with a sample window once an external image exists |
| 3 | Voice 0 of Guitar and Quintet alternates waiting and release (state changes at ticks 901, 905, 906, 907, 909, 916, ...) and never reaches `go`/playing although `go` was sent 3 of 3 | state samples `transitions`, 2,700 samples | Fix the synthetic header so `+0x10 + +0x1A <= 0x52`, or find who writes `+0x1A` in the real flow |
| 4 | Choir plays but the step word reads 0, so no source-frame count and no "past the first ring" verdict | `source_frames` 0.0, 1,627 non-silent ticks | Find the writer of `DM(0x9242D)` (pitch table entry `e`, OQ22) |
| 5 | No SY state store after note-off, so no voice free, release or stealing timing | 0 stores in 300 off ticks, 6 of 6 runs | The note-off path of the ARM (release state `0x1C083C` from the ARM side) |

Not found in the paths run is not proof of absence: only these 3 programs, one chord, one key-down and one note-off were run, with the stand-in sample.

## 5. Implications

- The D147 return-path numbers hold, but the 40,682 reads belong to the wait at return `0xC01E26FC` (`DM(0x9219A)`, descriptor in `r4`, index 0, mask `1 << k`), not to `0xC01E0694`.
- `go` is only sent once the release bit is seen, so a stub that sets that bit on `prepare` would unlock `go` without the chip; the live chip gives the bit its real timing relative to the ISR clock.
- The live chip changes the key-down to `go` sequence from 1 prepare to 3 prepare plus 3 go for a 3 note chord, and for Choir the voice reaches the playing state, which the stub never did.

## 6. Follow-ups

1. Real-time chip clock inside the ARM instruction loop (replaces `--poll-advance`).
2. Writer of the step word `DM(0x9242D)` and of `+0x1A`; real sample data through `--epchain`.
3. ARM note-off path for SY voices; stealing with more voices than the 56 records.

## Reproduce

```bash
cargo build --release -p nord-sharc --bin sharc-live
uv run pytest tools/tests/test_arm_sy_timeline.py tools/tests/test_arm_dsp_return.py tools/tests/test_arm_dsp_live.py tools/tests/test_arm_dsp_route.py tools/tests/test_arm_dsp_link.py
M=.   # main checkout with target/ and .reference/
C=$M/target/fw_unpacked
PYTHONPATH=tools uv run --with unicorn --with capstone python tools/arm_dsp_live.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
  --eeprom $M/target/oled/eeprom.bin --program "$M/factory_programs/Program/Bank A/Mm-Oh Choir Whl.ns4p" \
  --chip "SY:0=sy=$M/target/oracle/sy.bin" --live target/release/sharc-live --return-path --poll-advance 64 \
  --sy-timeline --keys 60,64,67 --key-ticks 1500 --ns4b "$M/.reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b" --out run.json
```

Stub: `--chip "PNO:2=org=<core2 image>"` instead of the SY chip. A run takes 4 to 8 minutes (about 450 M ARM instructions of boot).
