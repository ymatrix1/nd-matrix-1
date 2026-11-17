# Research: a live link between the ARM harness and one emulated DSP chip (PNO core 1) (D138)

- **Checked:** 2026-11-17
- **Sources (local, never committed):** OS v1.66 `target/fw_unpacked/bank0_CODE.bin` and `bank0_SRAM.bin`, factory restore EEPROM `target/oled/eeprom.bin`, program `factory_programs/Program/Bank A/B3 Chords n Solo.ns4p`, PNO core 1 SRAM image `target/PNO/core1_sram.bin`, boot log `target/bulk/pno_core1.script`. Tools (new): `tools/arm_dsp_link.py`, `tools/arm_dsp_live.py`, `crates/nord-sharc/src/runner/live.rs`, `sharc-live`. Data (numbers only): [arm-dsp-live-link.json](data/arm-dsp-live-link.json). Background: [arm-fw-emulation](arm-fw-emulation.md), [arm-organ-messages](arm-organ-messages.md), [arm-tick-timers](arm-tick-timers.md) (D130), [arm-dsp-capture](arm-dsp-capture.md) (D124), [dsp-peripherals-audio](dsp-peripherals-audio.md) (D120), [b3-leslie-pno-lane](b3-leslie-pno-lane.md) (D126), [dsp-frame-rate](dsp-frame-rate.md), [fx-leslie](fx-leslie.md).
- **Question:** Can the ARM firmware harness and one SHARC oracle chip run together, so that the ARM's own messages reach the chip as the firmware sends them (no capture file, no replay step), and does that open the paths that stayed closed in replay (for PNO core 1 with a B3 program: the Leslie processor `0x1256AD`, D126)?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by the command in "Reproduce", **EMPIRICAL** = measured in this harness, **ASSUMED** = reading or placeholder.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Does a live link work? | **Yes, one direction.** The ARM harness boots the real firmware to the main loop (500.2 M instructions), recalls B3 Chords n Solo, holds MIDI key 60 and releases it, while `sharc-live` (PNO core 1) receives **30,067 ARM words as they are stored** and runs **2,441 passes** coupled to the 24 kHz ARM tick. All 2,441 passes return to the poll loop (no fault, no step limit). | VERIFIED (run, section 5) |
| Chip to ARM direction | **Telemetry only.** The chip's replies (pass summary, lane level, probe hits, any data word via `D`) reach the Python side every batch, but nothing is written back into ARM state: the only ARM-visible DSP answers are the GPIO toggle handshake (`0xC006CA48`, stubbed to 1) and the SY voice-ready bit (`DspAck`); neither has an identified PNO-side writer. | VERIFIED (design limit, section 4) |
| Which ARM to DSP words arrived? | Boot (before the main loop) 22,389; load phase (800 ticks) **7,340**; idle (100) 3; key held (1,500) 180; release (300) 155. Total 30,067 words, **8,398 distinct data words**. 5,910 further PNO core 1 path 2 stores lie outside the 1 M word core window (bulk stores, `0x200xxxxx`) and are counted, not sent. | VERIFIED (run) |
| Same state as the replay? | Of the **8,306** distinct words of the old replay (`pno_core1.script` + `org-system-load-pno1.script`), **8,306 are present** in the live stream (0 missing); 27 differ in value (10 in `0x95xxx`, 7 in `0xB0xxx`, 5 in `0xB4xxx`, 3 in `0xB2xxx`, 2 in `0xE0xxx`; another program and settle length), 92 words are new (key phase: 59 in `0x95xxx`, 33 in `0xB4xxx`). The load phase sends exactly **7,340** words, the count of the committed load stream. | VERIFIED (script diff) |
| Is the live run equal to a replay of its own stream? | **Yes, exactly.** `sharc-sys` fed the messages `sharc-live` received (pass index = passes run before the message) gives lane `2:0` peak **0.074589**, rms **0.017788** in both, 2,441 passes; two live runs give a byte-identical message stream and equal per-phase numbers (section 5). | VERIFIED |
| Does `0x1256AD` run now? | **No: 0 of 2,441 passes.** Also `0x1237F8` and `0x1236F8` (cabinet): 0 of 2,441. B3 processor `0x121E2B`: **2,143 of 2,441** (from pass 298 on, i.e. after the recall settles); chain loop `0x1213E9`: 2,441 of 2,441. The ARM sent **no word in `0xC2E00..0xC2FFF`** (chain heads `0xC2F48, 0xC2F82, 0xC2FBC, 0xC2FD6` stay at the stub `0xC2E4A`) and no word with the value `0x1256AD`. The Leslie element descriptors `0xB0476` / `0xB047E` arrive at pass 160 and no pass reads `0xB0470..0xB048F` (traces of passes 161 to 163 and 298 to 300). | VERIFIED (probe, `sharc-ring`, stream scan) |
| Lane level against the old replay | Live: peak 0.0746, rms 0.0178 over 2,441 passes with key 60 held from pass 813 to 2,170. Old replay (boot log + load + `key1`, 2,016 passes): peak 0.0674, rms 0.0249. Not the same note, program build or window, so the levels are the same order of magnitude, not a comparison. | EMPIRICAL |
| Verdict on the gap | **Not closed by the in-window ARM stream under the conditions of this run.** Narrow statement VERIFIED: the ARM sent no word in `0xC2E00..0xC2FFF` and none equal to `0x1256AD`. It does not cover: the 5,910 PNO core 1 stores outside the core window (bulk path, counted not sent), a DSP reply that the ARM may wait for (reply direction missing), other chips, the DSP running during the boot, or the 16 x denser ARM stream per DSP pass. If the installer is reached through the bulk path or a reply handshake, this run cannot see it. | EMPIRICAL (the narrow statement VERIFIED) |

