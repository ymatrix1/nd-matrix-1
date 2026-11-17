# Research: the real ARM timer (24 kHz), the periodic tasks and the engine constants (D130)

- **Checked:** 2026-11-17
- **Sources (local, never committed):** OS v1.66 `target/fw_unpacked/bank0_CODE.bin` and `bank0_SRAM.bin` run in the ARM harness (`unicorn`, `arm_boot_full.prepare` + `arm_steady.boot_to_main_loop`), the factory restore EEPROM image `target/oled/eeprom.bin`, `factory_programs/Program/Bank A/Sidechain Whl II.ns4p`. Tool: `tools/arm_tick_probe.py` (new, tests `tools/tests/test_arm_tick_probe.py`). Data (addresses, periods, counts only): [arm-tick-timers.json](data/arm-tick-timers.json), [arm-tick-keydelay.json](data/arm-tick-keydelay.json), [arm-tick-fxwatch.json](data/arm-tick-fxwatch.json), [arm-tick-flagscan.json](data/arm-tick-flagscan.json). Background: [synth-arp-patterns](synth-arp-patterns.md) s9, [arm-organ-messages](arm-organ-messages.md) s10, [fx-leslie](fx-leslie.md) s11, [synth-super-lp24-clips](synth-super-lp24-clips.md) s3b, [fx-mod-types](fx-mod-types.md) s7, [dsp-frame-rate](dsp-frame-rate.md), [arm-dsp-capture](arm-dsp-capture.md) (D124).
- **Question:** which hardware timers does the firmware program, at what rate does each run (the main tick: 1 kHz or 24 kHz?), which periodic tasks hang off it with what period and what do they write to the DSPs, and which engine constants depend on the answer? Also: where does the "PUMP 2.11 Hz against 6.1 Hz free-run" figure come from, and is the arpeggiator gate (OQ32) 2/3 or 1/2 of a step?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by the command in "Reproduce", **EMPIRICAL** = measured in this harness, **ASSUMED** = reading or placeholder.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Main tick | **24 kHz**, not 1 kHz. Timer0 (`0x01C20000`) in dual 32-bit unchained mode; `PRD34 = 0x3E8 = 1000` (written at `0xC0050298`), `PRD12 = 0xFA = 250` (`0xC0069518`); both halves run continuously. `24,000,000 / 1000` = **24 kHz** on TIM34 (system interrupt 22, handler `0x800007D8`: `tick64 += 1`, context switch, ring drain) and `24,000,000 / 250` = 96 kHz on TIM12 (no interrupt attached; its counter is read 3 times in the whole boot). The set-up routine `0xC005019C(dev, half, hz)` is called with `hz = 0x5DC0 = 24000` | VERIFIED (run: `arm_tick_probe.py timers`, 24 MHz input clock = the firmware constant `0x016E3600`; the board clock itself ASSUMED, section 5) |
| Is anything 1 kHz? | **No periodic task of the host firmware runs at 1 kHz.** The scheduler has 17 periodic entries at 1, 4, 8, 10, 100 and 200 Hz (section 3) and the arpeggiator clock at 1,500 Hz. The "1 ms" of `arm_steady.py`, `gain_dezip.rs`, `vox_timeline.rs` was the harness label of 200,000 instructions, not a hardware period | VERIFIED (table below) |
| Key-down delays (Vox slots 3 / 17 / 47 "ticks") | **timer driven**: with the interrupt injected every 200,000 / 100,000 / 50,000 / 25,000 instructions the Vox key-down message groups arrive at timer interrupt 3, 17 and 47 every time. A timer interrupt is **41.67 us**, so the delays are **0.125, 0.71 and 1.96 ms**, not 3 / 17 / 47 ms (lower bounds: the ARM must also have done the work in 12,500 cycles per tick, section 5) | VERIFIED (run: `arm_tick_probe.py keydelay`) for the tick counts, ASSUMED for real time |
| Rates in the harness | Effective rates are lower than the nominal periods: the scheduler looks at one list entry per 6 kHz wheel call, so a 5 ms entry runs at 166 Hz (6.0 ms) and the 10 ms Leslie rotor update at 84 Hz (11.9 ms) | EMPIRICAL (harness, main loop always keeps up) |
| ARM to DSP words of periodic tasks | In steady state with an idle program only the **Leslie rotor update `0xC02090F4`** writes: 3 words per call (PNO core 1 DM `0xB2F25`, `0xB2F26`, `0xB2FA1`, the "3 words every 28 M instructions" of [arm-organ-messages](arm-organ-messages.md) s10). None of the other 16 entries or the arpeggiator clock writes | VERIFIED (run, 12,000 ticks) |
| PUMP "2.11 Hz against 6.1 Hz" | Source recorded: [synth-super-lp24-clips](synth-super-lp24-clips.md) s3b, A:25 Sidechain Whl II, 28 s clip, strongest 0.5 to 8 Hz envelope modulation: hardware **2.11 Hz** (126 BPM = 2.10 Hz, one reset per beat), `--firmware-fx` render **6.11 Hz** (free-running). The free-run figure is **reproduced from the firmware laws**: `pump_free_run_hz(73)` = 6.108 Hz (section 4) | VERIFIED (free-run, unit test), EMPIRICAL (hardware figure, one clip) |
| Does the ARM drive the PUMP counter? | **Not by a periodic FX message**: loading A:25 and running 30,000 timer interrupts (1.25 s, 2.6 beats at 126 BPM) sends **0 FX DSP words** of any kind. The counter `c[-0x15]` is therefore not written by the ARM through the message ports; how it advances (DSP side from a Master Clock word, another DSP, not running in the harness) stays open | VERIFIED (negative in this harness), open (OQ36) |
| Arpeggiator gate (OQ32) | **2/3 of a step** in the firmware's boot state: the pattern object constructor `0xC00FD7B8` stores `[obj+0xC] = 1`, the Glue update `0xC011A51C` then selects the grid `0xC02BC1A0` (grid points `0, 0x55555` per `0x80000` = 2/3 beat; the straight grid of flag 0 has `0, 0x40000`); no store to that flag was found in the pattern code. It does not depend on the tick (phase based). Pattern off compiles all `E`, so the plain arpeggio is 2/3 too | VERIFIED (disassembly + `ns4_arp_player.py gates` replay, section 6), the meaning of the flag (shuffle) ASSUMED, hardware recording still wanted |

