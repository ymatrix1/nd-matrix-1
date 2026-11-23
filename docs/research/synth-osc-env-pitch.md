# Osc Env Amount under Env To Pitch: DSP consumer of word 12 (OQ44)

**Checked:** 2026-11-22 (layers B / C, velocity; first checked 2026-11-21). **Sources (local, never committed):** OS v1.66 firmware images, ORG core 2 image (`sharc-sys` oracle), recall captures of 9 factory programs (`tools/arm_dsp_capture.py`), factory `.ns4p` files; tool `tools/synth_osc_env_pitch.py`; data [synth-osc-env-pitch-oracle.csv](data/synth-osc-env-pitch-oracle.csv), [synth-osc-env-pitch-traces.csv](data/synth-osc-env-pitch-traces.csv); [morph-fw-osc-env-dst](morph-fw-osc-env-dst.md), [synth-env-law](synth-env-law.md).
**Question:** what does the ORG core 2 oscillator stage do with common 0 word 12 under Env To Pitch: scale in semitones, curve, sign, and how does the native engine compare?

## 1. Answer

Under Env To Pitch the oscillator pitch moves by `48 * word * level` semitones, `word` the float32 the ARM writes to `DM(0xE02F4)` (`osc_env_dsp_word`) and `level` the osc envelope chain `F + 6 = DM(0xC118E)`. Positive words raise, negative words lower the pitch. With the taper of the word (`T(127 abs(x) / 60)`, `0..48`) the excursion at full level is `T(127 abs(x) / 60)` semitones, so Env Amt 60 is 48 semitones (4 octaves).

| Fact | Evidence |
|---|---|
| The Env To Pitch bit changes three ORG core 2 words at recall: strip 0 words 20 and 24 (`0xE0014` `1.0 -> 0`, `0xE0018` `-1.0 -> 0`, the Osc Ctrl routes) and common word 4 (`0xE02EC`, `0 -> 0x30000000`) | recall diff of Time Warp Saw with the bit patched both ways (15,302 messages each) |
| With the bit set, a word 12 sweep moves the pitch, not the wave shape | Time Warp Saw recall: word 0.5 gives 1047 Hz from 262 Hz (24.0 st). With the bit clear the same word only changed the amplitude of the 12dB Sweep output |
| Pitch is `48 * word * level` | [synth-osc-env-pitch-oracle.csv](data/synth-osc-env-pitch-oracle.csv): 12 words (`-1..1`) x 4 levels (`0.25..1`, `F + 6` poked every pass, osc ADR rates at the slowest index), 46 rows, FFT peak of `DM(0xC1F89)`, max error 0.029 st |
| Sign follows the word | rows with words `-1 .. -0.1`: pitch below the note by the same amount |

The Osc Ctrl route (bit clear) is a separate path: the pass block at `0x24D24C` adds `strip word 20 * DM(F + 10)` to the wave position word `DM(0xC1F91) = 0.5 + clip(...)`, `F + 10` being `F + 6 * word`.

## 2. Envelope level: the oracle osc chain stops at 0.5

Per program the oscillator pitch follows `48 * word * F+6` with `F+6` held at 0.5 once the decay is done: Wooden Queen (amount 44, word 0.4597, peak 22.1 st) sits at 11.04 st (model 11.03); Circus Box and Saw Horn show the same 0.5 factor. This is the decay sustain of D210 (amp and filter chains), now also found on the osc chain (3 of 3 programs with a visible excursion). The native chain decayed to 0.

## 3. Native engine (opt-in `--synth-osc-env-morph`)

`SynthLayer::osc_env_fw_pitch` (set per frame from `Stage4Engine::synth_osc_env_morph`) turns on, for `osc_env_to_pitch` layers only:

* pitch `= osc_env_pitch_semitones(amount * 60, level)` = `48 * osc_env_dsp_word * level` (`ASSUMED`: key velocity scales the level);
* the osc envelope decay stops at 0.5 (`VERIFIED`, section 2).

