# Synth chain end to end: SHARC oracle on the real ARM stream against native `nord-core`

- **Checked:** 2026-11-17
- **Sources (local, never committed):** ORG core 2 image (`tools/ns4_dsp_ldr.py target/DSP/dsp_image_2_ORG.bin --sram DIR`), OS v1.66 ARM firmware through `tools/arm_dsp_capture.py` (recall plus key 60 velocity 100 of five factory programs, [arm-dsp-capture](arm-dsp-capture.md)), `sharc-sys` (new `--tap`), native `synth_e2e_native` (new), driver `tools/synth_e2e_oracle.py` and `tools/synth_e2e_metrics.py` (tests `tools/tests/test_synth_e2e_*.py`). Derived numbers only: [data/synth-e2e-oracle-vs-native.csv](data/synth-e2e-oracle-vs-native.csv). No audio, firmware or sample content is stored or printed. Background: [synth-filter-stage-vs-model](synth-filter-stage-vs-model.md) (D122), [synth-voice-control](synth-voice-control.md), [keydown-zones](keydown-zones.md) s7, [dsp-peripherals-audio](dsp-peripherals-audio.md) (D120).
- **Question:** D122 showed the filter stages equal the Rust port when both get the same words. Nobody measured the whole synth voice (oscillator, envelope, gain, filter) of the oracle against the native engine on a played note. Does the real ARM stream make the oracle produce that voice, and which stage carries the gap to `nord-core`?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Does ORG core 2 make sound from the real ARM stream? | **Yes for a synth program, no output lane.** The recall installs the program's oscillator stage in strip 0 word 0 (`0x1212D3`, `0x121AB5`, `0x121C54`, `0x1215FB` in the five programs) and its filter in word 1 (`0x1253BD`, `0x12566E`, `0x12549D`); the key word `DM(0xC1DCC)` (+ `0xC1DCD` for a second lane) starts the voice (stage runs in 699 of 1000 passes after key-on). The pass buffers and every output lane stay silent (`--lane` peak 0 for sets 0..3): the voice ends in the channel record, 4 samples per pass at `DM(0xC1F96 - 13)`, written in place by the B (filter) stage. The new tap reads it | VERIFIED (`sharc-sys --tap`) |
| Who receives the key | The ARM sends the key word to **PNO core 2 only**, never to ORG core 2. The PNO core 2 and ORG core 2 images give the same voice for the same words (identical envelope trace, 80s Sci-Fi) | VERIFIED |
| Sample rate of the stage | 4 samples per pass, pass = 2 SY frames at 43,402.34 Hz, so 86,804.69 Hz. The oracle fundamental reads 261.59 Hz for Smooth Lead (expected 261.63, -0.3 cent) and 130.86 Hz for 80s Sci-Fi (octave -1, expected 130.81, +0.7 cent): **under 1 cent on these 2 of 5** programs (one oscillator, no detune). Crunch Synth reads 261.77 (+0.9 cent) but native reads 264.09 for the same note, so its pitch is ambiguous; 12dB Sweep reads 260.92 (-4.7 cent, unison 3 detune ASSUMED); Phat 3 reads a harmonic (526 Hz). So pass = 2 frames (ASSUMED in D116, D121) holds to a few cents on four programs and to under 1 cent on two; it is not measured on the other three | EMPIRICAL (2 of 5 under 1 cent, 4 of 5 within 5 cent) |
| Oscillator path level, oracle against default native | `-1.9`, `+0.3`, `+11.0`, `+11.4`, `+4.4` dB (12dB Sweep, 80s Sci-Fi, Crunch Synth, Phat 3, Smooth Lead). With `--synth-stages --synth-p verified` the gap grows to `+6.0 .. +14.6` dB (Phat 3 stays `+11.4`). **Not unison / layer normalised**: the oracle is one channel record, native sums the layer (section 4) | VERIFIED (measurement), cause open |
| Filter stage gain (filtered over unfiltered, steady RMS) | oracle `+4.3`, `-4.2`, `+5.9`, `-2.3`, `+6.0` dB; native `-0.7`, `-9.5`, `+0.1`, `-0.1`, `+0.1` dB. Oracle minus native: `+4.9`, `+5.3`, `+5.8`, **`-2.2`**, `+5.9` dB (`--synth-fw-filter`: `+4.6 .. +5.4`, Phat 3 `-2.2`). Four programs (12dB Sweep, Crunch, Smooth Lead **and the Ladder24 80s Sci-Fi**) sit `+4.9 .. +5.9` dB above native; only Phat 3 (Ladder24) is the outlier. The top of the range is `20 log10 1.98 = 5.93` dB, the table A input gain of D122 at resonance 0 | VERIFIED (measurement), the 1.98 reading ASSUMED |
| Spectral shape | band shape difference (third octaves within 80 dB of the peak, **each vector normalised to its own peak**, floor at -80 dB): oscillator path `2.9 .. 4.4` dB on three programs but `19.6` (Crunch) and `29.1` (Phat 3) on two; after the filter `21 .. 46` dB. The filter adds shape error on the three programs whose oscillator path is close (12dB Sweep, 80s Sci-Fi, Smooth Lead); on Crunch and Phat 3 the shape already differs before the filter | VERIFIED (measurement) |
| Envelope | **Not comparable.** The three envelope chain states (`F - 20`, `F - 8`, `F + 6`, `F = 0xC1188`) show the same ramp (1.023 s per unit, release tau 0.323 s) in all five programs although the programs' file attack values differ (Smooth Lead 22, the others 0). The recall stream does not carry the program's envelope words; the records stay at the image values, so **the oracle's filter and amp modulation is not the program's** (confound, section 5). The audio rise time (90 % of steady) is 5 .. 30 ms in the oracle against 5 .. 15 ms native, but the audio rise is not the chain ramp | VERIFIED (identical traces), cause open |
| Which stage dominates | **Not settled.** Decomposing the filter-output level gap (`+3.0`, `+5.6`, `+16.8`, `+9.2`, `+10.4` dB) into oscillator term and filter-gain term: oscillator `-1.9`, `+0.3`, `+11.0`, `+11.4`, `+4.4`; filter `+4.9`, `+5.3`, `+5.8`, `-2.2`, `+5.9`. The oscillator term is at least as large as the filter term on **2 of 5** (Crunch, Phat 3), the filter term on the other three (Smooth Lead only by 1.5 dB). The band shape gap grows through the filter on 3 of 5 and is large before it on 2 of 5. With the envelope and unison confounds below, the data support "both stages differ and the filter gain is a repeatable `+5` dB on four programs", not "the filter dominates" | VERIFIED (numbers), attribution open |

