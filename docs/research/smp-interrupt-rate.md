# SMP interrupt rate, the second voice, the SY1SY2 FIFO bias and the tap level per class (OQ41, D211)

- **Checked:** 2026-11-21
- **Sources (local, never committed):** OS v1.66 `target/fw_unpacked/bank0_CODE.bin`, `target/oracle/smp.bin`, `target/oracle/sy.bin` and `sy.lst`, factory backup `Nord Stage 4 Factory Restore RevK v1.66.ns4b`, runs of `tools/arm_dsp_live.py --piano-bind` (Astoria Grand 3,000 and 8,000 key ticks, Felt Upright, Classic Wah Clav, Bubbles Mk I at 3,000) and of the SY chip (Mm-Oh Choir Whl, 1,500). Code: `tools/smp_tap_report.py`, `tools/smp_voice_replay.py`, `tools/sy_fifo_probe.py` (new), `crates/nord-sharc/src/runner/live_sy.rs`, `bin/sharc_live.rs`. Earlier notes: [dsp-frame-rate](dsp-frame-rate.md), [smp-voice-pitch](smp-voice-pitch.md), [smp-page-refill](smp-page-refill.md), [piano-bus-convention](piano-bus-convention.md).
- **Question (OQ41, D205 follow-ups):** at which rate does the SMP chip take its audio interrupt (the tap sounds 261.6 Hz only at 43.4 to 43.5 kHz), does the second voice of a note-on run under pacing, does the SY1SY2 chip need the same FIFO fix, and how far is the tap level from the native render per program class?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by the commands in "Reproduce" or a unit test, **EMPIRICAL** = measured in this harness, **ASSUMED** = reading.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Interrupt rate of the SMP chip | **The DSP frame rate `F = 44,444,000 / 1024 = 43,402.34 Hz`**, the same as PNO, ORG, FX and SY. The SMP image does not program its audio clock (SPORT control word `0x020C21F0`, internal clock and frame sync bits clear), so the rate is the external codec word clock the ARM calls `F` in one global (`0xC01BF74C` pool, literal `0x47298A58`). Tap pitch at `F`: 261.72 Hz, +0.62 cent against C4; at 44.1 kHz +28.2 cent, at 48 kHz +174.9 cent | VERIFIED (ARM literal, `tools/smp_tap_report.py rate`, test `rate_report_candidates_and_the_rate_that_gives_the_note`); SPORT bit reading ASSUMED (layout from the ADSP-2148x HRM, not re-read here) |
| The "43.4 to 43.5 kHz" of D205 | It is `F` itself. The rate that sounds exactly C4 is 43,386.7 Hz (0.6 cent below `F`); the older readings (43.4 to 43.5 kHz) came from 4 digit cycle counts (0.00601 to 0.00604) | VERIFIED (arithmetic) |
| Step law base `0.80618` | `35,000 / F = 0.806413`; the ARM word is 0.80618, 0.50 cent lower. The 0.78 cent step of the ARM fine tune table (dsp-frame-rate s2) is larger than this gap; the origin of the 0.50 cent is not found. It does not move the rate: the whole pitch error stays under 1 cent | EMPIRICAL |
| Voices of one note-on | **Two voices start together** (Astoria Grand: stroke 513 at record `0x93928`, stroke 502 at `0x9395E`, same step word `0xCC5D4800`), a third (stroke 523, record `0x93CBE` = voice 17) starts at **key off**. With `--fifo-pace 1` the history rings of voice 0 and voice 1 equal the reference decode bit for bit | VERIFIED (`tools/smp_voice_replay.py --voice 0 --stroke 513`, `--voice 1 --stroke 502`: MATCH both) |
| Third voice (stroke 523) | **Not decoded by the replay tool**: the page header of stroke 523 has predictor order 4, the tool (and the documented `0xE3A0C` rows) know orders 0 to 3. Blocker: order 4 row of the coefficient table | EMPIRICAL (negative) |
| SY1SY2 under pacing | The SY ISR has the same FIFO consumer: `R3 = 0x80000` at the probe in every interrupt (256 of 256 snapshots), so the SY model storing the plain write address (`index_bias 0`) replays the ring like the SMP model did before D199. New opt-in `sharc-live --index-bias ADDR` and chip window token `paced` (`--fifo-pace 1 --index-bias 0x80000`); defaults unchanged | VERIFIED (run, `tools/sy_fifo_probe.py`) |
| Tap level against the native render | Raw tap RMS is 6.9 to 10.7 dB below the native program render (4 programs, 69 ms from the onset); with the receiver scale `0.3732` the gap is 15.4 to 19.3 dB. Per class: grand -10.1 dB, upright -9.3 dB, clavinet -6.9 dB, electric -10.7 dB (one program each in this run; the 12 program pilot spreads -5.6 to -13.3 dB). **Not a parity number** (s5) | EMPIRICAL |

