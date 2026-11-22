# Morph (Wheel, A.T., Ctrl Ped): firmware law against the native engine

Checked: 2026-11-17 (task `ns4-kq1o`, decision [D159](../DECISIONS.md)). **2026-11-18 (D166): the firmware law is the default; `--legacy-morph` restores the native law described as "default" below.** **2026-11-21 ([D204](../DECISIONS.md), section 7): organ drawbar, Leslie speed, delay rate, arp range / rate and Osc Env Amount morph captured or bounded; the FX delay rate morph is played (it reaches the audio only with the opt-in `FirmwareFx::delay_time`).** **2026-11-21 ([D206](../DECISIONS.md), section 8): the Osc Env Amount morph is captured on all 9 programs and played with the opt-in `--synth-osc-env-morph`.**

Sources: OS v1.66 ARM image run in the Unicorn harness (`tools/arm_program_morph.py`, Store-record
recall of a program, MIDI on UART2); factory `Program/Bank A/Horizon Whl.ns4p` from the local
RevK v1.66 backup (never committed); `crates/nord-core/src/{morph.rs,synth_morph.rs,engine/mix.rs,
cbin/synth_preset.rs,fx/mod.rs}`; manual p.43-44 (Morph).

Question: how does the ARM turn Wheel / A.T. / Ctrl Pedal into the edit values it sends to the DSPs,
and where does the native engine differ? Why does A:16 `Horizon Whl` (layer A cutoff morph wheel
0.079, three synth layers) sound far from the hardware clip?

## 1. Method

1. A program goes into the Store record of the EEPROM image (`arm_program_load.build_image`); the
   firmware boots and recalls it (about 100 s).
2. UART2 gets `B0 01 v` (Mod Wheel, `Fnc_MIDIWheel`), `B0 0B v` (Control Pedal, `Fnc_MIDIExpr`) or
   `D0 v` (Aftertouch, `Fnc_MIDIAftertouch`), then 100 ticks run.
3. A code hook on the layer client dispatch `0xC00CC4F0 (store, 0, id, &value)` logs every edit id
   the firmware re-dispatches and the Q16.16 value behind the pointer: that is the morphed edit
   value the setter receives (the same path as a knob turn, `arm_synth_freq_law.py`).
4. `tools/morph_test_program.py` writes a copy of a factory program with a distinct depth on every
   target (`DEPTHS`: synth A `+10 / +20 / -30` steps, synth B `+50 / +100 / -100`, synth C
   `+5 / -15 / +25`, FX `+12 / +24 / -36`, piano and organ levels); it is for the harness only and
   is not committed.
5. `tools/morph_law_report.py` fits `value(v) = a + b w(v)` per series with the firmware weight
   `v / 128` and the native weight `v / 127`.

Data: `docs/research/data/morph-fw-law.csv` (5390 dispatch rows: 51 single-event scenarios plus 4
combinations), `morph-fw-law-fit.csv` (245 fitted series), `morph-fw-horizon.csv` (Horizon Whl, wheel
1 / 32 / 64 / 96 / 127, with the ORG core 2 cutoff words), `morph-horizon-cutoff.csv` (value to
word to Hz of the two Horizon cutoff morphs, both laws).

## 2. Firmware law (all VERIFIED by run)