## 2. Method

1. **Stream.** `tools/arm_dsp_capture.py one` boots the ARM with the program in the Store record, runs the recall (800 ticks), idle (100), then key 60 velocity 100 on and off ([arm-dsp-capture](arm-dsp-capture.md)). `core2_load_script` turns the ORG core 2 recall (933 words, phases `load` and `idle`) into a `sharc-sys --script-flat` file; `key_script` turns the PNO core 2 key words into messages at pass 300 and pass 300 + 10,851 (0.5 s hold). Every key word of the phase is delivered in capture order (a two lane layer sends `0xC1DCD` before `0xC1DCC`).
2. **Oracle.** `sharc-sys IMAGE --epdma --script-flat load --script-at 0:key --tap 0xC1F89:8 --tap 0xC1174:1 --tap 0xC1180:1 --tap 0xC118E:1 --tap-out FILE`, 16,575 passes. The tap writes the listed `DM` windows after every pass (little-endian `u32`, pass-major). Samples are the PEx lane of the record, 4 per pass; the three single words are the envelope chain states. Second run with `DM(0xC0B8C) = 0x125B79` (null B stage) poked before every pass: the oscillator path (A stage output, filter bypassed).
3. **Native.** `synth_e2e_native PROGRAM --layer 0 --frames --start --hold [--no-filter] [option flags]` renders `SynthLayer::render_frame` of layer A (before layer level, pan, FX and master) with the same note, velocity and timing; `--no-filter` is the native oscillator path. Variants: default, `--synth-fw-filter`, `--synth-stages --synth-p verified --synth-fw-filter`.
4. **Metrics** (`synth_e2e_metrics.py`): RMS level of the steady part (0.15 s after the note on to 0.05 s before the note off), 5 ms windowed RMS envelope and 90 % rise time, third-octave band levels and their mean absolute difference, the null depth of the band levels and of the envelope after a best-fit gain, the spectral peak pitch, and the unaligned waveform null (about 0 dB: the oscillator phase is not aligned, so it carries no information).

