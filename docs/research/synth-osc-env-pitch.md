# Osc Env Amount under Env To Pitch: DSP consumer of word 12 (OQ44)

**Checked:** 2026-11-21. **Sources (local, never committed):** OS v1.66 firmware images, ORG core 2 image (`sharc-sys` oracle), recall captures of 9 factory programs (`tools/arm_dsp_capture.py`), factory `.ns4p` files; tool `tools/synth_osc_env_pitch.py`; data [synth-osc-env-pitch-oracle.csv](data/synth-osc-env-pitch-oracle.csv), [synth-osc-env-pitch-traces.csv](data/synth-osc-env-pitch-traces.csv); [morph-fw-osc-env-dst](morph-fw-osc-env-dst.md), [synth-env-law](synth-env-law.md).
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

Of the 384 factory programs 17 set the Env To Pitch bit on some layer (22 layers); 9 programs have a non-zero amount (14 layers). Peak excursion `T(127 abs(x) / 60)` at level 1 (the sustain 0.5 halves it):

| Program (layer) | Amount | Peak st |
|---|---|---|
| Wooden Queen (A) | 44 | 22.1 |
| Cinema Rise (A, B, C) | -38, -18, -20 | 15.2, 2.4, 3.1 |
| Lyle Lead J3 (B) | 21 | 3.5 |
| Circus Box (A) | -11 | 0.7 |
| Saw Horn (A) | 7 | 0.2 |
| Detuned Fanfare, Bounce Clav, OB Fanfare, Trumpet Synth | 1 to -4 | below 0.1 |

3 programs (Wooden Queen, Cinema Rise, Lyle Lead J3) move the pitch by more than 2 semitones; the amount is below 10 steps on 6 of the 9. The decoder field was read with `tools/ns4_synth_fields.py` (layer enable not checked).

Layer A of the 8 programs with a layer A amount was traced ([synth-osc-env-pitch-traces.csv](data/synth-osc-env-pitch-traces.csv), pitch relative to the amount-0 run, 6 times from 10 to 250 ms; oracle = `sharc-sys` ORG core 2 with the recall capture, osc ADR and word 12 injected; native = `synth_e2e_native --no-filter --unity --synth-osc-env-morph`, note 60):

| Program | Result |
|---|---|
| Wooden Queen | oracle 11.03 to 11.04 st, model 11.03, native opt-in 10.89 to 11.16 st, native default -0.13 to 1.08 st: **the opt-in matches the oracle within 0.2 st at 6 of 6 times** |
| Saw Horn, Bounce Clav, Detuned Fanfare, Trumpet Synth, OB Fanfare | oracle and native within 0.3 st of 0 (excursions below the pitch estimator); two native windows (Saw Horn and OB Fanfare, 80 ms) read a 19 st estimator jump on the saw spectrum, not a pitch event |
| Circus Box (-0.35 st), Cinema Rise (-8 to -15 st) | **not resolved**: the windowed peak estimator fails on both sides (Circus Box reads silence at 6 of 6 native times, Cinema Rise is a noisy pad with large downward excursions); no claim |

## 5. What is not covered

* Layers B and C (strip and word addresses differ; Lyle Lead J3 and two Cinema Rise layers); Cinema Rise and Circus Box pitch traces (a better detector, or a tap on the pitch index word instead of the audio).
* The key velocity term and `kOscEnvVel` (the oracle runs used one key velocity).
* `kOscEnvDst = 0` (Osc Ctrl) native depth: unchanged, `ASSUMED`.
* Audible A/B of the opt-in on the 3 programs; the flip to default is the owner's call.

## Implications

OQ44 is answered: the DSP scale is 48 semitones per unit word times the envelope level, linear in level, odd in sign. The native opt-in reproduces it on the one program with a large settled excursion (Wooden Queen). No default changes; flip proposal in D214.

## Follow-ups

Cinema Rise and Circus Box traces with a better pitch detector; layers B / C words; velocity term; listening A/B of Wooden Queen, Cinema Rise, Lyle Lead J3 with the flag.