| Item | Firmware | Check |
|---|---|---|
| Value cells | `[base, wheel, aftertouch, pedal]`, plain integers in edit steps (`Edt_Syn_kFltFreq` of Horizon layer A reads `[52, 10, 0, 0]`; file `0.07874 x 127 = 10`) | `--cells` run, 39 morph-capable ids of `arm-edit-params.csv` (registry column `4;...`) |
| Controller input | the three sources arrive as MIDI: CC1, CC11 and channel aftertouch all re-dispatch their ids | 245 series, `morph-fw-law.csv` |
| Weight `w(v)` | `v / 128`, exactly `1.0` at `v = 127` | fit residual 0.0001 steps over every series, all three sources; `+127` depth reads 0.9922, 1.9844, ..., 125.0156, 127.0 for `v = 1, 2, ..., 126, 127` |
| Sum | `x = base + dW w(v_W) + dA w(v_A) + dP w(v_P)` | wheel 127 + A.T. 127 + pedal 127 on synth A cutoff (52, +10, +20, -30) reads 52 |
| Clamp | on the total, to the id range (`0..127`; `0..16383` for the 14-bit delay rate) | synth B cutoff (64, +50, +100, -100): wheel 114, A.T. 127 (clamp), pedal 0 (clamp), wheel + A.T. 127, **wheel + A.T. + pedal 114** (not `127 - 100 = 27`), either order |
| Delivery | one re-dispatch per change, value in Q16.16 (`0x3E0000` = 62), no slew in the ARM stream (the settled value is there by tick 3) | scratch per-tick probe of the piano A level (not committed; `arm_program_morph.py` settles 100 ticks) |
| Cutoff sender | sender `0xC01C56CC` writes ORG core 2 word `(x - 52) << 24` (Q8.24) for a Q16.16 value `x`, **one whole step lower once frac(x) >= 0.5** (`54.4375 -> 0x02700000`, `54.5 -> 0x01800000`, `54.75 -> 0x01C00000`); the word is fractional, not rounded | static dispatch run of 54.0 .. 55.0 and `morph-fw-horizon.csv` `dsp` rows: wheel 32 on layer A (x = 54.5) sends `0x01800000`, wheel 64 on layer C (x = 40.5) sends `0xF3800000`; test `cutoff_rows_reproduce_the_measured_horizon_words` |

The Morph group ids `Morph_Grp_kOne..Five` have range `0..128`, which is the weight scale `v / 128`.

## 3. Native law and what is wired

| Target | Legacy (`--legacy-morph`, the engine law before D166) | Firmware law (default since D166, `--firmware-morph` is still accepted) |
|---|---|---|
| Weight | `v / 127` (`MorphControllers`) | `v / 128`, full at 127 (`firmware_weight`) |
| Sum and clamp | `clamp(base + sum d w, 0, 1)` per parameter (same as firmware) | same |
| 7 layer levels | played (`layer_levels`, then the layer gain law) | same |
| 3 synth cutoffs | played, geometric interpolation of the 128-step table | same, plus the sender quirk (`synth_morph::sender_norm`, minus one step from frac 0.5) |
| 9 FX slots per chain | played (Mod 1/2 rate and amount, EQ mid, drive, delay mix, feedback, reverb) | same |
| FX delay rate (14 bit) | decoded, **not played** (`ns4-13ua`) | played since D204 (`FxMorphSet::delay_rate`, section 7.2); it changes the delay only with the opt-in `FirmwareFx::delay_time` (the engine time law ignores the 14 bit value) |
| Synth Osc Ctrl, resonance, LFO rate, LFO amount, filter env amount | decoded, **no slot, not played** | played (`synth_morph.rs`) |
| Organ drawbar morph | decoded, not played | not played; no factory program uses it, the firmware recall does not apply it (section 7.3) |
| Arp range morph | range decoded, depths dropped | not played; dispatch captured, the range scale is not (section 7.4) |
| Arp rate morph | depths dropped | not played; no factory program uses it |
| Osc Env Amount (`kOscEnvAmt`) morph | decoded since D206, not played | played only with the opt-in `--synth-osc-env-morph` (D206, section 8; 9 programs); default render unchanged |
| Leslie speed morph | absent | not played; no factory program uses it, layout `EMPIRICAL` (section 7.5) |

## 4. Native law against firmware law, in numbers

Weight (edit steps of a depth `d`, error `= d (w_native - w_fw)`):

| `v` | `w_fw` | `w_native` | error at `d = 127` |
|---|---|---|---|
| 1 | 0.0078 | 0.0079 | +0.0 |
| 32 | 0.2500 | 0.2520 | +0.25 |
| 64 | 0.5000 | 0.5039 | +0.50 |
| 100 | 0.7813 | 0.7874 | +0.77 |
| 126 | 0.9844 | 0.9921 | +0.98 |
| 127 | 1.0000 | 1.0000 | 0 |

Largest native-law residual over the 245 fitted series: 0.78 steps on 7-bit ids (depth 99 steps),
35.99 steps of 16383 on the delay rate (depth 4608). The native law is early by at most 0.8 % of the
depth and joins at both ends.

Cutoff, Horizon Whl (`morph-horizon-cutoff.csv`, word measured in the ORG core 2 `dsp` rows of
`morph-fw-horizon.csv`; Hz from the 128-step display table with geometric interpolation of the
fractional word, `ASSUMED` for the DSP side):

