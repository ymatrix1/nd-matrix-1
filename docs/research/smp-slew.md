# SMP voice slew loop and stream edge handling

- **Checked:** 2026-11-14
- **Sources (local, never committed):** SMP image of OS v1.66 (`python tools/ns4_dsp_images.py target/fw_unpacked/bank0_CODE.bin --out target/oracle/img`, `python tools/ns4_dsp_raw.py target/oracle/img/dsp_image_4_SMP.bin --sram target/oracle/smp.bin`; listing `python tools/adi_dis.py target/oracle/smp.bin 0x380600 0x382B10`, CCES 2.12.1; pc = `0x1C0300` + listing offset); `sharc-ref` oracle (`target/release/sharc-ref`, scenarios from `tools/smp_slew_scn.py`); [smp-voice](smp-voice.md), [piano-dsp-path](piano-dsp-path.md), [piano-sound-load](piano-sound-load.md) s11, [piano-bus-convention](piano-bus-convention.md), [piano-receiver-scale](piano-receiver-scale.md).
- **Question:** What does the SMP do between the float fields `+11..+16` of a voice record and the Q31 gains `0 / +1` that the voice loop multiplies in (slew time behaviour), and what does it do at the edges of a sample stream (loop, end, history)?

Tags follow `.agents/rules/provenance-in-code.md`. "Oracle" is the `sharc-ref` core running the real SMP image; the SIMD register swap and the long-word SIMD access are ASSUMED forms of that core ([smp-voice](smp-voice.md) s7), the numbers below hold on it.

## 1. Result

| Item | Result | Tag |
|---|---|---|
| Where the slew runs | Loop `0x1C06B9..0x1C06E6` of the audio interrupt (`LCNTR = 0x39`, SIMD, `M6 = 0x36` = one record per pass, before the voice loops). One interrupt updates records `0..=56`, the next `57..=113` (3 interrupts in a row: 0..56, 57..113, 0..56), so **every record is updated every second audio interrupt** | VERIFIED (run, 114 records poked, fields compared after each of 4 interrupts) |
| Fields read and written | Inputs `-2, -1, +2, +3, +11 .. +18` (floats) and `+19, +20` (Q31 words); outputs `-2, +13, +14, +15, +16` (floats) and the gains `0, +1`. A probe that changes one field per record (52 records, 20 fields) shows that `-20, -19, +4, +5` and the old `0, +1` do not influence any output | VERIFIED (run) |
| Update law | See section 2: one-pole-like x lane `v = max(F11 + F15 F13 F17, 0)`, y lane `u = max(F12 + F16 F14 F18, 0)` with decaying state `F14`, ramp `w = max(clip(F-2 + F-1, F3), F2)`, gains `(fix31(w v) F19) >> 31` and `(fix31(w v) F20) >> 31` | VERIFIED: Rust model equals the oracle in 11 of 11 random records (all 7 outputs) and in 60 of 60 updates of a trajectory record (both gains), `piano/smp_slew.rs` |
| Arithmetic | 40-bit extended float: `MODE1 = 0x1001CFC` (listing `0x1C06AA`) leaves `RND32` clear and sets `TRUNCATE`: intermediates keep 32 mantissa bits, every operation truncates toward zero, a store drops 8 more bits. With 24-bit truncation (or round to nearest) the model misses by 1 ulp in most fields; with `piano/ext40.rs` it is bit exact | VERIFIED (run) |
| Time behaviour | Per update of one record (every 2 interrupts): `w` moves by `F-1` per update toward `F3` (ramp with ceiling, floor `F2`); the x state follows `v' = F11 + v F13 F17` with `F13` replaced by the y state `u` each update, and `u' = F12 + u F14 F18` with `F14' = F14 F18`, so the decay coefficient `F14 F18^t` falls geometrically and `u` falls super-exponentially to the target `F12`. Example trajectory (60 updates, `F-1 = 0.04`, rates `0.979`, `F11 = F12 = 0`): the left gain rises with the ramp to `1.77e8` (0.0825 of full scale) at update 4 and decays to 0 by update 60; table in the goldens of `smp_slew.rs` | VERIFIED (run) for the arithmetic, the physical meaning of the ramp / decay split is ASSUMED |
| What the ARM writes at note-on | Four-word groups per record (`piano-sound-load` s3): record `R = 0x92848 + 54 k`, words `R + 14 = 1.0`, `R + 12 = 0`, `R + 11 = 0`, `R + 16 = 0.9793 (0x3F7AB576)`; so `F14 = 1`, `F11 = F12 = 0`, `y_state = 0.9793`, identical for every velocity and note. The other fields (`F13, F15, F17, F18, F-2, F-1, F2, F3, F19, F20`) are set by other ARM writers (`0xC00B6030`, `0xC00B61AC` tables, `0xC00B64D8` init) that were not run | EMPIRICAL (message list, address arithmetic) |
| Gain word in the record | The level word `level_q23 / 2^23` of [piano-sound-load](piano-sound-load.md) s10 sits at DM `0x9252A` / `0x9251E`, not in the 54-word record; how it reaches `F19 / F20` or `F11` is not traced | open |