```
cargo build --release -p nord-sharc --bin sharc-sys -p nord-tools --bin synth_e2e_native
uv run python tools/ns4_dsp_ldr.py target/DSP/dsp_image_2_ORG.bin --sram DIR
C=target/fw_unpacked
uv run --with unicorn python tools/arm_dsp_capture.py one $C/bank0_CODE.bin $C/bank0_SRAM.bin \
    --eeprom target/oled/eeprom.bin --out target/arm_dsp_all "factory_programs/Program/Bank A/Smooth Lead.ns4p"
uv run python tools/synth_e2e_oracle.py report target/arm_dsp_all/Bank_A__Smooth_Lead.txt.gz \
    "factory_programs/Program/Bank A/Smooth Lead.ns4p" --sharc-sys target/release/sharc-sys \
    --image DIR/core2_sram.bin --native target/release/synth_e2e_native --workdir target/synth_e2e \
    --native-arg=--synth-fw-filter
uv run python tools/synth_e2e_oracle.py csv default=r.json fwfilter=r2.json
```

## 3. Per program (default native, `--synth-fw-filter` in brackets where it differs)

Programs: layer A of `12dB Sweep` (Shape 2, type 0, unison 3), `80s Sci-Fi` (Multi 1, Ladder24, octave -1), `Crunch Synth` (Shape 0, type 0, unison 1), `Phat 3 Osc MiniM` (Pure 2, Ladder24, octave -1, three layers play), `Smooth Lead` (Shape 0, LP24). All analog, no WAVE, no FM, no sample. Levels are RMS dB, oracle minus native.

| Program | Osc path level d | Osc path shape d | Filter gain oracle / native (diff) | Filter out level d | Filter out shape d | Env null (dB) |
|---|---|---|---|---|---|---|
| 12dB Sweep | -1.9 | 3.8 | +4.3 / -0.7 (+4.9) | +3.0 (+2.6) | 21.4 (18.9) | -6.3 |
| 80s Sci-Fi | +0.3 | 2.9 | -4.2 / -9.5 (+5.3) | +5.6 (+5.3) | 44.3 (42.5) | -7.3 |
| Crunch Synth | +11.0 | 19.6 | +5.9 / +0.1 (+5.8) | +16.8 (+16.4) | 42.3 (39.8) | -8.2 (-8.1) |
| Phat 3 Osc MiniM | +11.4 | 29.1 | -2.3 / -0.1 (-2.2) | +9.2 (+9.1) | 46.3 (47.2) | -6.9 |
| Smooth Lead | +4.4 | 4.4 | +6.0 / +0.1 (+5.9) | +10.4 (+9.6) | 33.2 (30.9) | -11.8 |

Shape d = level-normalised third-octave band difference, dB (`band_delta_db`, floor 80 dB below each peak). The first version of this metric was not level normalised and ignored bands below the floor in one vector only; the numbers above replace it.

The pitch estimate is the strongest spectral peak, so a resonant filter can report a harmonic (12dB Sweep filter out 522 Hz against 261 Hz; Phat 3 oracle 526 Hz against 131 Hz with the oscillator path at the same value): the pitch rows of the CSV are only a rate check where oracle and native agree (Smooth Lead, 12dB Sweep oscillator path, 80s Sci-Fi).

Reading (ASSUMED where marked):