| Layer | Wheel `v` | Dispatched knob | Sent knob | Word | Hz fw | Hz native | Native vs fw |
|---|---|---|---|---|---|---|---|
| A | 0 | 52 | 52 | `0x00000000` | 277.0 | 277.0 | 0.00 dB |
| A | 32 | 54.5 | 53.5 | `0x01800000` | 302.4 | 320.7 | +0.51 dB |
| A | 64 | 57 | 57 | `0x05000000` | 370.0 | 370.8 | +0.02 dB |
| A | 96 | 59.5 | 58.5 | `0x06800000` | 403.3 | 428.8 | +0.53 dB |
| A | 127 | 62 | 62 | `0x0A000000` | 494.0 | 494.0 | 0.00 dB |
| C | 0 | 34 | 34 | `0xEE000000` | 98.0 | 98.0 | 0.00 dB |
| C | 32 | 37.25 | 37.25 | `0xF1400000` | 118.5 | 118.6 | +0.01 dB |
| C | 64 | 40.5 | 39.5 | `0xF3800000` | 134.9 | 143.4 | +0.53 dB |
| C | 96 | 43.75 | 42.75 | `0xF6C00000` | 162.7 | 173.2 | +0.54 dB |
| C | 127 | 47 | 47 | `0xFB000000` | 208.0 | 208.0 | 0.00 dB |

Two sources of difference: the weight (above, at most 0.8 % of a depth) and the sender dropping a
whole step once the fraction of the dispatched knob is 0.5 or more. The sender term dominates:
up to about 0.5 dB of cutoff where it fires, 0 at rest and at full wheel. `--firmware-morph`
reproduces both terms (the default since D166); `--legacy-morph` keeps `v / 127` and no drop.

## 5. Horizon Whl (A:16)

Cells after recall (`--cells`, `morph-fw-horizon.csv`; every depth is a wheel depth, A.T. and pedal
are 0):

| Target | Base | Wheel depth | Wheel 127 | Native |
|---|---|---|---|---|
| Piano A level | 0 | +127 | 127 | played (piano A is silent at rest, in both) |
| Synth A cutoff | 52 (277 Hz) | +10 | 62 (494 Hz) | played |
| Synth B filter env amount | 32 | +23 | 55 | **not played with `--legacy-morph`**; played by default since D166 |
| Synth C level | 60 | +30 | 90 | played |
| Synth C cutoff | 34 (98 Hz) | +13 | 47 (208 Hz) | played |
| Delay mix of two FX chains | 0 | +38 | 38 | played |

Conclusion: with the wheel at rest (0) every morph target of Horizon Whl equals its base in the
firmware and in the native engine, so the morph system does not explain a gap at rest. With the
wheel up the native default differs by up to 0.5 dB of cutoff (sender step drop), 0.8 % of a depth in level and mix,
and the missing synth B filter-envelope morph (+23 of 127 steps of envelope amount at full wheel).
That is the whole morph-side difference for A:16. The gap the owner hears must come from
elsewhere (arpeggiator, D81, the SUPER and MULTI stages on the layers); this note does not claim
it explains the video.

## 6. Limits and follow-ups

- Sources were driven by MIDI (CC1, CC11, `D0`). The physical wheel and pedal jack reach the ARM
  through other functions (`Fnc_WheelDirect`, `Edt_Sys_kPedalExpression*`); the law behind those
  is `ASSUMED` equal (same morph group ids). `Edt_Sys_kPedalExpressionGain` shapes the pedal
  value before the morph (native: `pedals.control_gain`, ASSUMED).
- Organ drawbar morph (`Edt_Org_Preset_kDb1..9`): bounded in section 7.3 (the recall leaves the
  cells at their defaults; writing depths by hand does not subscribe the ids; no DSP word moves
  with a key held). The firmware condition stays open, but no factory program needs it.
- Not captured: `Edt_Ext_kCCValue*` (cells hold the registry default `-127` for all three
  sources). Delay rate, Leslie speed and arp morph: section 7.
- DSP side: the firmware reaches the DSP through the same setters as a knob (value to word
  already `VERIFIED` for cutoff, `synth-filter-freq-law.csv`); the morph run reproduces the
  measured words (table in section 4). The SY / ORG oracle was not rerun for the other
  targets: nothing differs between a morphed and a static dispatch of the same edit value.