## 2. Design (written before the implementation)

Goal: the smallest live link, one chip, ARM messages in, chip state out, deterministic.

**Which chip.** PNO core 1 with a B3 program: it has the most known gaps tied to ARM input (Leslie gate D126, rotary words, B3 processor `0x121E2B`), its image and preset (`--pno`) are in `sharc-sys`, and 22,389 of its words come from the boot itself. ORG/SY (synth envelope words, D132) needs a different oracle preset per core and the SY ring; it is the second step.

**Transport.** A child process with a line protocol on stdin/stdout:

| Option | Verdict |
|---|---|
| File (capture then replay) | what exists; cannot carry replies, and is the thing to remove |
| pyo3 / in-process Rust | needs a Windows toolchain plus GIL handling; couples the build of the emulator to Python; no benefit over a pipe at 30,000 words per run |
| Rust CLI embedding the ARM output | the ARM side is `unicorn` in Python; a Rust ARM core does not exist |
| **Pipe to `sharc-live`** | chosen: no clock, no thread in the chip, replies a pure function of the commands |

Protocol (`runner/live.rs`): `M W1 W2` store a data word before the next pass; `P N` run up to `N` passes and reply `P req= idle= steps= last= pass= halted= cnt= sumsq= peak= [hits=pc:n,...] [error=]`; `T 0|1` coverage on/off; `D ADDR N` read data words; `Q`. `M` and errors produce **no output** (an unsolicited error line filled the pipe and deadlocked the first two runs; fixed, the error is reported by the next `P`).

**Message format.** The ARM stores `(address, [pad], value)` to the EMIFA ports. The harness groups the stores incrementally (`PortGrouper`, equal to `arm_emu.group_port_writes` for any split of the stream, unit test) on the real ports during the boot and on the fake ports after it. A PNO core 1 path 2 message becomes `M dm_word value` with `dm_word = (address - 0x28000000) / 4` ([arm-param-messages](arm-param-messages.md) s2).

**Ordering.** Messages reach the chip in ARM store order. The granularity is one ARM tick: all stores of a tick land before the next pass, the order inside the tick is kept. The real SPI/DMA path delivers inside a pass at interrupt time; the oracle has no such interrupt, as in every replay (ASSUMED, same as `sharc-sys`).