- The oscillator offsets do not follow the file unison value (3, 0, 1, 1, 0 give `-1.9`, `+0.3`, `+11.0`, `+11.4`, `+4.4`), but the native column sums every voice of the layer and the oracle reads one channel record (section 4), so no unison or layer count is normalised out here. They are the level the strip stage emits against the engine's fitted per-wave level (`p_table.rs`, EMPIRICAL): D122 predicted that the fitted factors absorb what is upstream, this puts a number on the oscillator part. The verified stage path (`--synth-stages --synth-p verified`) is *further* from the oracle, so the fitted defaults hide a missing gain rather than a wrong one (ASSUMED).
- The filter offset (oracle gain minus native gain) is `+4.9 .. +5.9` dB on four programs, including the Ladder24 80s Sci-Fi (its absolute oracle gain `-4.2` dB is low only because native is `-9.5`). The range tops out at the table A input gain at resonance 0 (1.98, +5.93 dB), an ASSUMED link. Phat 3 (Ladder24, `-2.2`) does not follow it and is the open Ladder24 position case of D122.
- The envelope chain is the one thing the real stream could not provide: see section 5.

## 4. Where the oracle voice goes

The ORG core 2 pass writes the voice into the channel records (`0xC1F96 + 0x36 c` then `0xC2942 + 0x34 c`, [org-stage-map](org-stage-map.md) s2). No ARM word in the recall or the key path installs a mix or output stage that moves it into `0xB0000..0xB2FFF`: 16 words change there, all constants. So the full-chain *output* of the oracle (layer gain, pan, mix) is **not** available on this stream and the comparison stops at the filter output, before layer level (D87), pan and the FX chip.

**Channel and unison mismatch (confound).** The oracle tap reads channel 0, PEx lane, of the stage record: one voice. The native render is `SynthLayer::render_frame` of the whole layer, which sums every unison voice (file unison 3, 0, 1, 1, 0; Phat 3 sends several key lanes in the capture). Oracle and native therefore do not carry the same voice count; every level difference in sections 1 and 3 includes that mismatch, and it is not corrected. An equal-count comparison needs the other channels tapped or native forced to one voice (follow-up 7).

**Envelope confound.** Because the oracle envelope chain stays at the image default in all five programs, the filter envelope and amp envelope reaching the oracle filter and gain are not the program's. The filter gain and shape rows compare the oracle's default modulation with native's program-specific one. They cannot say how much of the shape gap is envelope driven.

## 5. Implications

- The synth chain is now measurable end to end on the real stream. The first numbers say both the oscillator and the filter stage differ from native, and neither dominates: the oscillator level term is at least the filter term on Crunch and Phat 3, the filter gain is a repeatable `+4.9 .. +5.9` dB on four programs, and the shape gap exceeds 19 dB before the filter on two programs. The earlier attribution to the filter alone is not supported (the stage code itself is right on equal words, D122).
- Native `--synth-fw-filter` does not close the filter gain: the real filter output is about 5 dB above the engine's on four programs with the engine at unity. Candidate: the `cin` input gain, ASSUMED, to be tested by feeding the native filter the table A gain.
- Fitted factors (`p_table.rs`, `p_type.rs`) cannot be re-derived from this alone: the oscillator and filter levels disagree in sign across programs, the envelope is the image default and the voice count is not matched.
- The SY stage rate agrees with pass = 2 frames to under 1 cent on two programs and within 5 cent on four; OQ22's "pass = 2 frames" stays ASSUMED with this support, not VERIFIED.

## 6. Follow-ups

1. Envelope: send the program's `Edt_Syn` envelope edits (or run the ARM edit path) so the records carry the program's attack, decay, release; then compare the amp chain, the filter chain and the audio rise per program.
2. Test the `cin` hypothesis: read table A input gain per resonance in the oracle for the five programs and apply it in `fw_filter` input (native, opt-in) and re-measure the filter gain row.
3. Oscillator level: tap the P block gain words the ARM sends (`synth-p-words.csv`) per program and compare with the per-wave fitted levels.
4. Ladder24 programs (80s Sci-Fi, Phat 3): position word of a sounding voice (D122 open item).
5. Output stage: find which ARM message installs the mix / output hand-over for synth layers, so the lane `--wav` comparison against hardware clips can include layer level and pan.
6. Extend to the other filter types and to unison / two-lane layers (this run reads channel 0 PEx only).
7. Match the voice count: tap every channel of the layer and sum, or force native to one unison voice, then redo the level rows.
8. Repeat the rate check on an unambiguous single-voice program per octave to settle pass = 2 frames.