## 2. The loop as equations

Per record (x lane = word `f`, y lane = word `f + 1` of each field pair read as a SIMD pair):

```text
d_x = F13 * F17                       v  = max(F11 + F15 * d_x, 0)      -> F15
d_y = F14 * F18                       u  = max(F12 + F16 * d_y, 0)      -> F16, F13;   F14 <- d_y
w   = max(clip(F-2 + F-1, |F3|), F2)                                      -> F-2
q   = round_to_nearest(w * v * 2^31)  (saturating, 40-bit float product)
F0  = (q * F19) >> 31     F1 = (q * F20) >> 31                           (floor)
```

The loop copies the x product into the y lane with `r8 <-> s6`, so **both gains come from the x lane**; the y lane only produces `u`, which becomes the coefficient `F13` of the next x update. Not modelled: in one probe the oracle gain was one LSB above the model for products with a Q31 multiplier of exactly `0x40000000` / `0x7FFFFFFF` (17 of 104 values, ties of the integer multiply); with random multipliers the model is exact (142 of 142 gains, the oracle runs behind `CASES`). Saturation (`w v >= 1`, `q` clamps to `0x7FFFFFFF`) is implemented but the oracle capture of that case was lost in the NaN guard of the dump, so it is not golden.

## 3. Stream edge handling (VERIFIED runs, `tools/smp_slew_scn.py edges`)

The state machine subroutine at `0x1C0A89` (listing `0x789`) is called twice per interrupt (records `2 j` and `2 j + 1`, walking from record 0: records 0, 1 on one interrupt, 2, 3 on the next). Observed with `F+8 = 1`, `F-8 = 100`, `F-7 = 40`, `F-6 = 500`, `F-5 = 600`, `F-4 = 7`, `F+6 = 0.5`, `F+7 = 0.25`, `F-24 = 3`:

| Case | Result | Tag |
|---|---|---|
| counter `F-30 >= F-8` (loop end) | `F-30 <- F-30 - F-7` (130 gives 90), `F-24 <- (F-24 + F-7) & 15` (3 gives 11): the position wraps by the loop length and the history slot offset is realigned | VERIFIED (run, 2 of 2 records per interrupt) |
| counter `> F-8 - F-7` | the rate word `F17` is loaded from `F+6` (0.5); at or below it from `F+7` (0.25): a different slew rate inside the loop region | VERIFIED (both branches) |
| counter below `F-8` and at or above `F-9` | the record is cleared by the routine at `0x1C0B08` (`F+8`, `F-30`, `F-24` and the other words of the listing `0x808..0x866` zeroed, `F+8 = 0` marks the voice free): the end of a non-looping stream stops the voice, nothing else is read past the last sample | VERIFIED (run: `F-9 = 0` stops every record) |
| page words | after a loop wrap `F-15 <- F-6` and `F-10 <- F-4 + 1` in the probe (counter 90 below `F-5 = 600`); the listing compares the wrapped counter with `F-5` before choosing `F-6` or `F-5` and incrementing `F-4`, but the oracle incremented `F-4` while keeping `F-6`: the two conditional moves are not both explained | EMPIRICAL |
| step limit | no clamp: `counter += step_int + carry` ([smp-voice](smp-voice.md) s3), so a step above 1 skips residuals and the predictor decodes noise; the ARM keeps the step at or below 1 ([piano-dsp-path](piano-dsp-path.md): cents table index `-cents`) | VERIFIED (oracle equals model at step 1.3, [smp-voice](smp-voice.md) s5), ARM side EMPIRICAL |
| ring end | the 256-word stream ring wraps by `& 0xFF` with no end test; the DSP never reads an end marker, the ARM refills ahead of the position ([smp-voice](smp-voice.md) s4) | VERIFIED (listing and runs) |