**Clock coupling.** ARM timer 24 kHz (D130). DSP pass `2 / F`, `F = 43,402.34375 Hz = 1,388,875 / 32`, so a pass is 46.08 us and **0.9042 passes per ARM tick**; the owed passes after `t` ticks are `floor(t * 1,388,875 / 1,536,000)` (integer, `Clock`, unit test). The DSP clock **starts at the main loop** (ASSUMED: the ARM downloads the DSP images during the boot, the harness stubs the download); boot messages are queued and land before pass 0, as `--script-flat` does today. The harness keeps its 200,000 instructions per tick (16 x the 12,500 cycles of the 300 MHz ARM, [arm-tick-timers](arm-tick-timers.md) s5): ARM work per tick is 16 x what the hardware does, so the DSP sees the ARM stream 16 x denser in DSP time than on hardware (`--insns-per-tick` changes it).

**Out of scope.** Other chips (their messages are counted: FX, ORG, SY, SMP in section 3), SPI/DMA transfers and sample flash, a DSP to ARM reply that changes ARM state, real-time speed (the pass takes 1.15 ms interpreted, 25 x the period, `sharc-sys` figures), the GPIO handshake, interrupts inside a pass, the FX chip hand-over.

## 3. Implementation

| Part | File | Tests |
|---|---|---|
| Chip server: command interpreter, PNO preset, boot | `crates/nord-sharc/src/runner/live.rs`, `src/bin/sharc_live.rs` | 6 + 2 Rust unit tests (round trip, idle/halt/lane/probe summary, equality with `lane_run`, error replies, preset, arguments); `sharc-sys --pno` now uses the same `pno_preset()` |
| Clock, incremental grouper, protocol client, stats | `tools/arm_dsp_link.py` | `tools/tests/test_arm_dsp_link.py` (12, including the read timeout and a dead server) |
| Firmware driver (boot with feed, phases, key, report) | `tools/arm_dsp_live.py` | `tools/tests/test_arm_dsp_live.py` (4, feeder and report helpers) |

ARM messages for other chips are counted per chip, core and path (`arm_messages` in the JSON).

## 4. Why the reply direction stops at telemetry

Three ARM-visible DSP answers are known: (a) the GPIO toggle handshake of the flow control `0xC006CA48` ([arm-dsp-protocol](arm-dsp-protocol.md) s4: a DSP-driven input toggles, the ARM pulses an output), (b) the SY voice-ready status bit that `DspAck` sets (ASSUMED meaning, [synth-sample-kernel-rule](synth-sample-kernel-rule.md) s12), (c) the 3-word read port of the DSP receive descriptor ([dsp-rx-protocol](dsp-rx-protocol.md)). The oracle models none of the DSP's output pins or its transmit path, and the PNO core 1 passes in this run touch no DSP-side writer of an ARM-visible state (no ARM read of a PNO answer is known). A reply would therefore be invented, not measured. The `D` command already returns any DM word each batch, so a mapping "DM word to ARM memory word" can be added the moment one is found.

## 5. Numbers (run `arm-dsp-live-link.json`)

| Phase | ARM ticks | Words sent | Passes | `0x121E2B` | `0x1213E9` | `0x1256AD` | `0x1237F8` | `0x1236F8` |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| boot (to main loop) | 0 (500.2 M insns) | 22,389 | 0 | 0 | 0 | 0 | 0 | 0 |
| load | 800 | 7,340 | 723 | 425 | 723 | **0** | 0 | 0 |
| idle | 100 | 3 | 90 | 90 | 90 | **0** | 0 | 0 |
| key 60 held | 1,500 | 180 | 1,357 | 1,357 | 1,357 | **0** | 0 | 0 |
| released | 300 | 155 | 271 | 271 | 271 | **0** | 0 | 0 |
| total | 2,700 | 30,067 | 2,441 | 2,143 | 2,441 | **0 of 2,441** | 0 | 0 |

Wall time 97 s (boot 30 s). Passes per tick: 2,441 / 2,700 = 0.904. ARM messages by chip and core over the run (all paths): PNO core 1 35,977 (30,067 sent + 5,910 outside the window), PNO core 2 13,509, ORG core 1 25,329, ORG core 2 13,509, FX 3,495 and 2,765, SMP 1,540 (path 1), SY 290 (path 1).