## 2. Why `F` is the SMP interrupt rate

| Evidence | Reading | Tag |
|---|---|---|
| The ARM holds one frame rate for every DSP: literal `0x47298A58` = 43,402.34375 Hz at pool `0xC01BF74C`, loaded at `0xC01BF43C` into a global; the delay words, the SY fade words, the Pipe bank, the B3 generator and the Vox word agree to 0.0001 cent ([dsp-frame-rate](dsp-frame-rate.md) s3) | The SMP is one of the five DSP images fed by the same codec clock | VERIFIED |
| SMP init (`0x1C11D1..0x1C11DD`, [piano-bus-convention](piano-bus-convention.md) s2) writes `DM(0x0C00) = 0x020C21F0`: word length 32, `FSR` set, the internal-clock (`ICLK`, bit 10) and internal-frame-sync (`IFS`, bit 14) bits clear; no divider word is written for a clock of its own | The serial port is a slave of the board clock, exactly like the SPORTs of PNO, ORG and FX (`ICLK = 0`, `IFS = 0`, dsp-frame-rate s4). The SMP cannot choose 44.1 or 48 kHz | ASSUMED (bit layout from the HRM) |
| The ring is the transmit buffer of that port and the FX receives it (SPORT3B, [piano-receiver-scale](piano-receiver-scale.md)); the FX stages run per frame at `F` ([dsp-frame-rate](dsp-frame-rate.md), D180) | One SMP frame is one FX frame; the SMP ISR is the SPORT frame interrupt of the same clock | EMPIRICAL |
| `tools/arm_dsp_link.py` already runs the DSP pass clock as `2 / F` since D138 | The harness clock and the tap rate agree | VERIFIED (code) |
| Pitch of the tap (Astoria Grand, 3,000 key ticks, left lane from the loudest sample, 8,192 frames): `0.00603008` cycles per frame; reference decode of stroke 513 `0.00755095` cycles per decoded sample; step `0xCC5D4800 / 2^32 = 0.79829836` | Ratio tap over `reference * step` = 1.00036 (0.62 cent): the voice resamples the decode at the step word. Times `F`: 261.72 Hz, +0.62 cent against C4 | VERIFIED (run, `smp_tap_report.py rate`) |
| The stroke itself sits at 264.28 Hz at its 35,000 Hz rate; the stroke tuning is -17 cent, so `264.28 * 2^(-17/1200) = 261.70 Hz` is **the pitch the ARM aims at when the voice runs at `F`** (+0.50 cent against C4) | The ARM step law aims at `F`, not 44.1 or 48 kHz: at 48 kHz the note would be 289.4 Hz | VERIFIED (arithmetic) |

Every candidate other than `F` misses by more than the 0.6 cent of the measurement: 44.1 kHz by +28.2 cent, 48 kHz by +174.9 cent. The D205 table of Hz at 48 kHz (288.6 to 290.0) must be read at `F`: 260.9 to 262.2 Hz, inside the 0.5 % spread of the 4 digit cycle counts.

## 3. Second and third voice

