# Research: the live ARM to DSP link on the organ-program chips, ORG core 2 and PNO core 2 (D144)

- **Checked:** 2026-11-17
- **Sources (local, never committed):** OS v1.66 `target/fw_unpacked/bank0_CODE.bin` and `bank0_SRAM.bin`, factory restore EEPROM `target/oled/eeprom.bin`, programs `factory_programs/Program/Bank A/Smooth Lead.ns4p` and `12dB Sweep.ns4p`, ORG core 2 image (`tools/ns4_dsp_ldr.py target/DSP/dsp_image_2_ORG.bin --sram DIR`), PNO core 2 image `target/PNO/core2_sram.bin`. Tools: `crates/nord-sharc/src/runner/live_presets.rs`, `runner/live.rs`, `sharc-live` (`--org`, `--window`), `tools/arm_dsp_route.py`, `tools/arm_dsp_live.py`. Data (numbers only): [arm-dsp-live-link-synth.json](data/arm-dsp-live-link-synth.json). Background: [arm-dsp-live-link](arm-dsp-live-link.md) (D138), [synth-e2e-oracle-vs-native](synth-e2e-oracle-vs-native.md) (D132), [arm-dsp-capture](arm-dsp-capture.md) (D124), [pno-jit-arm-profile](pno-jit-arm-profile.md), [sy-smp-audio](sy-smp-audio.md), [sharc-sy1sy2-smp](sharc-sy1sy2-smp.md), [arm-tick-timers](arm-tick-timers.md) (D130).
- **Question:** Can several `sharc-live` chips run on one ARM stream with one shared clock, so that a synth program recall plus a held key reaches ORG core 2 / PNO core 2 (and SY) as the firmware sends it, and do the synth envelope words that are missing from the D132 recall stream appear?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by the command in "Reproduce", **EMPIRICAL** = measured in this harness, **ASSUMED** = reading or placeholder.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Several live chips on one clock | **Yes.** Two `sharc-live` processes (ORG core 2, PNO core 2) run under one 24 kHz ARM tick; after every tick both run `floor(t * 1,388,875 / 1,536,000)` passes in total (same `Clock`, 2,700 ticks, **2,441 passes each**, all 2,441 return to the poll loop, no halt). `arm_dsp_route.Router` sends each ARM message by `(comm, core, path)`; unrouted messages are counted. Two runs of Smooth Lead give byte-identical message streams for both chips and equal per-phase numbers | VERIFIED (run, `cmp`) |
| New presets | `--org`: entry `0x125C88`, poll loop `0x125DDF`, mailbox `0xB00E2`, boot 100,000, gates `0x282C0380 = 1`, `0x3108D008 = 5` (the `sharc-sys` defaults) serves ORG core 1 / 2 and PNO core 2 (same program, loader entry `0x125C88`). `--window ADDR:COUNT:STRIDE` reads the level from the voice record (`0xC1F89`, 4 samples) because the synth voice ends in the channel record, not in a lane | VERIFIED (unit tests, runs) |
| Which ARM words reach each chip (Smooth Lead) | **ORG core 2:** boot 12,601 words (11,280 distinct), load 912 (822 distinct), idle 0, key phases **0**. **PNO core 2:** the same 12,601 + 912, key phase **2** words (`DM(0xC1DCD)`, `DM(0xC1DCC)`), release 2. Words that reach no live chip (counted): PNO core 1 35,563, ORG core 1 25,333, FX 3,501 + 2,765, SMP path 1 1,540, SY path 1 **290** | VERIFIED (run) |
| Does the stream equal the offline capture? | **Yes.** The 912 ORG core 2 load words are identical in value and order to `arm_dsp_capture one` of the same program (`cmp` of the sequences); the 912 PNO core 2 load words likewise. The capture also has the notes 36 and 84 (2 + 2 words each) that the live run does not play | VERIFIED (script diff) |
| Do the missing envelope words appear? | **No new words appear.** The live load stream is the D132 stream (above), so no envelope parameter word is sent that the capture lacked; the chip does not answer into ARM state, so nothing can change the ARM's choice of words. See section 3 for the chain states, which **do** differ per program here | VERIFIED |
| Where the key goes | Only to PNO core 2, never to ORG core 2 (D132 confirmed live: 0 key words reach ORG core 2). **ORG core 2 stays silent** (voice window peak 0.0, 9,764 samples), the filter stage `0x12566E` still runs in 2,234 of 2,441 passes (first pass 207) | VERIFIED |
| Voice level against the old replay (Smooth Lead) | Live PNO core 2: window peak **0.449895**, rms **0.130574** (9,764 samples). Replay of its own stream through `sharc-sys`: identical (peak 0.4498952627, rms 0.1305744982). D132 way (ORG core 2 image, its recall log + the 4 key words of PNO core 2 merged by pass, `--old-replay ORG2,PNO2`): peak 0.4498952627, rms 0.1305744982, **equal to the live PNO core 2 to all printed digits**. 12dB Sweep: live peak 0.202917, rms 0.084409; old way peak 0.203037, rms 0.084449 (+0.06 % peak, the images differ by 16 bytes of data) | VERIFIED |
| SY (SY1 / SY2) live chip | **Not built: a design blocker, section 4.** 290 SY path 1 words are counted | VERIFIED (negative) |