The engine's edge rule (repeat the edge sample, [piano-engine](../specs/piano-engine.md) s9) has no hardware counterpart in these words: the hardware loops (`F-8`, `F-7`) or stops (`F-9`) a stroke by position; **answered in [piano-sample-end](piano-sample-end.md) (D255):** a factory `.npno` stroke sets a loop (`u32` at record byte `0x24` start, `0x28` length), `F-9` is `total_frames`, and a looping held note never reaches `F-9`; `F+6` is `d = 2 * -i16` of the `i16` at `0x38`.

**Update (D103).** The key-up sample record is wired into the opt-in slew voice (`slew_voice.rs::start_release_sample`): `F15` 1.0 (0.922 at velocity 127), `F17 = 0x3F7FF38E`; see [piano-attack-fw](piano-attack-fw.md). No attack-shaping word exists in the lists.

**Update (D110).** The rate word `F17` is derived from the stroke record (`i16` at `0x3C`, pitch ratio), see [piano-attack-oracle](piano-attack-oracle.md).

## 4. Damper-down strokes, unison, dynamics compressor, EQ

No new hardware evidence. The slew loop and the voice loops are the whole per-record path of the SMP image (listing `0x1C0680..0x1C0A70`): there is no biquad coefficient, detune table or compressor state in them, and the post stage is the unity one-pole of [piano-bus-convention](piano-bus-convention.md) s2b. A counterpart of Unison, Dynamics or Timbre in the PNO or FX image is **not found** (consistent with [piano-dsp-path](piano-dsp-path.md): those ids send nothing); the engine's versions stay `ASSUMED`. Damper-down strokes (`kind = 1` harp strokes selected with the pedal down) have no list or record of their own in the SMP: they are ordinary strokes with a different level (`F-2..F3` ramp and `F19 / F20`), so the selection rule stays on the ARM side (stroke search, [piano-sound-load](piano-sound-load.md) s8) and is not decoded.

## 5. Level chain (item 6)

The slew does not change the steady-state level of any stroke at the engine's unit of measure unless `F11`, `F19 / F20` and the ramp ceiling differ from unity, which the note-on message does not show. The master chain (`DM(0xB6518)` word, layer words 1.6 / 2.5 / 3.25) was not re-run against the clips in this pass: `tools/emu_vs_hw.py` now lists the verified-chain variants for the Velvet Grand segment (`piano-chain`, `vel-88-piano-chain`), and the standing result of [piano-receiver-scale](piano-receiver-scale.md) s6.3 (piano chain +10 to +12 dB, organ chain -14 to -15 dB, same video, same path) holds. Defaults stay (D68).

## Reproduce

```bash
python tools/smp_slew_scn.py random target/exp/slew.scn --records 10 57   # random records, one interrupt
python tools/smp_slew_scn.py trajectory target/exp/traj.scn               # 60 updates of record 20
python tools/smp_slew_scn.py edges target/exp/edges.scn                   # loop wrap / rate / end
target/release/sharc-ref target/oracle/smp.bin target/exp/slew.scn --out target/exp/slew
cargo test -p nord-core --lib piano::smp_slew piano::ext40
```

## 6. The slew wired: ARM note and release values, voice gains (VERIFIED run, opt-in engine)

**Method.** `tools/arm_smp_slew_init.py capture` runs the real firmware note-on (library stub, bound `.npno`, `arm_piano_load`) and keeps the path 1 messages; `replay` feeds them to the SMP image in `sharc-ref` (`msg` pairs in batches of 40 per vector 13 + audio interrupt: the SMP FIFO loses the tail of a longer batch, EMPIRICAL) and captures the slew fields of the voice records after every interrupt. Sounds: Velvet Grand (velocities 20, 64, 100, 127), Bright Grand, Wurlitzer 1, Amber Upright (velocity 90), note 60.