## 2. Timers and interrupts (`arm_tick_probe.py timers`)

Every store to Timer0 during the boot (24 writes, in the data file) reduces to:

| Register | Offset | Final value | Meaning | Written by |
|---|---|---|---|---|
| TGCR | `0x24` | `0x7` | TIMMODE = dual 32-bit unchained, both halves released | `0xC0050208`, `0xC00694xx` |
| TCR | `0x20` | `0x800080` | ENAMODE12 = ENAMODE34 = continuous | `0xC00636F0`, `0xC002422C`, `0xC002426C` |
| PRD12 | `0x18` | `0xFA` = 250 | 96 kHz free-running counter, no interrupt | `0xC0069518` (timer object ctor `0xC00693xx`, default frequency `0x17700` = 96,000) |
| PRD34 | `0x1C` | `0x3E8` = 1000 | **24 kHz interrupt** | `0xC0050298` (set-up `0xC005019C`, called from `0xC00699C0` and `0xC0069B54` with `0x5DC0`) |

Interrupt attach (`0xC005012C(dev, handler, channel)`, AINTC system interrupt `[dev + 4]` = 22) is called twice: first `0x80000410` (bare `tick64 += 1`), then `0x800007D8` (tick plus the context-3 ring drain). Both attach to channel 4. The period is `PRD / 24 MHz` and the firmware divides `0x016E3600` = 24,000,000 by the frequency it is given (`0xC0050244..0xC0050260`).

The earlier text (`arm_steady.py`, [arm-organ-messages](arm-organ-messages.md) s10.1: "1 kHz timer, 24 MHz / 0x5DC0 = 1 ms") read `24 MHz / 0x5DC0` as a period; the routine's argument is a frequency. [synth-arp-patterns](synth-arp-patterns.md) s9.3 had the same reading from the code; this run adds the register values.