- With `--firmware-morph` the native synth morph acts on the engine fields (`resonance`,
  `osc_ctrl`, `lfo_*`, `filter_env_amount`); the strip knob words of the firmware synth path
  (`strip_knobs`) are not re-sent. Copy, Paste and Clear Morph carry the five new slots since D166
  (`engine/morph_clip.rs`); a live edit of one of the five fields becomes the new base
  (`SynthMorphSet::apply`), and cleared depths put the base back once.

## 7. Uncaptured targets, second pass (2026-11-21, D204)

Sources: the same harness (`tools/arm_program_morph.py`, new options `--dsp-all`, `--settle`, event
`note:N`), local firmware (OS v1.66, `target/fw_unpacked/`, never committed), the factory backup
`Nord Stage 4 Factory Restore RevK v1.66.ns4b` (local), `tools/morph_usage_survey.py`,
`tools/morph_test_program.py`. Data: `data/morph-fw-delay.csv`, `data/morph-fw-arp-range.csv`.

Question: what does the firmware do for the morph targets section 6 left open (organ drawbars, delay
rate, Leslie speed, arp rate / range), and what is worth porting?

### 7.1 How many of the 384 programs use each target

`python tools/morph_usage_survey.py "<backup>.ns4b" --list` reads the raw morph words (no decoder
loss) of every program. Drawbar and delay rate agree with the decoder (`ns4 convert --to-json`) on
all 384 programs; arp rate / range offsets are the decoder's (`cbin/synth_preset.rs`,
`synth_arp.rs`); the Osc Env Amount layout is checked against the ARM cells (7.4); the Leslie layout
is `EMPIRICAL` (7.5).

| Target | Programs | Wheel | A.T. | Pedal | Which |
|---|---|---|---|---|---|
| Organ drawbar (18 bars) | **0** | 0 | 0 | 0 | none |
| Leslie speed | **0** | 0 | 0 | 0 | none |
| FX delay rate | **6** | 2 | 4 | 0 | Frozen Time Whl, Q and A, TapePiano AT, J3 Split Special, J3 Super Sawz, Taped Section |
| Arp rate | **0** | 0 | 0 | 0 | none |
| Arp range | **1** | 1 | 0 | 0 | FM Harpies |
| Osc Env Amount (not on the task list, found while capturing) | **9** | 6 | 3 | 0 | Sine-Saw Plk Whl, Chrome Layer, FM Cloud Whl, Favorite Video, Distorted Whl II, Rich FM Pad Whl, Cassette Pad AT, FM Harpies, Time Warp Saw |

The delay-rate depths are large: J3 Super Sawz wheel `-8319` steps from 16383 (20 ms to about 400
ms), J3 Split Special A.T. `-8191`, TapePiano AT `-2175`, Taped Section `-4096`, Q and A `-1152`,
Frozen Time Whl `+128` from 13568.

### 7.2 FX delay rate: law and audio VERIFIED, played since D204

Run: factory program `Bank A / B3 Old Forest` rewritten by `morph_test_program.py` (delay-rate depths
`+12 / +24 / -36` of 127, i.e. `+1548 / +3096 / -4644` steps of 16383, base 11008), MIDI wheel /
A.T. / pedal, 100 ticks per event, DSP messages of the PNO chip core 1 logged (`--dsp-all`).
`data/morph-fw-delay.csv`:

| Event | Dispatched `kDlyRateClkDiv` | Rounded | DSP word `c[0x15]` (DM `0xB28AF`) |
|---|---|---|---|
| wheel 1 | 11020.0938 | 11020 | 10803 |
| wheel 33 | 11407.0938 | 11407 | 9331 |
| wheel 64 | 11782.0 | 11782 | 7734 |
| wheel 100 | 12217.375 | 12217 | 6661 |
| wheel 127 | 12556.0 | 12556 | 6007 |
| A.T. 64 | 12556.0 | 12556 | 6007 |
| A.T. 127 | 14104.0 | 14104 | 2913 |
| pedal 127 | 6364.0 | 6364 | 20231 |

(The CSV has the two further rows wheel 5 and A.T. 1.)

* The dispatch is `base + sum(depth_i w_i)` with `w = v / 128` (`1.0` at 127), the section 2 law, on
  a 14 bit id (clamp `0..16383`).