| Item | Result | Tag |
|---|---|---|
| Records | Every note-on starts records 80 and 81 (the first one is the "extra" voice block `DM 0x9251C..21`, the second the main voice block `DM 0x92528..2D`); a note-off starts the release sample on another record (106 in the capture) and resets records 80 and 81 | VERIFIED (4 sounds) |
| Extra voice | The ARM writes `F3 = 0` and `F-1 = 0` for it and its block word `DM 0x92521 = 0` (the initial ramp): the ramp stays 0, **the gain of the extra voice is 0 for every velocity and sound captured**; only the main voice is audible | VERIFIED (4 sounds, 4 velocities) |
| Main voice at note-on | After the SMP voice-start handler (which overwrites the ARM words `F-1`, `F3`): `F-2 = 0.25` (block word `0x9252D`), `F-1 = 0`, `F2 = 0`, `F3 = 0.5`, `F11 = F12 = 0`, `F13 = F14 = F16 = F18 = 1.0`, `F19 = F20 = 0x70000000` (ARM, 0.875), `F15 = ` level word (`DM 0x9252A`, `level_q23 / 2^23`), `F17 = ` rate word (`DM 0x9252C`) | VERIFIED (oracle rows) |
| Attack | No ramp: `w = max(clip(0.25 + 0, 0.5), 0) = 0.25` from the first update, so the gain is `fix31(0.25 * level) * 0.875`, i.e. **`0.21875 * level` of Q31 full scale** (`level` = the firmware level of `NpnoLevel`). The attack time is the stroke's own | VERIFIED (4 velocities, first 8 updates exact, `slew_voice::tests::note_on_gains`) |
| Hold decay | `v' = v * F13 * F17` with `F13 = u = 1`: a slow geometric decay `1 - d / 2^24` per update (`d = 374 / 314 / 190 / 154` at velocity 20 / 64 / 100 / 127 on Velvet Grand: time constant 2.1 / 2.5 / 4.1 / 5.0 s at 43,402 Hz / 2). Other sounds at velocity 90: `d = 224` (Bright Grand), 444 (Wurlitzer 1), 284 (Amber Upright); Velvet interpolates to 221 | rate words VERIFIED per run; the law over velocity and sound is open (the engine interpolates the Velvet points for every sound, EMPIRICAL / ASSUMED) |
| Release | The note-off write sets `F11 = F12 = 0`, `F14 = 1.0`, `F16 = 0x3F7FBB6E` (0.99958) on records 80 and 81, identical for all velocities and sounds: the decay factor per update becomes `0.99958 * F17` (time constant 44 ms for `F17 = 0x3F7FFF42`, 60 dB in 0.30 s). A poked reset on a held record read back from the oracle: 4 of 5 updates equal the model, one is +1 LSB (`slew_voice::tests::release_gains`) | VERIFIED (messages, run) |
| Release sample | The key-up sample's own record gets `F15 = 0x3F7FFFFE` (about 1.0; `0x3F6C1CEA` = 0.922 at velocity 127), `F17 = 0x3F7FF38E` (time constant 0.23 s), the same `F-2 = 0.25`, `F19 = F20 = 0.875`: gain `0.21875` at most. Not wired (the engine keeps `amp * 0.30` with its 1.15 s decay) | VERIFIED messages, not modelled |
| Update clock | One update per two audio interrupts; the DSP interrupt rate is `43,402.34 Hz` ([dsp-frame-rate](dsp-frame-rate.md); one interrupt per output frame is ASSUMED for the SMP, like SY) | ASSUMED |
| Multiplier ties | 47 random records with fixed multipliers `0x10000000 .. 0x7FFFFFFF`: the model equals the oracle in 41; four are +1 LSB in the oracle (multipliers `0x50000000`, `0x10000000`, `0x7FFFFFFF` twice) and two are saturated `q` (`w v >= 1`: the oracle gives `-1` where the model gives `0x0FFFFFFF` / `0x1FFFFFFF`). The firmware multiplier `0x70000000` is exact in 6 of 6 of this set, but a release update shows the same +1 (the oracle `q` one above the model: the rounding of the 40-bit product by `FIX` differs in rare cases); the pattern follows no remainder threshold, so the tie rule is **not** decoded and the engine accepts +-1 LSB of Q31 (about -186 dB) (`piano/smp_slew.rs` test `multiplier_set_against_the_oracle`) | VERIFIED count, rule open |

**Engine.** `piano/slew_voice.rs` (`PianoLayer::firmware_slew`, `ns4 render-patch --piano-slew-fw`, GUI Firmware bar Piano group "SMP gain slew"): the main voice runs `SlewRecord` at `43,402.34 / 2 / sample_rate` updates per sample; its gain is the slew gain `F0 / F1` instead of `amp * env`; the extra voice is not started; note-off (or pedal release, or a retrigger of the same note) applies the release write; the voice ends when the gain is below `1e-6`. Dynamics and una corda keep scaling. Not slewed: release samples, damper-down strokes (legacy path), SFZ / WAV / synthetic sounds.