Replay of the same stream (`sharc-sys --msg-script live.script`): 2,441 passes, 30,067 messages, lane `2:0` peak 0.074589, rms 0.017788, hashes of `0xB0000..0xB2FFF` `e6c44513a2b06bd8` and `0xC0000..0xC2FFF` `7d72a6693b8f47d7`; the second live run gives a byte-identical message stream (`cmp`) and equal per-phase numbers (`determinism` in the data file).

## 6. Implications

- **The transport works and costs little**: 150 lines of Rust, 250 of Python, no change to the oracle core. The same pattern serves any other chip (ORG core 2 and SY need their own preset, one `Preset` value each; `sharc-sys --pno` already shares `pno_preset()`).
- **The in-window ARM stream does not close the B3 Leslie gate (EMPIRICAL, narrow).** Even live, with 1,500 ticks of a held key, no chain-head word and no `0x1256AD` word is sent in the window, and the descriptors the ARM does send (`0xB0476`, `0xB047E`) are never read by a pass. Something else must trigger the DSP side consumer, or a DSP answer is required first (unmeasurable until the reply direction exists), or the installer sits behind the bulk path (5,910 stores not sent), or the Leslie of this program runs on the FX chip (PR #486, D135, still open). PR #493 (D136, open) reaches a similar negative from the recall and edit stream; the two are independent evidence for the same gap and touch the same `audio-remaining.md` row 9, so whichever merges second merges that row by hand.
- **Protocol equivalence is tested, hardware equivalence is not claimed**: `server_passes_equal_lane_run_on_the_same_messages` (Rust) checks that the server's summary equals `lane_run` for the same messages, and the manual run found the live lane equal to a `sharc-sys` replay of its own stream (`cmp` of two live streams too, 97 s each, not automated). Both sides share the oracle code, so this checks pass-index alignment and the probe/non-probe pass variants, not the oracle against hardware. The load stream count matches the committed one (7,340).
- **Synth envelope words** (D132) need the ORG/SY live chip: the transport carries over, the preset does not exist yet.

## 7. Follow-ups

1. Add ORG core 2 and SY as live chips (preset per core, several `sharc-live` processes in one run, shared clock), to see whether the synth envelope words appear when the synth recall runs against a live SY.
2. Reply direction: find a PNO-side word the ARM polls (OQ36 PUMP counter is the same question for FX) and write it back into ARM memory at each batch; then re-run to see whether the chain-head words appear.
3. Find the DSP-side trigger of the chain installer (what reads `0xB0474..0xB048F`): trace the PNO image for loads of that window with a mode word set.
4. DSP clock from the main loop is ASSUMED: compare with the boot messages delivered one pass at a time (`--dsp-from-boot`).
5. Automate the two-run `cmp` determinism check and the live-vs-`sharc-sys` replay comparison (`replay()` prints, asserts nothing).
6. Compare live lane against a hardware clip of the same program and key once the key and settle times match (the 16 x instruction budget per tick, section 2, changes the ARM to DSP spacing).

## Reproduce

```bash
cargo build --release -p nord-sharc --bin sharc-live --bin sharc-sys
C=target/fw_unpacked; R=target/release
uv run --with unicorn --with capstone python -u tools/arm_dsp_live.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
  --eeprom target/oled/eeprom.bin --program "factory_programs/Program/Bank A/B3 Chords n Solo.ns4p" \
  --dsp-image target/PNO/core1_sram.bin --live $R/sharc-live --sys-exe $R/sharc-sys \
  --probe 0x1256AD --probe 0x121E2B --probe 0x1213E9 --probe 0x1237F8 --probe 0x1236F8 \
  --log live.script --out live.json                      # about 100 s
sharc-ring target/PNO/core1_sram.bin --script-at 0:live.script --trace 0xB0470:0x20 --from 298 --to 301
uv run pytest tools/tests/test_arm_dsp_link.py tools/tests/test_arm_dsp_live.py
cargo test -p nord-sharc --lib runner::live
```