* The DSP word is `fw_delay_time::delay_word(round(x))` on all 10 rows (test
  `fx::tests_fw_delay_morph::morphed_value_gives_the_measured_dsp_time_word`). The handler
  `0xC010C514` rounds the Q16.16 cell with `(cell + 0x8000) >> 16`
  ([fx-delay-time](fx-delay-time.md) s2), so a fractional morph value is rounded to the nearest
  step, as the native port does.
* Wheel + A.T. + pedal at 127: dispatch 11008 (`1548 + 3096 - 4644 = 0`), but the DSP word in that
  run stayed at the 6007 of the first event: **not a result**. The delay unit is told by the clock
  object `0xC0024CC8`, which is rate limited by a time stamp ([fx-delay-tempo](fx-delay-tempo.md)
  s5); three events inside one settle window leave the unit on the first value. Single events
  (the table) are not affected. `Frozen Time Whl` (also run with `--settle 2000`) shows no time word
  in this harness; its dispatches are `13632` (wheel 64) and `13696` (wheel 127) = 13568 `+ 128 w`.
* Native before D204: the morph deltas were decoded and dropped. Now `FxMorphSet::delay_rate` is
  loaded (`program_fx.rs`, export in `program_fx_export.rs`, GROUP sharing, Copy / Paste / Clear
  Morph with 10 FX slots) and `apply_morph` sets `delay_rate_raw = round(16383 clamp(base + sum d
  w))`. Tempo sync reads the same raw value through `fw_delay_sync` (the sync hand-off is
  `ASSUMED`, fx-delay-tempo s5).
* The audio effect needs `FirmwareFx::delay_time` (opt-in, off by default): the engine time law
  (`delay_time_ms`) does not look at the 14 bit value, so without the switch nothing changes (test
  `engine_time_follows_the_morph_only_with_the_firmware_switch`). The firmware cross-fades two taps
  for 45 ms on a time change; the engine keeps its own slew (`ASSUMED`).

### 7.3 Organ drawbar morph: not applied by the recall, not needed by any factory program

Runs on `B3 Old Forest` (both organ layers on, Preset): the test program (all depths) and a variant
with only `+32 / 127` wheel depth on all 18 drawbars (bars `8 8 6 4 0 0 7 0 0`).

* Zero of 384 factory programs store a drawbar depth (7.1).
* `Edt_Org_Preset_kDb1..9` cells exist in both organ stores (`0x9100bed0`, `0x9102e760`, domain
  `0xC`) but hold registry defaults after recall: `kDb9` reads base 8 although the program stores
  0, and all depth words are 0 although the file carries `+32 / 127`. The Store recall does not
  load organ drawbars or their depths (they live in a different record, `Buffer_kOrgDrawbar`,
  [arm-organ-messages](arm-organ-messages.md) s6), so nothing subscribes the ids to the morph.
* Writing a depth into the cell by hand (`kDb5` `[0, 16, 0, 0]`, `kDb1` `[8, -4, 0, 0]`) and
  sending wheel 127 re-dispatches no drawbar id: the dispatcher loop at `0xC00CCA90` walks a
  per-source subscription list (`[manager + 0x10]`; the layer level and synth ids are in it), not
  the cells.
* With key 60 held (`note:60`; the note changes PNO core 1 words against a no-note scenario),
  wheel 64 / 127 change no ORG or PNO weight word (one word, `DM 0xE0E78`, differs between the two
  and is absent from the no-wheel run: a smoothing state).

Result: **no firmware condition under which drawbar morph applies was found.** The recall path that
should subscribe the ids is not exercised by the harness (the organ preset load is the unknown).
The native stance (drawbar morph only in Preset mode, `drawbar_morph_applicable`) stays `ASSUMED`.
It costs nothing today: no factory program has a drawbar depth.

### 7.4 Arp range, arp rate, Osc Env Amount

* **Arp range** (`FM Harpies`, layer A, base 7, wheel depth `+22`): cells `[7, 22, 0, 0]`,
  dispatch 18.0 at wheel 64 and 29.0 at wheel 127 (`data/morph-fw-arp-range.csv`): the section 2
  law, played by the firmware. The native engine keeps `SynthArpDecoded::range` raw and drops the
  depths; the raw range to octave scale is `ASSUMED` (`range_thirds`) and the sequencer's reading
  of the dispatched value was not captured, so no port. One program uses it.