`SMP0.script` of the Astoria Grand run (3,000 and 8,000 key ticks): at pass 814 the ARM writes **two** voice starts (requests `0x9251A = 0x93928` and `0x92526 = 0x9395E`, histories `0xC0500` and `0xC0510`, level words `0x3F800000` / `0x3F72EA88` for both), and `smp_flash` detects descriptors for stroke 513 and, `0x36` words later, stroke 502. A third start (`0x92526 = 0x93CBE`, history `0xC0610`, stroke 523, 46,359 frames) comes at pass 3571 in the 3,000 tick run and pass 8092 in the 8,000 tick run: the **note-off**, a release sample.

| Voice | Record | Stroke | Result under `--fifo-pace 1` | Tag |
|---|---|---|---|---|
| 0 | `0x93928` | 513 | history ring = reference decode at counter 381 | VERIFIED |
| 1 | `0x9395E` | 502 | history ring = reference decode (counter 379, ring one sample ahead of the counter word, `match_counter`) | VERIFIED |
| 17 (release) | `0x93CBE` | 523 | not replayed: page header order 4 unknown to the tool | open |

So the "layers 2 and 3" of the tap are voices of the same file in the same flash image (`rec11` 28271 in all three descriptors); no second `.npno` is needed for them. The D205 tap sum already contains voice 1; the release voice is silent in the tap window of a held note. The second piano layer of a program (`layer_b`, 36 of 384 programs) is a different problem: it needs its own sound object on another core (candidates `0x90C0CAC0`, `0x90C24180`, `0x90C3B840`, stride `0x176C0` after core 0 `0x90BF5400`) and a second flash image (`L` loads one file). Not done.

## 4. SY1SY2 under pacing

The SY listing sets `b0 = 0x93046`, `l0 = 0x280`, `r2 = 0x280`, `r3 = 0x80000` in one block (`sy.lst` `0xA9B..0xAAA`) and its ISR computes `n = (DM(0x2400) + R3 - I0) >> 1` capped by `DM(0x92180)` (`0x317..0x32A`), the SMP code of D199. Replay of the `SY0.script` of Mm-Oh Choir Whl (317 pairs, 2,300 passes, `--fifo-pace 1`, probe `0x1C063F`):

| Model | Probe hits (consumer entries) after the first message | Pairs delivered | Tag |
|---|---|---|---|
| plain index (`index_bias 0`, as in D153 to D191) | 294,336 | 317 | VERIFIED (run) |
| `--index-bias 0x80000` | 316 | 317 | VERIFIED (run) |

`R3` is `0x80000` in all 256 snapshots of the first probe run. With the plain index the probe at the consumer entry is reached about 128 times per pass (2 interrupts per pass) instead of once per delivered pair: the unbiased model keeps the consumer busy in every interrupt, the signature of the stale replay of D199 (what the consumer does with the extra entries is not traced here). The SY census (D153 onward) therefore ran on the replaying model. A full ARM run of the same program with `SY:0=sy=IMAGE=epchain+paced` (flags `--sy --epchain --fifo-pace 1 --index-bias 0x80000`) ends like the default run: 2,441 passes, 317 messages, no error, no halt (EMPIRICAL; no audio statistic is captured without `--return-path`). The opt-in flags change nothing by default; the SY census was **not** rerun (about 3 min of CPU per program, and `sy_audio_peak` needs `--return-path`).

## 5. Tap level against the native render, per class

`smp_tap_report.py level` (tap and native over 69 ms from each onset; native = `ns4 render-patch` of the whole program, note 60 velocity 100):

| Class | Program | Tap RMS dB | Native RMS dB | Tap minus native | With receiver scale 0.3732 |
|---|---|---|---|---|---|
| Grand | Astoria Grand | -26.5 | -16.4 | -10.1 | -18.7 |
| Upright | Felt Upright | -24.1 | -14.7 | -9.3 | -17.9 |
| Clavinet | Classic Wah Clav | -31.9 | -25.1 | -6.9 | -15.4 |
| Electric | Bubbles Mk I | -27.2 | -16.5 | -10.7 | -19.3 |

