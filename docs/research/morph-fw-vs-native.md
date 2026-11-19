# Morph (Wheel, A.T., Ctrl Ped): firmware law against the native engine

Checked: 2026-11-17 (task `ns4-kq1o`, decision [D159](../DECISIONS.md)). **2026-11-18 (D166): the firmware law is the default; `--legacy-morph` restores the native law described as "default" below.**

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
| FX delay rate (14 bit) | decoded, **not played** (`ns4-13ua`) | not played |
| Synth Osc Ctrl, resonance, LFO rate, LFO amount, filter env amount | decoded, **no slot, not played** | played (`synth_morph.rs`) |
| Organ drawbar morph | decoded, not played | not played, firmware not captured (section 6) |
| Arp rate / range, OscEnvAmt, Leslie speed morph | decoded or absent, not played | not played |

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
- Organ drawbar morph (`Edt_Org_Preset_kDb1..9`): the test program carried non-zero drawbar
  depths, yet no drawbar cell held a depth after recall and no drawbar id was re-dispatched. The
  native code states drawbar morph acts in Preset mode only (`drawbar_morph_applicable`); the
  firmware condition is not captured. Open.
- Not captured: FX delay-rate morph audio (`ns4-13ua`), Leslie speed morph, arp rate / range
  morph, `Edt_Ext_kCCValue*` (cells hold the registry default `-127` for all three sources).
- DSP side: the firmware reaches the DSP through the same setters as a knob (value to word
  already `VERIFIED` for cutoff, `synth-filter-freq-law.csv`); the morph run reproduces the
  measured words (table in section 4). The SY / ORG oracle was not rerun for the other
  targets: nothing differs between a morphed and a static dispatch of the same edit value.
- With `--firmware-morph` the native synth morph acts on the engine fields (`resonance`,
  `osc_ctrl`, `lfo_*`, `filter_env_amount`); the strip knob words of the firmware synth path
  (`strip_knobs`) are not re-sent. Copy, Paste and Clear Morph carry the five new slots since D166
  (`engine/morph_clip.rs`); a live edit of one of the five fields becomes the new base
  (`SynthMorphSet::apply`), and cleared depths put the base back once.