* **Arp rate**: zero programs; `synth_preset.rs` reads the field and drops the depths. Not ported.
* **Osc Env Amount** (`Edt_Syn_kOscEnvAmt`, range `-60..60`): the file carries depths (`+92`, 7 bit
  signed base, then three 8 bit words that store `depth + 120`, `cbin-and-presets.md` s5). Cells of
  `FM Harpies` layer A `[21, 2, 0, 0]` and layer C `[30, 10, 0, 0]` match the file words; the
  dispatch is `21 + 2 w` (22 at wheel 64, 23 at 127) and `30 + 10 w` (35, 40): the section 2 law.
  The setter word is `knob / 60` (`VERIFIED`, [synth-knob-laws](synth-knob-laws.md)). The native
  engine has no slot (the five D166 slots do not include it) and nine programs use it: the largest
  remaining synth morph gap and a follow-up of its own (decoder field, slot, clip, tests).

### 7.5 Leslie speed

`Edt_OrgCom_kEffectsLeslieSpeed` (`0x18D`) has four cells. Organ payload bits `81..91` read
`SS 011 011 011` in every factory program (`SS` = the speed code): a 2 bit base and three 3 bit
words that store `depth + 3`, depth 0. This layout is `EMPIRICAL` (the constant pattern is the only
evidence). Test: the original `B3 Old Forest` with the wheel word set to `+2` and A.T. to `-2` (raw
bit patch, no JSON round trip): `--cells` lists no Leslie cell and no Leslie id is re-dispatched.
Either the layout is wrong or the recall does not load the depth; the harness cannot tell which.
Zero programs use it. Not ported.

### 7.6 Correction to the 245-series data

`morph_test_program.py` goes through `ns4 convert --to-json`, which does not carry the arp rate /
range depths nor the Osc Env Amount depths, and repacks them as zero words (depth `-127` / `-120`
on all three sources). In the 245-series run (`morph-fw-law.csv`) the series `Edt_Syn_kArpRange`
and `Edt_Syn_kOscEnvAmt` are therefore harness artifacts (`-0.9375` at wheel 1 is `-120 / 128`),
not firmware behaviour; the unpatched `B3 Old Forest` and `FM Harpies` runs hold no such phantom
depths. The fit of the other series is not affected.

### 7.7 Implications

* The delay-rate morph is VERIFIED end to end (dispatch, rounding, DSP word) and played; audible
  only with `delay_time`, so the default render is unchanged. Flipping `delay_time` is a separate
  owner decision (D204 does not propose it).
* Drawbar, Leslie speed and arp rate morph need no work today (0 programs); the arp range morph
  needs the range scale first (1 program).
* Osc Env Amount morph (9 programs) is worth a port: done in section 8 (D206, opt-in).

### 7.8 Follow-ups

- ~~Osc Env Amount morph slot~~ done in section 8 (D206).
- Organ drawbar subscription path (which recall function registers the ids) if a program with
  drawbar depths ever appears (user programs can).
- Delay hand-off in the harness: longer `--settle`, one event per scenario for the three-source
  combination, to turn the stale 6007 word into a measurement.
- Arp range: capture the sequencer's range from the dispatched value (`arp_firmware_walk`).

## 8. Osc Env Amount morph: capture of all 9 programs and the opt-in slot (2026-11-21, D206)

**Checked:** 2026-11-21. **Sources (local, never committed):** OS v1.66 `bank0_CODE.bin` /
`bank0_SRAM.bin`, the factory backup `Nord Stage 4 Factory Restore RevK v1.66.ns4b`, the harness
`tools/arm_program_morph.py --cells --dsp-all` (new `--history`), `tools/morph_osc_env_report.py`.
**Question:** what does the firmware do for the `Edt_Syn_kOscEnvAmt` (`-60..60`) morph of the 9
programs that use it, and can the native engine play it?

Programs (decoder-checked, section 7.1): Sine-Saw Plk Whl (A, wheel +42), Chrome Layer (C, base
-21, wheel -15), FM Cloud Whl (C, wheel +18), Favorite Video (A, A.T. -40), Distorted Whl II (A,
base 21, wheel +6), Rich FM Pad Whl (B, base 60, wheel -45), Cassette Pad AT (A, A.T. -35), FM
Harpies (A base 21 wheel +2, C base 30 wheel +10), Time Warp Saw (A, A.T. -48): 10 synth layers.
Scenarios: `wheel:64`, `wheel:127`, `at:127`, `pedal:127` and all three; 27 rows with a dispatch,
[data/morph-fw-osc-env.csv](data/morph-fw-osc-env.csv).

