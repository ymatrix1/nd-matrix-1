# Synth velocity from the firmware: amp velocity, filter-envelope velocity, factory census

- **Checked:** 2026-11-17 (D142 follow-up of 2026-11-15)
- **Sources (local, never committed):** OS v1.66 firmware (`target/fw_unpacked/`), ORG core 2 image (`tools/ns4_dsp_ldr.py DSP/dsp_image_2_ORG.bin --sram DIR`), `sharc-org` (`crates/nord-sharc`), factory backup `Nord Stage 4 Factory Restore RevK v1.66.ns4b`, `docs/research/arm-edit-params.csv`, [synth-knob-laws](synth-knob-laws.md), [keydown-zones](keydown-zones.md), [synth-adr-and-strip-law](synth-adr-and-strip-law.md).
- **Question:** How does a synth note's velocity change the level and the filter in the firmware, per program and per layer, and where do better note velocities matter?

Tags follow `.agents/rules/provenance-in-code.md`. Bean `ns4-9l1f`, decisions D104 and D142 (recall check and the default flip attempt, sections 7 to 9).

## 1. Result

| Item | Result | Tag |
|---|---|---|
| Velocity parameters of the Synth section | Three edit ids only: `Edt_Syn_kAmpEnvVel` (`0x131`, 0..3), `Edt_Syn_kFltEnvVel` (`0x13c`, 0..1), `Edt_Syn_kOscEnvVel` (`0x124`, 0..1), per layer. No Synth Touch curve exists (Touch is `Edt_Pno_kKbdTouch` only, [piano-touch-curves](piano-touch-curves.md)). | VERIFIED (registry) |
| What the ARM sends | One key word per synth key, byte 1 = `0x80 + g(vel)`; `g` is the saturating S-curve of the curve-6 velocity table (`synth/key_velocity.rs`, 127 entries, equal to the ARM run). The velocity setting never changes `g`: the setting travels in the strip words | VERIFIED (run) |
| Amp velocity gain `V` | `DM(0xC10B4)` of slot 0 in the ORG core 2 pass: `bits(V) = bits(a) + ((bits(a + b) - bits(a)) * G >> 7)`, `a` = strip 0 word 53, `b` = strip 6 word 53 (`synth-knob-laws.csv`), `G = g`, `G = 128` for `g = 127`. A geometric blend from `-6 s` dB at `g = 0` to `+2 s` dB at the top, `s` = setting; setting 0 is exactly 1.0. 512 of 512 words within 1 float ulp of the oracle | VERIFIED (run) |
| Filter-envelope factor `F` | With `kFltEnvVel` on (strip 0 word 19 = 0.0, strip 6 word 19 = 1.0) the slot word `DM(0xC2723)` = `G / 128` (peak of the filter envelope scales with it, `DM(0xC117C)` follows); off it is 1.0. 256 of 256 words within 1 ulp | VERIFIED (run) |
| Oscillator-envelope velocity | `kOscEnvVel` file bit equals the recalled edit value (s7). Its law is not isolated: it is carried in strip word 0 (`((v & 0x7F) << 24) | stage`), which the harness holds constant; the engine does not apply it (unchanged by D142) | open |
| That `V` multiplies the voice amplitude and `F` the filter-envelope depth | The slot words are read by the envelope loop (`pc 0x1271C3`, `0x127CE7` read `DM(0xC10B6) = 0.0017569 V`); `DM(0xC10B6)` is linear in `V` (ratio 6.81 equals `V(3, 100) / V(3, 10)` = 1.1229 / 0.1648, s8); the final multiply into the voice is not isolated (the default image makes no sound without the recall's oscillator words) | ASSUMED |
| File bits = edit value | The layer's `amp_velocity` (2b at `+216`), `filter_velocity` (`+339`) and `osc_env_velocity` equal the edit value the firmware holds after recalling the program: 27 of 27 values (3 programs x 3 layers x 3 fields, all of 0..3 for amp, 0 / 1 for the other two), s7 | VERIFIED (program recall, D142; was ASSUMED) |

## 2. The amp law

`g` is the key byte of the velocity (`0..=127`), `s` the setting. In dB: `V(g = 0) = -6 s`, `V(g = 127) = +2 s`:

| Setting | `V(g = 0)` | `V(g = 64)` | `V(g = 100)` | `V(g = 127)` | strip 0 word 53 (`a`) | strip 6 word 53 (`b`) |
|---|---|---|---|---|---|---|
| 0 | 1.0 | 1.0 | 1.0 | 1.0 | `0x3F800000` | `0` |
| 1 | 0.501 (-6.0 dB) | 0.815 (-1.8) | 0.992 (-0.1) | 1.259 (+2.0) | `0x3F004DCE` | `0x3F41FB22` |
| 2 | 0.251 (-12.0) | 0.647 (-3.8) | 1.020 (+0.2) | 1.585 (+4.0) | `0x3E809BCC` | `0x3FAAB6D5` |
| 3 | 0.126 (-18.0) | 0.501 (-6.0) | 1.123 (+1.0) | 1.995 (+6.0) | `0x3E00E9F9` | `0x3FEF4782` |

The curve is piecewise linear with the slope doubling at every power of two (bit interpolation): the series is `docs/research/data/synth-velocity-law.csv` (768 rows, amp and filter, `tools/synth_velocity_law.py`). `g` already contains the S-curve, so the dB figures above are per key byte, not per MIDI velocity: at MIDI velocity 40 / 64 / 100 the key bytes are 12 / 32 / 98 (`KEY_VELOCITY_G`).

Against the engine's earlier law (`amp_vel = (1 - s/3) + (s/3) g / 127`, strip chain default): setting 3 is 15.8 dB louder at velocity 100 than at 40 (was 18.2), settings 1 and 2 are steeper (4.9 and 10.0 dB, were 2.4 and 6.6), and the whole curve sits higher: at `g = 64` setting 3 is -6.0 dB (was -12.0 dB).

## 3. Method

1. `tools/synth_adr_oracle.py` framework: `sharc-org` runs full ORG core 2 passes (`--dev 0x282c0380=1 --dev 0x3108d008=5`); `--msg 0:53=<word>` writes strip words, `--pre 0xC1DCC=<word>` sets the slot-0 controller word `0x01000000 | (0x80 + g) << 8`.
2. Diff of the data memory `0xC0000..0xC4000` after 40 passes between setting 0 and 3 and between `g = 100` and 10 found `DM(0xC10B4)` (and `DM(0xC10B6) = 0.0017569 x`), `DM(0xC272F)` and the controller word itself as the only movers. `kFltEnvVel` on moves `DM(0xC2723)`, `DM(0xC117C)` and the cutoff coefficient `DM(0xC2924)` (0.01014 to 0.01012).
3. `g` swept 0..=127 with one poke per 4 passes (settled in under 4): the series are the CSV. The bit-linear fit of `bits(V)` has a maximum error of 1 ulp, with the single step at `g = 127` doubled (`G = 128`).
4. Census: `cargo run --release -p nord-tools --bin synth_velocity_census -- <backup> --out docs/research/data/synth-program-velocity.csv` (`--check` re-reads).

Reproduce the series: `python tools/synth_velocity_law.py oracle` (local `sharc-org` and ORG image), check without firmware: `python tools/synth_velocity_law.py --check`.

## 4. Factory census (384 programs, 1152 layers)

Layers counted when their bit (4..6) is set in a scene mask of the program: 579 layers in 287 programs.

| Fact | Count |
|---|---|
| Layers with amp velocity 0 / 1 / 2 / 3 | 230 / 166 / 157 / 26 |
| Layers with filter envelope velocity on (and filter on) | 144 (139) |
| Layers with oscillator envelope velocity on | 42 |
| Programs with at least one velocity sensitive layer (amp or filter) | 213 of 287 |
| Programs with amp velocity on a layer | 189 |

Amp gain change from velocity 40 to 100 (`amp_fw_db_v40_v100` column): setting 1 +4.9 dB, setting 2 +10.0 dB, setting 3 +15.8 dB. Rows: [synth-program-velocity.csv](data/synth-program-velocity.csv), one per program and layer with the flags and the dB values; `sample = 1` marks a SAMPLES layer, `0` an oscillator layer.

### The three programs of the listening check

Whole-program RMS re velocity 100 (`program_sweep`, notes C3 G3 E4 C5, default engine options, `--synth-velocity` forced the firmware law, a no-op since D142, `--legacy-synth-velocity` forces the linear blend; organ / piano layers of the program are included):

| Program (synth layer) | Velocity 20 | 40 | 70 | 127 | Reading |
|---|---|---|---|---|---|
| Horizon Whl, default | -1.43 | -1.31 | -0.87 | +0.43 | layers B and C amp 1 (+4.9 dB over 40 to 100 for the synth part), layer A amp 0; the organ layers of the program dilute the change |
| Horizon Whl, firmware | -2.65 | -2.43 | -1.62 | +1.50 | |
| Leaky Stabs II, default | -2.58 | -2.33 | -1.47 | +0.66 | layer A amp 1 (+4.9 dB over 40 to 100 for the synth part), B amp 0 |
| Leaky Stabs II, firmware | -5.22 | -4.64 | -2.79 | +2.08 | |
| Grand Pad, default | -13.39 | -10.27 | -5.51 | +3.12 | layer A amp 0, filter velocity on: the level change is the filter |
| Grand Pad, firmware | -13.96 | -10.75 | -5.64 | +3.12 | filter factor `F` 0.094 at 40, 0.766 at 100 |

So the velocity sensitive content of the three is: Leaky Stabs II and Horizon Whl through the amp (setting 1, +4.9 dB for the synth part), Grand Pad through the filter envelope (`F`). Better note velocities matter most for the 26 amp-3 layers, the 157 amp-2 layers and the filter-velocity pads.

## 5. Implications

- Default on since D142 (owner approval 2026-11-17): `SynthLayer::firmware_velocity` / `--firmware-synth-velocity`; `--legacy-synth-velocity`, `--legacy-all` and `SynthLayer::legacy` restore the linear blend. The amp gain is `V(setting, g)` and the filter-envelope depth follows `F`. GUI switch `Synth: Firmware velocity`.
- Why on despite the clips: the law and the file bits are VERIFIED (s7); still open: four of 16 distinct synth clips score worse at the assumed replay velocity 100 (s9), the multiply of `V` and `F` inside the voice is ASSUMED (s8), and `kOscEnvVel` is not applied by the engine.

## 6. Follow-ups

1. Isolate the multiply of `V` and `F` in the voice (read `DM(0xC10B6)` consumers `0x1271C3`, `0x127CE7`; needs the recall's oscillator words in the ORG pass, [synth-e2e-oracle-vs-native](synth-e2e-oracle-vs-native.md)). Done in D142: the file bits (s7).
2. Oscillator-envelope velocity (`kOscEnvVel`, strip word 0 selector) and its effect on `Edt_Syn_kOscEnvAmt` depth.
3. A hardware clip of one synth program at three velocities to decide the default flip and re-baseline `render_hash`.
4. GUI Firmware bar switch for the velocity law: done in D142 (`Synth: Firmware velocity`).

## 7. File bits against the edit value (D142, VERIFIED by program recall)

`tools/arm_program_synth_velocity.py` (new, no poke) puts a factory `.ns4p` into the Store record of the EEPROM image (`arm_program_load.build_image`), boots the full machine so the firmware recalls the program, then reads `Edt_Syn_kAmpEnvVel` (`0x131`), `kFltEnvVel` (`0x13C`) and `kOscEnvVel` (`0x124`) of every synth layer store through the layer value table (`arm_edit_capture.value_cell`). Expected values are the decoder values of the census CSV ([synth-program-velocity.csv](data/synth-program-velocity.csv)). Each layer client (4, 5, 6 = layer A, B, C) owns two stores that hold the same values.

| Program | Layer A (amp, flt, osc) file / recall | Layer B | Layer C |
|---|---|---|---|
| E: Steam Piano | 3,1,0 / 3,1,0 | 0,1,1 / 0,1,1 | 2,0,1 / 2,0,1 |
| E: Sub Surface | 2,1,1 / 2,1,1 | 3,1,1 / 3,1,1 | 2,0,0 / 2,0,0 |
| C: Santoor Pluck | 2,1,0 / 2,1,0 | 3,1,0 / 3,1,0 | 1,1,1 / 1,1,1 |

27 of 27 values equal; amp takes 0, 1, 2 and 3, filter and oscillator 0 and 1, in every combination of the three layers. This is the Touch method of D99 with real programs instead of single-bit flips; it does not isolate one bit position, but every field of three programs agrees, so the decoder positions (`+216`, `+339`, osc env velocity) are correct. Reproduce: `python tools/arm_program_synth_velocity.py $C/bank0_CODE.bin $C/bank0_SRAM.bin --eeprom target/oled/eeprom.bin --ns4b "<backup>.ns4b" --program "Program/Bank E/Steam Piano.ns4p" --expect A=3,1,0 B=0,1,1 C=2,0,1` (about 3 min per program; needs the local firmware, so CI covers the pure helpers only). The other two use `--program "Program/Bank E/Sub Surface.ns4p" --expect A=2,1,1 B=3,1,1 C=2,0,0` and `--program "Program/Bank C/Santoor Pluck.ns4p" --expect A=2,1,0 B=3,1,0 C=1,1,1`. The 27 values are committed in [synth-velocity-recall.json](data/synth-velocity-recall.json); the mapping of clients 4, 5, 6 to layers A, B, C by order is EMPIRICAL (the values match per layer).

## 8. Multiply of `V` and `F` (D142, partly isolated, ASSUMED stays)

`sharc-org` on the ORG core 2 image, amp setting 3 and 0, key byte `g` 100 and 10, 500 passes (`tools/synth_adr_oracle.py` framework, no recall oscillator): `DM(0xC10B4) = V` reads 1.1229 and 0.1648 (the table of s2: 1.123 at `g = 100`); `DM(0xC10B6)` reads 0.025242 and 0.003706, ratio 6.81 = `V` ratio 6.81, so `DM(0xC10B6)` is `V x 0.02248` exactly linear in `V` (EMPIRICAL: two points). The channel record `DM(0xC1F89..)` stays 0 and the 64-word envelope frame around `0xC1188` does not move with the setting, because the default image carries no oscillator stage (the recall stream does, [synth-e2e-oracle-vs-native](synth-e2e-oracle-vs-native.md)). So that `V` reaches the voice amplitude is not shown by a level ratio at the channel record; the claim "V multiplies the voice" stays ASSUMED. The kOscEnvVel law is not isolated either.

## 9. Default flip (D142): on by owner approval, four clips marginally worse

Owner rule of 2026-11-17 (D136): a firmware-validated path becomes the default unless a hardware clip gets worse. Result below: four clips are worse, and the owner approved the flip anyway (owner approval 2026-11-17, 4 clips marginally worse at replay v100, performance velocity of clips unknown). The 17 Part A synth rows of `docs/research/data/emu_vs_hw_segments.json` (16 distinct clips; `super_lp24_horizon` repeats `synth_horizon`; plus `organ_pipe` and `pianopad_grand`) were rendered with `tools/emu_vs_hw.py` at the segment's replay velocity (100, ASSUMED: the video does not give it) once with the linear blend (`default`) and once with `--firmware-synth-velocity`. Score = `gap_score` (lower is better). Data: [synth-velocity-clips.json](data/synth-velocity-clips.json) (derived numbers only).

| Clip | Default | Firmware velocity | Change |
|---|---|---|---|
| `synth_horizon` (= `super_lp24_horizon`) | 1.89 | 2.02 | **worse** (+0.13, RMS -0.2 to -0.0 dB) |
| `super_lp24_oopad` | 4.86 | 4.89 | **worse** (+0.03) |
| `synth_sineplk` | 4.73 | 4.75 | **worse** (+0.02) |
| `synth_filterbank` | 5.39 | 5.40 | **worse** (+0.01) |
| `synth_silkpad` | 1.64 | 1.47 | better (-0.17) |
| `organ_pipe` | 1.74 | 1.70 | better (-0.04) |
| other 10 rows (`synth_lead`, `synth_crunch`, `synth_megaline`, `super_lp24_sidechain`, `super_lp24_believer`, `synth_12db`, `synth_phat3`, `synth_80s`, `synth_leaky`, `pianopad_grand`) | equal within 0.005 | | equal |

Mean of the 17 rows 2.384 against 2.390. The earlier claim "no clip varies the synth velocity, so none can worsen" (D104, D136 audit) was wrong: every clip replays notes at one velocity, and the law moves the level at that velocity (setting 3 at `g(100)`: +3.3 dB over the linear blend, `V` 1.123 against `98 / 127` = 0.77, s2).

Replay velocity sweep of the five rows that move (score linear blend / firmware law; `tools/emu_vs_hw.py` variants with `velocity`):

| Clip | v60 | v80 | v90 | v100 | v110 | v127 |
|---|---|---|---|---|---|---|
| `synth_horizon` | 1.54 / 1.45 | 1.63 / 1.57 | 1.76 / 1.79 | 1.89 / 2.02 | 2.00 / 2.36 | 2.06 / 2.64 |
| `synth_silkpad` | 4.10 / 6.77 | 2.40 / 4.31 | 1.51 / 2.62 | 1.64 / 1.47 | 2.31 / 2.84 | 3.80 / 3.80 |
| `synth_filterbank` | 7.88 / 9.09 | 6.40 / 7.94 | 5.85 / 6.45 | 5.39 / 5.40 | 5.03 / 4.85 | 4.77 / 4.70 |
| `synth_sineplk` | 4.92 / 5.08 | 4.82 / 4.93 | 4.78 / 4.83 | 4.73 / 4.75 | 4.70 / 4.68 | 4.64 / 4.64 |
| `super_lp24_oopad` | 4.71 / 4.83 | 4.79 / 4.77 | 4.83 / 4.85 | 4.86 / 4.89 | 4.88 / 4.95 | 4.90 / 4.96 |

Reading: the sign of the change depends on the replay velocity, and the velocity of the hardware performance is not known (lossy YouTube audio of a live performance, D22 and the clip notes). The firmware law is better at low replay velocity for `synth_horizon` (v60 / v80) and worse at high; for `synth_silkpad` it is better only near v100. So the clips cannot confirm the law, and at the assumed v100 four rows are worse. Under the strict D136 rule the default would stay off; the owner overrode it.

Only `synth_horizon` (+0.13) is clearly outside the noise of a lossy-video score; +0.01 and +0.02 are not, but the D136 rule is strict. The cause of the worse rows can be the replay velocity, a level already absorbed in fitted EMPIRICAL constants (synth level, LP24 factor, D81 / D116, tuned at the linear blend), or a wrong application of `V` (ASSUMED); the RMS deltas in the clip data (`default_rms_db`, `fw_rms_db`) are the first thing to compare.

Open after the flip (they were the blockers under the strict rule), in order: (1) the performance velocity of the clips (the score changes sign with it; a clip with known velocities, or the per-note velocity of the events in `emu_events`, would decide); (2) the multiply of `V` and `F` in the voice (s8, ASSUMED); (3) the `synth_horizon` clip (layers B and C amp setting 1), the largest regression (+0.13 at v100, +0.58 at v127). Closed: the file bits (s7, VERIFIED) and the GUI switch (added, `Synth: Firmware velocity`).