These four runs reproduce the pilot column of [smp-voice-pitch](smp-voice-pitch.md) (-10.0, -9.5, -6.9, -9.8) to 0.7 dB. The 12 program spread is -5.6 to -13.3 dB (median -9.9), so the offset is not one constant per class: Astoria Grand and C and G are the same sound at -10.0 and -9.1.

Why this is not a parity number: the native render has the layer chain (amp, EQ, compressor, reverb, layer B) that the tap precedes; the master level word `DM(0xB6518)` is ASSUMED 1.0 ([piano-engine](../specs/piano-engine.md)); the hardware has no recording of the bare SMP ring. A like for like native number needs a dry piano render (no amp, EQ, compressor, reverb, mod, delay), which exists only as the `piano_dry` hardware clip ([brightness-downstream-oracle](brightness-downstream-oracle.md)) and not as an `ns4 render-patch` option. **Blocker:** a native render of layer A before the chain.

## Reproduce

```bash
C=target/fw_unpacked; B=".reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b"
cargo build --release -p nord-sharc --bin sharc-live -p nord-tools --bin smp_stroke_dump -p nord-core --bin ns4
uv run --with unicorn --with capstone python tools/arm_dsp_live.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
    --eeprom target/oled/eeprom.bin --program "factory_programs/Program/Bank A/Astoria Grand.ns4p" \
    --live target/release/sharc-live --poll-advance 64 --chip "PNO:1=pno=target/PNO/core1_sram.bin" \
    --chip "SMP:0=smp=target/oracle/smp.bin" --piano-bind "$B" --key-ticks 3000 --log run --out run/out.json
target/release/smp_stroke_dump run/smp_flash.npno 513 stroke513.f32
python tools/smp_tap_report.py rate run/smp_tap.f32 stroke513.f32 --step 0.7982983589   # s2
target/release/ns4 render-patch "Program/Bank A/Astoria Grand.ns4p" --backup "$B" --note 60 --velocity 100 --frames 20000 --wav native.wav
python tools/smp_tap_report.py level run/smp_tap.f32 native.wav [--receiver-scale]        # s5
python tools/smp_voice_replay.py run target/oracle/smp.bin target/release/sharc-live --stroke 502 --voice 1 --passes 1100   # s3, MATCH
# SY: run with --chip "SY:0=sy=target/oracle/sy.bin=epchain" and a SAMPLE program, then
python tools/sy_fifo_probe.py run/SY0.script target/oracle/sy.bin target/release/sharc-live --passes 2300 --fifo-pace 1 --index-bias 0x80000
```

## Implications

- The Hz of every SMP tap number is `cycles per frame * 43,402.34`; `arm_smp_flash.tap_fundamental` no longer assumes 48 kHz. OQ41 loses the interrupt-rate item.
- The tap sums at least two voices (strokes 513 and 502 for Astoria Grand); a per-voice level split needs the voice level words (`0x3F72EA88` = 0.9487 for both here) and is not done.
- The SY1SY2 chip needs the index bias for a faithful FIFO; every earlier SY live result (D153 to D191) used the replaying model.

## Follow-ups

1. ~~Order 4 predictor row (release strokes)~~ done (D224, [piano-release-voice](piano-release-voice.md)): stroke 523 decodes with `(-1, 4, -6, 4)`, the voice 17 ring equals the reference; the release level in the tap is open.
2. Second piano layer: bind `layer_b` on another core with a second flash image (`L` with an id) and check the tap with both layers.
3. Dry native piano render (layer A, no chain) to turn the level table into a parity number; then the 155-program run of [smp-voice-pitch](smp-voice-pitch.md) follow-up 1.
4. Rerun the SY census with `epchain+paced` (needs `--return-path`) and compare `sy_audio_peak` per program.
5. Origin of the 0.50 cent in the step base (`0.80618` against `35,000 / F`).