| Check | Result |
|---|---|
| Cells `[base, wheel, A.T., pedal]` equal the decoded file words (`depth + 120` stored, 8 bit at `+99 / +107 / +115`) | 10 of 10 layers |
| Dispatch equals `clamp(base + sum d w(v), -60, 60)`, `w = v / 128`, 1.0 at 127 | **27 of 27** rows, fractions kept (`-28.5`, `37.5`), the clamp is not hit by a factory program |
| DSP word (ORG core 2 common 0 word 12) equals float32 `x / 60` | **21 of 21** wheel rows, max error 2e-3 steps (float32 truncation, `0x3EBBBBBB` for 22 / 60); fractional values are not rounded |
| Layer B word | Rich FM Pad Whl writes `0xE01FE`, not `0xE0314` (stays 0); exact `x / 60` at both wheel values; `EMPIRICAL`, one program |
| DSP word of the **A.T. programs** (3 programs, 6 rows) | not `x / 60`: dispatch `-48` writes `-34.5`, `-24` writes `-6.1875`. **Cause (D213, [morph-fw-osc-env-dst](morph-fw-osc-env-dst.md)):** these layers have Env To Pitch on; the word is `sign(x) T(127 abs(x) / 60) / 48` (pitch taper), exact on 6 of 6 rows and 242 static steps |

The A.T. words are a smooth, monotone function of the dispatched value
([data/morph-fw-osc-env-at.csv](data/morph-fw-osc-env-at.csv), Time Warp Saw, A.T. 1 .. 127) and
the other two A.T. programs fall on the same curve (`-35 -> -15.68`, `-40 -> -21.67`; the curve
gives about 15.7 and 21.6 by linear interpolation), so the source is the dispatched value, not the program or
the layer level (Favorite Video has no level morph). The wheel programs read exactly `x / 60`. The cause (OQ43, closed by D213) is the Env To Pitch bit, see [morph-fw-osc-env-dst](morph-fw-osc-env-dst.md).
The knob value is the same for every source; only the destination changes the word.

**Native port** (`synth_morph.rs` slot 5, `osc_env_amount`): the knob position is
`(steps + 60) / 120`, so the bipolar amount `-1..1` is `2 norm - 1` and the file depth `d` steps
is `d / 120` (`MorphDeltas` of the decoder, `osc_env_amount_morph`); the sum and clamp are the
shared `MorphParameter::evaluate` (clamp `-60..60` steps is `0..1` here). `SynthMorphSet::apply_with`
writes `SynthLayer::osc_env_amount` only when `Stage4Engine::synth_osc_env_morph` is on
(`--synth-osc-env-morph`, default off); `apply` keeps its old behaviour, `--legacy-morph` skips the
whole set, so the default render and `--legacy-morph` are unchanged. A live edit becomes the new
base and clearing the depths puts the base back, like the five D166 slots. Copy / Paste / Clear
Morph carry the slot (`MorphClip`). Export keeps the base depths (`LOSSY_FIELDS`). Round trip over
the 384 factory programs re-encodes byte-identically (temporary corpus test, not committed: the
`.ns4p` files are local); 10 synth layers carry a non-zero depth.

Tests: `synth_morph_osc_env_tests.rs` replays the 27 CSV rows through `SynthMorphSet::apply_with`
(dispatch within 1e-3 steps) and the 21 wheel DSP words (2e-3 steps); `cbin::synth_raw` decodes the
FM Harpies words; `engine::tests_firmware_morph` and `render_opts` cover the gate and the flag.

**Implications.** The slot's value is `VERIFIED` for all 9 programs and all sources (dispatch 27 of 27, DSP word 27 of 27 by destination, D213). The flag stays opt-in because the engine's Env To Pitch depth (+/-12 st, `EMPIRICAL`) is not the firmware taper (OQ44); the flip proposal is in D213. No listening comparison was made.

**Follow-ups.** OQ44 (pitch depth scale); the audible effect of the osc envelope depth in the
affected programs (render A/B with `--synth-osc-env-morph`); layer B word `0xE01FE` against a
static dispatch.