## 2. Implementation

| Part | File | Tests |
|---|---|---|
| Presets as data (`pno`, `org`), boot | `crates/nord-sharc/src/runner/live_presets.rs` (moved from `live.rs`) | 3 |
| Voice window level (`with_window`), `--org`, `--window` | `runner/live.rs`, `src/bin/sharc_live.rs` | 1 + 3 (new) |
| Chip spec `CHIP:CORE=PRESET=IMAGE[=WINDOW[=PROBES[=WATCH]]]`, `Router`, `WatchStats`, merged replay script, tap levels | `tools/arm_dsp_route.py` | `tools/tests/test_arm_dsp_route.py` (10) |
| Multi-chip driver (`--chip` repeatable, one clock, per-chip log, replay, `--old-replay`) | `tools/arm_dsp_live.py` | `tools/tests/test_arm_dsp_live.py` (7) |

The D138 one-chip flags (`--dsp-image`, `--probe`) still work and build `PNO:1=pno=IMAGE`. Messages are sent to a chip in ARM store order; the granularity is one ARM tick (as in D138). Every chip sees the same pass count, so the ORG and PNO chips are time aligned.

## 3. Numbers (Smooth Lead and 12dB Sweep, key 60 held 1,500 ticks, 2,700 ticks)

| Phase (ticks) | passes | ORG core 2 words | PNO core 2 words | `0x12566E` hits (both chips) | PNO core 2 voice peak after the phase |
|---|---:|---:|---:|---:|---|
| boot (500.2 M insns) | 0 | 12,601 | 12,601 | 0 | |
| load (800) | 723 | 912 | 912 | 516 | |
| idle (100) | 90 | 0 | 0 | 90 | |
| key 60 held (1,500) | 1,357 | 0 | 2 | 1,357 | key at pass 813 |
| released (300) | 271 | 0 | 2 | 271 | |
| total | 2,441 | | | 2,234 | 0.4499 (ORG core 2: 0.0) |

Wall time 119 s per run (boot 30 s). Probe `0x121C54` and `0x1253BD` (stage words of other programs of D132): 0 hits; the Smooth Lead filter stage is `0x12566E`, which hits identically on both chips.

**Envelope chain states** (`0xC1174`, `0xC1180`, `0xC118E`, `F = 0xC1188`, float words), per program, as `(first change pass, max, final)`; the live key goes to pass 813 and the chain starts at pass 815 or 816:

| Program | Run | `0xC1174` | `0xC1180` | `0xC118E` |
|---|---|---|---|---|
| Smooth Lead | live PNO core 2 (sampled per tick batch) | (816, 1.0, 0.9393) | (816, 1.0, 0.8972) | (816, 0.0315, 0.0000) |
| Smooth Lead | its stream on the ORG image, live timing | (815, 1.0, 0.9393) | (815, 1.0, 0.8972) | (815, 1.0, 0.0001) |
| Smooth Lead | D132 timing (recall flat at pass 0, key at 300, off at 11,151) | (301, 1.0, 0.2841) | (301, 1.0, 0.1133) | (301, 0.2518, 0.0) |
| 12dB Sweep | live PNO core 2 (sampled per tick batch) | (816, 1.0, 0.9195) | (816, 0.0660, 0.0653) | (815, 1.0, 0.0000) |
| 12dB Sweep | its stream on the ORG image, live timing | (815, 1.0, 0.9195) | (815, 0.066, 0.0653) | (815, 1.0, 0.1602) |
| 12dB Sweep | D132 timing | (301, 1.0, 0.1855) | (301, 0.5278, 0.4328) | (300, 1.0, 0.0) |

(The live rows sample once per ARM tick batch, so a short peak such as `0xC118E` of Smooth Lead (0.0315 sampled, 1.0 tapped every pass) can be missed; the replay rows tap every pass. The live PNO core 2 and the replay on the ORG core 2 image agree on 5 of 6 finals to 3 decimals; the exception is 12dB Sweep `0xC118E` (0.0000 on the PNO core 2 image, 0.1602 on the ORG core 2 image), and the voice peak differs by 0.06 %: the two images are not identical, cause open.)

Reading:

- The chain trajectories **differ between the two programs** in every run, live and D132 timed. That does not reproduce the D132 s1 row "identical ramp in all five programs"; a program-specific word reaches the chain in this stream, or the trajectories depend on a recalled word that D132 did not compare (cause open, EMPIRICAL). The live run and the replay of the same stream on the same image agree (Smooth Lead: every level digit), so the difference between programs is not the live link.
- The chain traces **differ between the two timings** (recall spread over 723 passes against all words at pass 0), so the envelope state depends on when the recall words arrive relative to the key. D132 used the flat delivery.

## 4. SY1 / SY2: the blocker

A live SY chip needs a different chip model, not a `Preset`:

| Need | Evidence | Status |
|---|---|---|
| Pass model | SY1SY2 and SMP have **no mailbox pass**: they start at FLAG12 and are interrupt driven ([pno-jit-arm-profile](pno-jit-arm-profile.md) s1 row "Other cores", [sharc-sy1sy2-smp](sharc-sy1sy2-smp.md) s4); the voice start is FIFO reset vector 13, then one audio interrupt (vector 17) per output frame ([sy-smp-audio](sy-smp-audio.md) s1), i.e. two per pass, not a poll loop | `Live::run_passes` cannot drive it |
| Image | SY1SY2 has no loader header; the flat SRAM image comes from `tools/ns4_dsp_raw.py --sram`, the boot kernel needs 171,826 steps to its `IDLE` | exists (`sharc-ref`) |
| Messages | the 290 SY words are **path 1** `(word1, word2)` = `[opcode:4 \| address:28], value` into the command FIFO ring (`0x93046`) plus the DMA write index `DM(0x2400)`, not stores into a DM word as `M W1 W2` does | new protocol command |
| Sample refill | the external-port DMA that refills the 128-frame source ring is a model of its own (D137, PR #496) | not on this branch |

The smallest working subset is therefore ORG core 2 and PNO core 2 (this PR). The SY chip is the next session: `sharc-live --sy` with `F` (FIFO message), `I VECTOR` (interrupt) commands on top of `sharc-ref`'s harness, ~two interrupts per pass, boot-kernel start from the ARM's own image download (`0xC0241C58`). The `Router` already counts path 1 per chip and is the place to route them.

## 5. Implications

- **The transport scales to several chips for free**: the preset is data, the second process is another `--chip`. One run costs 2 x 1.2 ms x 2,441 passes of interpretation plus the ARM boot (119 s wall).
- **The recall stream does not change when the chips run live**, because nothing flows back (D138). Everything D132 could not see in the recall stream is still not in it; the missing words are not an "ARM waits for the DSP" artefact visible with this link.
- **The envelope question moves**: the chain states are program specific here, so D132's "not comparable, image default" row should be re-read before the envelope part of the e2e comparison is set aside. This PR does not re-run D132.
- **The key goes only to PNO core 2** (live confirmation); a comparison against `SynthLayer` needs the PNO core 2 voice, not the ORG core 2 one.

## 6. Follow-ups

1. SY live chip (section 4): interrupt driven pass model, path 1 FIFO delivery, boot from the ARM image download.
2. Re-check D132 s1 "Envelope" with the live traces: tap every pass for all five programs under the live timing and under D132 timing, and say which word makes the program difference.
3. Key duration: the live hold is 1,500 ticks (62.5 ms of timer time); D132 held 0.5 s. A 12,000 tick hold gives the steady state needed for the level comparison against `synth_e2e_native`.
4. Reply direction (D138 follow-up 2) is still absent; SY's voice-ready bit (`DspAck`) is the first candidate.
5. DM word window: 5,910 PNO core 1 stores lie outside the 1 M word window (D138); ORG core 2 has 0 skipped.

## Reproduce

```bash
cargo build --release -p nord-sharc --bin sharc-live --bin sharc-sys
uv run python tools/ns4_dsp_ldr.py target/DSP/dsp_image_2_ORG.bin --sram ORG        # core2_sram.bin
C=target/fw_unpacked; R=target/release; W=0xC1F89:4:2; K=0xC1174,0xC1180,0xC118E
uv run --with unicorn --with capstone python -u tools/arm_dsp_live.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
  --eeprom target/oled/eeprom.bin --program "factory_programs/Program/Bank A/Smooth Lead.ns4p" \
  --chip "ORG:2=org=ORG/core2_sram.bin=$W=0x12566E=$K" \
  --chip "PNO:2=org=target/PNO/core2_sram.bin=$W=0x12566E=$K" \
  --live $R/sharc-live --sys-exe $R/sharc-sys --old-replay ORG2,PNO2 --log live --out live.json   # about 130 s
uv run pytest tools/tests/test_arm_dsp_route.py tools/tests/test_arm_dsp_live.py
cargo test -p nord-sharc --lib runner::live
```