**Rate word law (VERIFIED per run, `tools/arm_smp_slew_init.py rates`).** `d = (1 - F17) * 2^24` of the ARM note-on, note 36 / 60 / 84 (Velvet Grand, level word in brackets): velocity 20: 238 / 374 / 790; 64: 188 / 314 / 572; 100: 140 / 190 / 404; 127: 118 / 154 / 316; velocity 88 at notes 28 / 36 / 42 / 48 / 55 / 60 / 66 / 72 / 84: 168 / 162 / 180 / 190 / 218 / 226 / 268 / 298 / 404. Bright Grand (velocity 20 / 64 / 100): 226 / 368 / 736 at 20, 184 / 328 / 664 at 64, 144 / 190 at 100 (notes 36 / 60 / 84, 100 only 36 and 60). So the rate is **faster for high keys (about x2 from note 60 to 84, x0.75 at note 36) and slower for loud notes**, and the grand sounds differ by at most 15 %. The engine's `rate_for(velocity, note)` interpolates the Velvet points bilinearly (EMPIRICAL, `piano/slew_voice.rs`); the previous velocity-only table at note 60 is its middle column. Other sounds (Wurlitzer 1, Amber Upright) differ by a factor 1.5 to 2 (ASSUMED one table).

**Clip comparison (2026-11-15, D72; details and the table in [piano-receiver-scale](piano-receiver-scale.md) s7).** Velvet Grand, Part A, velocity 88: default +7.3 dB RMS, `--piano-slew-fw` -5.7, with `--firmware-piano-chain` -3.0 dB; velocity 100: +9.7 / -4.8 / -2.2 dB. Hold decay (early to late window of a held note) hardware -2.8 dB, default -4.85 / -4.33, slew -1.81 / -2.41; release drop hardware -2.95 dB, default -7.6, slew -1.03 / +0.01; attack (10 to 90 %) hardware 87 ms, default 66, slew 120 / 138 (worse). A held chord reads -31.0 dBFS (legacy), -28.3 (chain), -51.4 (chain + slew) while the hardware DSoundman clip puts the piano at the level of the synth pad, which the legacy default reproduces: the held chord contradicts the Part A clip by about 22 dB. A 1500 tick ARM capture after a note-on writes nothing to the voice records after tick 52, so the slew values hold for the whole note. **The default stays off (D22, D72).**

```bash
uv run --with unicorn python tools/arm_smp_slew_init.py capture target/fw_unpacked/bank0_CODE.bin \
    target/fw_unpacked/bank0_SRAM.bin --cache target/keydown.snap --ns4b ".reference/<backup>.ns4b" \
    --npno "Piano/Grand/Velvet Grand Med 6.1.npno" --velocity 100 --out target/exp/on100.json
python tools/arm_smp_slew_init.py replay target/exp/on100.json target/exp/on100.scn --records 81 --rows 30
target/release/sharc-ref target/oracle/smp.bin target/exp/on100.scn --out target/exp/on100
cargo test -p nord-core --lib piano::slew_voice piano::smp_slew
```

## Implications

- The voice gain the DSP applies is a product of a ramp and a decaying state, updated every second interrupt. The note-on values are now known (section 6): constant ramp `0.25`, hold gain `0.21875 * level`, slow velocity dependent decay, 44 ms release; the extra voice is silent. The engine runs this as the opt-in `piano_slew` (rate word law over velocity and key, section 6); the default envelope is unchanged (D72).
- Loop and end are position rules of the stroke, not sample repeats; the engine's edge-sample repeat is an approximation.

## Follow-ups

1. ~~Run the ARM writers to get the slew inputs for a note~~ (done, section 6: the note-on and note-off message lists replayed through the SMP voice-start handler). Open: the law of the rate word `F17` over velocity, key and sound (`DM 0x9252C` writer), the page-switch fields `F-9 .. F-4` from the stroke descriptor, and the release sample record.
2. Map the stroke descriptor fields to `F-9 .. F-4` (loop end, loop length, page words) on a real `.npno` and replay a page switch on the oracle.
3. ~~Run the clip comparison~~ (done, section 6 and [piano-receiver-scale](piano-receiver-scale.md) s7: level, hold and release improve on Part A, attack worsens, the DSoundman held chord contradicts; no flip). Open: reconcile the two clips (the hardware-side gain of a held stroke, or the per-video recording level), and whether the key-up sample (release record) adds the missing sustain level.
4. Check the SIMD register swap and the SIMD long-word load of the oracle against the ADI simulator (the slew loop is the first consumer that depends on both).