## 3. Periodic tasks (`arm_tick_probe.py wheel` and `run`)

Scheduler (VERIFIED, run): the main loop compares `tick64` with its last value and, when it changed, calls the **tick wheel** `0xC0052AA4`: 512 slots (`wheel + 0x30`), one slot function per timer interrupt. 320 slots are empty (`0x800002A8`, `bx lr`), 128 call the **periodic list** (thunk `0xC009B8A8` -> SRAM `0x8000030C`, every 4th slot = **6,000 per s**) and 64 the **one-shot stack** (thunk `0xC009B864` -> `0x800002AC`, every 8th slot = **3,000 per s**). The periodic list holds 17 entries of `0x28` bytes `{id, period (ticks of 24 kHz), due, object, function, mode}`; each call of the list examines **one** entry in rotation (cursor `[list + 0x3F0]`), runs it when `due <= tick64` and sets `due = now + period` (mode 1). So an entry is seen every 17 x 4 = 68 ticks (2.83 ms) at best; the effective period is the nominal one plus up to 68 ticks.

| Task | Function, object | Nominal | Hz nominal | Hz measured in the harness | ARM to DSP words | Source |
|---|---|---|---|---|---|---|
| Arpeggiator clock (6 clock objects, `0xC054CC10`) | `0xC01E6368`, `0x90032B80` | every 16 ticks, called from the tail of the timer ISR (return `0x80000A38`) | 1,500 | **1,500.0** (750 calls in 12,000 ticks) | none; it notifies the pattern player `0xC023B3EC` (the player did not run: no arp program) | VERIFIED (run) |
| Entry 8 (unidentified) | `0xC009F13C`, `0x90039D30` | 120 ticks = 5 ms | 200 | 166 | none seen (idle) | VERIFIED nominal, EMPIRICAL measured |
| Leslie rotor update (entry 19) | `0xC02090F4`, `0x9115D2B0` | 240 ticks = 10 ms | 100 | **84** (42 in 0.5 s) | **3 per call**: PNO core 1 DM `0xB2F25`, `0xB2F26`, `0xB2FA1` | VERIFIED (run); [fx-leslie](fx-leslie.md) s11 |
| entry 9 | `0xC023D098`, `0x90DBF710` | 2,400 = 100 ms | 10 | 10 | none seen | VERIFIED nominal |
| entries 10 to 16 (7 objects) | `0xC01731EC` | 3,000 = 125 ms | 8 each | 8 | none seen | VERIFIED nominal |
| entry 18 | `0xC01AC91C`, object 0 | 3,000 = 125 ms | 8 | 8 | none seen | VERIFIED nominal |
| entries 4 to 7 (4 objects) | `0xC0096EB8` | 6,000 = 250 ms | 4 each | 4 | none seen | VERIFIED nominal |
| entries 2, 3 | `0xC004C8AC` | 24,000 = 1 s | 1 each | 1 | none seen | VERIFIED nominal |
| One-shot stack | 28 pending events at the main loop, 5 later | 3 kHz look at the top | n/a | n/a | n/a | VERIFIED (count) |

What the tasks are named for is **not** known beyond the Leslie update (`0xC02090F4`, [fx-leslie](fx-leslie.md)); the others write nothing to the DSPs in the idle harness, so no message id can be assigned (the 384-program captures of [arm-dsp-capture](arm-dsp-capture.md) show where the event-driven words go: key phases only for SY, the recall for the strips).

**There is no ARM-side LFO or envelope update.** No periodic entry writes strip, LFO or envelope words in this state, and the SY capture of D124 sends its per-voice words once per note (17 words), not periodically within 40 ticks (1.7 ms of timer time, a short window). The synth envelope pass and the LFO run on the SY / ORG frames (43,402.34 Hz, [dsp-frame-rate](dsp-frame-rate.md)); the ARM tick does not enter them.

## 4. PUMP: the 2.11 Hz and 6.1 Hz figures

`pump_free_run_hz(v)` in `tools/arm_tick_probe.py` evaluates the verified law (`c[-32]` phase increment per call at the 11 sample values `v`, [fx-mod-types](fx-mod-types.md) s7, `fx/fw_mod1_data.rs` `RATE_10`) and the routine's cycle: the phase runs from `0x80000000` to the cap `c[-22]` = `0x7FFFFFFF` (type 10) at `inc` per call, one call per two frames (the piano unit, [fx-mod-types](fx-mod-types.md) s7):