Default path unchanged (+/-12 st linear, `EMPIRICAL`).

## 4. Affected programs and oracle against native

Of the 384 factory programs 17 set the Env To Pitch bit on some layer (22 layers); 9 programs have a non-zero amount (13 layers; the 14 of D217 was a miscount). Peak excursion `T(127 abs(x) / 60)` at level 1 (the sustain 0.5 halves it):

| Program (layer) | Amount | Peak st |
|---|---|---|
| Wooden Queen (A) | 44 | 22.1 |
| Cinema Rise (A, B, C) | -38, -18, -20 | 15.2, 2.4, 3.1 |
| Lyle Lead J3 (B) | 21 | 3.5 |
| Circus Box (A) | -11 | 0.7 |
| Saw Horn (A) | 7 | 0.2 |
| Detuned Fanfare, Bounce Clav, OB Fanfare, Trumpet Synth | 1 to -4 | below 0.1 |

3 programs (Wooden Queen, Cinema Rise, Lyle Lead J3) move the pitch by more than 2 semitones; the amount is below 10 steps on 6 of the 9. The decoder field was read with `tools/ns4_synth_fields.py` (layer enable not checked).

Layer A of the 8 programs with a layer A amount was traced ([synth-osc-env-pitch-traces.csv](data/synth-osc-env-pitch-traces.csv), [synth-osc-env-pitch-layers.csv](data/synth-osc-env-pitch-layers.csv), [synth-osc-env-pitch-velocity.csv](data/synth-osc-env-pitch-velocity.csv), pitch relative to the amount-0 run, 6 times from 10 to 250 ms; oracle = `sharc-sys` ORG core 2 with the recall capture, osc ADR and word 12 injected; native = `synth_e2e_native --no-filter --unity --synth-osc-env-morph`, note 60):

| Program | Result |
|---|---|
| Wooden Queen | oracle 11.03 to 11.04 st, model 11.03, native opt-in 10.89 to 11.16 st, native default -0.13 to 1.08 st: **the opt-in matches the oracle within 0.2 st at 6 of 6 times** |
| Saw Horn, Bounce Clav, Detuned Fanfare, Trumpet Synth, OB Fanfare | oracle and native within 0.3 st of 0 (excursions below the pitch estimator); two native windows (Saw Horn and OB Fanfare, 80 ms) read a 19 st estimator jump on the saw spectrum, not a pitch event |
| Circus Box (-0.35 st), Cinema Rise (-8 to -15 st) | **not resolved**: the windowed peak estimator fails on both sides (Circus Box reads silence at 6 of 6 native times, Cinema Rise is a noisy pad with large downward excursions); no claim |

## 5. Layers B and C, the two open programs, key velocity (2026-11-22)

**Voice layout (VERIFIED, oracle runs, `tools/synth_osc_env_pitch.py`).** Layer `L` plays in voice slot `L` and reads strip `2 L` (62 words each), the common block `0xE02E8 + 32 L` (word 12 at `0xE02F4 + 32 L`) and the osc ADR at `0xE0020 + 124 L`. Its osc envelope chain `F + 6` is `0xC118E + 62 (L >> 1) + (L & 1)` (two lanes per strip chain: A `0xC118E`, B `0xC118F`, C `0xC11CC`) and its voice record `0xC1F89 + 54 L` (A `0xC1F89`, B `0xC1FBF`, C `0xC1FF5`). Check: Cinema Rise with one key word at a time moves exactly the chain and record of that slot; word 12 sweeps of Cinema Rise and Detuned Fanfare move only the slot of the layer written.

**Held-level law, all 13 layers ([synth-osc-env-pitch-layers.csv](data/synth-osc-env-pitch-layers.csv)).** `F + 6` poked to 0.5 and 1.0, ADR at the slowest index, pitch against the word 0 run, once with a probe word 0.1 (4.8 st at level 1) and once with the program's own word:

| Layer | Own word, level 1 / 0.5: oracle against `48 word level` |
|---|---|
| Cinema Rise A (-38) | -15.27 / -7.62 st against -15.22 / -7.61 |
| Cinema Rise B (-18) | -2.36 / -1.18 against -2.37 / -1.18 |
| Cinema Rise C (-20) | -3.07 / -1.53 against -3.07 / -1.53 |
| Circus Box A (-11) | -0.69 / -0.34 against -0.69 / -0.35 |
| Wooden Queen A (44) | 22.07 / 11.03 against 22.07 / 11.03 |
| Detuned Fanfare A, B | 0.01 against 0.01 |
| Saw Horn, Trumpet Synth, OB Fanfare, Bounce Clav A | within 0.09 st (estimator floor on unison saws) |

Probe word: every sounding row within 0.09 st of `48 word level` (worst: Saw Horn, 2.313 against 2.4 at level 0.5). Cinema Rise and Circus Box are therefore resolved by the held-level law; the windowed estimator of the dynamic traces still fails on Cinema Rise A and Circus Box (section 4).

**Dynamic traces, B and C ([synth-osc-env-pitch-traces.csv](data/synth-osc-env-pitch-traces.csv), now with a `layer` column).** Cinema Rise B: oracle -2.21 to -1.89 st, native opt-in -2.33 to -1.79; Cinema Rise C: oracle -2.93 to -2.16, native opt-in -3.04 to -2.07. **The native opt-in matches the oracle within 0.12 st at 12 of 12 times on layers B and C**; the native default (linear) is 1 st off. Detuned Fanfare B is flat in both.

**Silent layers.** Bounce Clav B and Lyle Lead J3 B give no signal in their oracle voice record (Bounce Clav has one key word; Lyle has two key words but only record 0 sounds and no word 12 reaches it), so no pitch is measurable. Their amounts (-4: 0.06 st; +21: 3.5 st at level 1) are not verified. Native plays Lyle Lead J3 B at 1.74 st (settled); whether the hardware sounds that layer is open (layer enable not decoded here). Wooden Queen and Cinema Rise A remain the only audible excursions above 2 st.

**Key velocity (VERIFIED).** The ARM key word carries the velocity as byte 1 = `0x80 + g`. Wooden Queen recalled with `kOscEnvVel` patched on moves the amount word 0.4597 from common word 12 to strip 6 word 14 (a velocity-blended pair, like the filter velocity pair), and the pitch follows `48 word level G / 128`: `g` = 0 / 16 / 32 / 64 / 98 / 127 gives 0 / 2.76 / 5.52 / 11.03 / 16.90 / 22.07 st ([synth-osc-env-pitch-velocity.csv](data/synth-osc-env-pitch-velocity.csv), model equal within 0.002 st at 6 of 6). With the bit off the pitch ignores `g` (22.07 st at every `g`). `G = g`, 128 at `g = 127`: the filter envelope factor of [synth-velocity](synth-velocity.md). The native opt-in used the linear note velocity (`ASSUMED`); it now uses `G / 128`.

Both oracle core 2 images (`target/PNO` and `target/synth_env/sram`) give 22.066 st for the same run. Commands: `tools/synth_osc_env_pitch.py layers | trace | velocity` (module docstring).

## 6. What is not covered

* Lyle Lead J3 B and Bounce Clav B (silent in the oracle record).
* The velocity term on layers B and C (measured on layer A; same layout).
* `kOscEnvDst = 0` (Osc Ctrl) native depth: unchanged, `ASSUMED`.
* Audible A/B of the opt-in; the flip to default is the owner's call.

## Implications

OQ44 is answered: the DSP scale is 48 semitones per unit word times the envelope level (times `G / 128` with Env Velocity), linear in level, odd in sign, the same on layers A, B and C. The native opt-in reproduces it on Wooden Queen and Cinema Rise B / C. No default changes; flip proposal in D217 / D222.

## Follow-ups

Whether Lyle Lead J3 B and Bounce Clav B sound on the hardware (layer enable); listening A/B of Wooden Queen and Cinema Rise with the flag.