| Rate value `v` | `inc` (per call) | Calls per cycle | Cycle rate at `F / 2` = 21,701 calls per s |
|---|---|---|---|
| 73 (0.575 of 127; the A:25 rate 0.58) | 1,208,832 | 3,553 | **6.108 Hz** |
| 74 | 1,225,216 | 3,505 | 6.19 Hz |

The `--firmware-fx` render of A:25 measured **6.11 Hz** ([synth-super-lp24-clips](synth-super-lp24-clips.md) s3b); the formula gives 6.108 Hz, so the free-run side of the "6.1 Hz" is explained by the verified law with no other term. The hardware figure, **2.11 Hz** from the same measure on the 28 s hardware clip, equals the Master Clock beat (126 BPM = 2.10 Hz): the routine resets the phase to `0x80000000` whenever the counter `c[-0x15]` differs from its copy `c[-0x14]`, and the free-run cycle (0.164 s) is shorter than a beat (0.476 s), so the hardware pumps once per beat and the free-run render pumps 2.9 times per beat. The two numbers were a coordinating-session figure; the source is the clip analysis above (one clip, FFT bin resolution of the 28 s window about 0.04 Hz), now recorded here and in [audio-remaining](audio-remaining.md) row 17.

**What drives the counter is open.** `arm_tick_probe.py fxwatch` loads A:25 (Master Clock on, 126 BPM, PUMP on in all three FX chains per the clips note) and runs 30,000 timer interrupts: **zero FX DSP words**, periodic or not. The counter is not written through the ARM message ports in this harness. Candidates (ASSUMED, not tested): the DSP reads a Master Clock word the ARM stored earlier and counts beats itself; another DSP writes it; the Master Clock does not run in the harness (the ARM's Master Clock object `[clock + 4]` = 120 BPM at boot, [fx-delay-tempo](fx-delay-tempo.md)). Next: find the DSP side writer of `c[-0x15]` (SHARC FX image) and the ARM object that holds the beat count.

## 5. Engine constants against the measured timer

| Engine constant | File | Value | Real | Divergence | What depends on it |
|---|---|---|---|---|---|
| `CONTROL_TICK_HZ` (ARM writes gain targets at a timer tick) | `synth/gain_dezip.rs` | 1,000 Hz, ASSUMED | no 1 kHz task; the synth voice words are event driven (once per note-on in the D124 capture); timer 24 kHz | 24 x the tick, the 1 kHz update itself has no firmware counterpart found | opt-in only (`gain_dezipper` off, FirmwareStrip chain on): the gain target is latched every 1 ms (48 samples at 48 kHz) instead of when the ARM writes it; the pole `0.979` per SY frame is VERIFIED and unchanged |
| `TICK_SECONDS` (Vox key timeline) | `organ/vox_timeline.rs` | 0.001 s per tick, ASSUMED | 1/24,000 s = 41.67 us per timer interrupt; the delays are timer driven | slot delays 3 / 17 / 47 ticks (key-down), 1 / 7 / 37 (key-up): **0.125 / 0.71 / 1.96 ms and 0.04 / 0.29 / 1.54 ms** against 3 / 17 / 47 ms and 1 / 7 / 37 ms (24 x too slow) | the Vox weight steps on the firmware path (`render_vox_frame`): the 2' slot starts 45 ms late and the 8' / 4' slots 16 ms late relative to hardware, if the ARM keeps up |
| `UPDATE_TICKS` / `TICK_HZ` (Leslie rotor) | `organ/rotor_arm.rs` | 240 ticks at 24,000 Hz = 10 ms, VERIFIED | same (nominal) | none nominal; effective update period in the harness 11.9 ms (84 Hz), the ramps would be 16 % slower (EMPIRICAL) | already on the 24 kHz base |
| Arp step `30 / D` s | `synth/arp_rate.rs` | derived from the 24 kHz clock (D66) | same | none | already on the 24 kHz base |
| Arp gate `E` = 4/8 of a step, plain arpeggio held to the next step | `synth/` pattern code | ASSUMED | 2/3 of a step, staccato (section 6) | **E 0.500 against 0.667 of a step**; plain arpeggio 1.0 against 0.667 | OQ32, opt-in change left to a follow-up |
| Strip pass 2 SY frames | `synth/strip_map.rs` | 2 frames, VERIFIED (index law) | DSP frame based | none (not a tick quantity) | envelope stage rates and the LFO are DSP frame based and do **not** depend on the ARM tick |
| Harness `INSNS_PER_TICK` | `tools/arm_steady.py` | 200,000 instructions per interrupt, "1 ms" | 12,500 cycles per interrupt at 300 MHz (ASSUMED CPU clock, `PLL0` is programmed by the bootloader) | the harness runs **16 x** more ARM instructions per timer interrupt than the hardware could | any work gated by instructions (parsing, display, queue drains) finishes in fewer interrupts in the harness than on hardware; timer-gated work (above) is right in ticks |

**Audible effect (ASSUMED, not measured against hardware):** the Vox key-down timeline is the only non-opt-in path that moves: the firmware's slot weights arrive within 2 ms instead of 47 ms, so the Vox attack gets its 2' content 45 ms earlier (a faster, brighter edge). That assumes the ARM completes the key-down work inside the timer delays, which at 12,500 cycles per tick for the 3-tick first group is not certain: the real delay is at least the timer count, possibly longer. No hardware Vox attack recording was compared. The gain de-zipper moves by under 1 ms, inaudible. Envelope and LFO rates do not change. The arpeggiator gate moves the note length by a sixth of a step (2/3 against 1/2), audible as a longer staccato.

**Superseded by D131:** the owner approved the flip, `TICK_SECONDS` and `CONTROL_TICK_HZ` use the measured tick by default (`--legacy-1khz-tick` restores 1 ms); arp gate and PUMP unchanged. The text below is the D130 state: **No engine change in D130** (D22, the effect of the Vox delays is ASSUMED); the constants are documented with this note and tagged in their source comments. A later opt-in could carry `TICK_SECONDS = 1 / 24,000` once a hardware Vox clip or a measurement of the ARM's key-down work time exists.

## 6. Arpeggiator gate (OQ32)

Path: constructor `0xC00FD7B8` builds the six pattern objects (`0xE0` bytes, vtable `0xC02B79FC`) and stores `[obj + 0xC] = 1` (`0xC00FD814`), `[obj + 8] = 4`, `[obj + 0x10]` = a grid pointer. The Glue update `0xC011A51C` (free-run branch, `[glue + 0xCC] != 1`) tests `[obj + 0xC]`: 0 selects the grid `0xC02BBF98` (straight), non-zero the grid `0xC02BC1A0`; in the Master Clock branch it picks the table entry at `+0x410` (flag 0) or `+0x434` (flag 1). The player `0xC023B3EC` sends a note-off at the second grid point of a step.

Measured with the real compiler and player (`python tools/ns4_arp_player.py CODE gates`, run again here): free-run grid `0, 0x40000, 0x80000, 0xC0000, ...` (half a beat) and **boot grid `0, 0x55555, 0x80000, 0xD5555, ...`** (`0x55555 / 0x80000` = **2/3**); "player events == one record per grid point, all patterns". The pattern objects are created with flag 1, so the boot state uses the second grid and an `E` step ends at 2/3 of the step.

Writers of the flag: `arm_tick_probe.py flagscan` lists every `str rX, [rN, #0xC]` (excluding the stack) in the three arpeggiator code ranges: 37 sites (22 in `0xC023B000..0xC0243000`, 8 in the Glue code, 7 in the constructor area). The one in the pattern constructor is `0xC00FD814`; the others store other objects' `+0xC` fields (the compiler writes the 16-byte record `{type, tick, pan, acc}`: `0xC0242514`, `0xC02425C4` are `acc`); a data-flow trace of each site was **not** done, so the scan is a candidate list, not a proof. The stronger evidence is the earlier run ([synth-arp-patterns](synth-arp-patterns.md) s9.5: a write hook over the edit ids `0x10B..0x114` and the replayed boot found no store) plus this one: nothing in the 12,000-tick idle run or the A:25 load changes the grid.

**Answer: 2/3 of a step for `E` and for the plain arpeggio, VERIFIED in the replayed boot state, the meaning of the flag (a shuffle / triplet feel) ASSUMED, no hardware recording.** The tick measurement is not needed for this (grid points are phases of the 2^21 clock, not interrupts). Unresolved: whether the hardware reaches the same state (the ARM could set the flag from a UI path the harness does not run), settled only by a hardware recording of an arp note against its step.

## 7. Implications

- **The ARM timer is 24 kHz.** Anything in the repo that calls a harness tick "1 ms" is wrong by 24: the 41.67 us interrupt, 200,000 instructions per interrupt (16 x the hardware budget at 300 MHz). Notes that count "ticks" ([org-system-run](org-system-run.md), [keydown-paths](keydown-paths.md), [arm-dsp-capture](arm-dsp-capture.md)) count interrupts; rates between events in those notes are in 41.67 us units.
- **Vox key timeline** (the only non-opt-in divergence found): delays 24 x too long (section 5).
- **Envelope pass and LFO: no tick dependence.** Row 18 of [audio-remaining](audio-remaining.md) reduces to: the 2-frame pass stays ASSUMED / VERIFIED by the index law, the ARM tick is no input.
- **PUMP**: the free-run rate is explained; the hardware beat reset needs the counter writer (OQ36).
- **Arp gate**: 2/3 (OQ32) pending a hardware check.

## 8. Follow-ups

1. Opt-in `TICK_SECONDS = 1/24,000` for the Vox timeline once a Vox attack clip or the ARM work time is measured (a 12,500-cycle budget per interrupt: measure the instructions the ARM needs for the key-down handler with the real `ARM926` timing, not the harness count).
2. OQ36: find the writer of the PUMP counter `c[-0x15]` (DSP side or a Master Clock word) and drive it from the engine's Master Clock; one PUMP clip (A:25) as the check.
3. Opt-in arp gate 2/3 and staccato plain arpeggio (stored as eighths: GUI dial, host parameters, pattern words) behind a flag; hardware clip of an arp note against its step.
4. Re-label the harness: `arm_steady.INSNS_PER_TICK` comments and the "1 ms" docstrings (done for `arm_steady.py` here); decide whether `INSNS_PER_TICK` should become 12,500 so that instruction-gated work matches the 300 MHz hardware (changes every capture; not done).
5. Identify the other periodic tasks (entries 2 to 7, 9, 10 to 16, 18) by the objects they service; none wrote DSP words in an idle run, so try a sounding program.

## Reproduce

```bash
C=target/fw_unpacked        # firmware, never committed
uv run --with unicorn --with capstone python tools/arm_tick_probe.py all $C/bank0_CODE.bin $C/bank0_SRAM.bin \
  --ticks 12000 --out docs/research/data/arm-tick-timers.json          # about 4 min
uv run --with unicorn --with capstone python tools/arm_tick_probe.py keydelay $C/bank0_CODE.bin $C/bank0_SRAM.bin \
  --cache target/keydown.snap --model 1 --out docs/research/data/arm-tick-keydelay.json
uv run --with unicorn --with capstone python tools/arm_tick_probe.py fxwatch $C/bank0_CODE.bin $C/bank0_SRAM.bin \
  --eeprom target/oled/eeprom.bin --program "factory_programs/Program/Bank A/Sidechain Whl II.ns4p" \
  --ticks 30000 --settle 1000 --out docs/research/data/arm-tick-fxwatch.json
uv run python tools/arm_tick_probe.py flagscan $C/bank0_CODE.bin x > docs/research/data/arm-tick-flagscan.json
uv run --with unicorn --with capstone python tools/ns4_arp_player.py $C/bank0_CODE.bin gates
uv run pytest tools/tests/test_arm_tick_probe.py
```

`pump_free_run_hz(73)` prints 6.108 (unit test `test_pump_free_run_reproduces_6_11_hz_and_the_beat_is_2_1_hz`).
