# SAMPLE programs: real sample audio out of the SHARC oracle

- **Checked:** 2026-11-17
- **Sources:** [sample-voice-gate](sample-voice-gate.md) (PR #499), [dsp-flash-dma-model](dsp-flash-dma-model.md), [sy-smp-audio](sy-smp-audio.md), [sy-pitch-entry-writer](sy-pitch-entry-writer.md); tools `crates/nord-tools/src/bin/smp_stream.rs`, `crates/nord-sharc/src/bin/sharc_ref/pcm_ring.rs`, `tools/sample_stream_scn.py`, `tools/sample_stream_audio.py`, `tools/sample_stream_coverage.py`; data `data/sample-stream-audio.json` (10 programs), `data/sample-stream-coverage.json` (119 programs). Samples read from the local `.ns4b` through repo tools only; no sample or firmware bytes are committed.
- **Question:** does real sample audio come out of the SY oracle for SAMPLE programs, how close is it to the native render, and what blocks all 119?

## 1. Method

1. `smp_stream resolve` maps program to layer members (native name hash); `stream` extracts the packed `\0stk` stream and the decoded PCM of the zone for key 60 / velocity 100.
2. Two oracle feeds for SY voice 0 (SY1SY2 image, ARM messages of the k60 capture, prepare first):
   - `stream`: the real packed stream, unpacker runs. **Fails for every program** (D112 desync): ring stays zero, or trap at pc 0x1c12dd (Mm-Oh Choir).
   - `pcm` (new `sharc-ref` command): decoded PCM (x 2^-6, 1-pole tail from ARM c0/c1) written into the 128 frame source ring ahead of the read position. Unpacker bypassed; SY resampler, step word and ring read are the real firmware code.
3. Pitch entry `e` is **modelled** (`pack_ratio(pitch_word(key)/0.05)` = 790 for key 60, ASSUMED, D65), because its writer is unfound. Pitch agreement is therefore by construction of `e`, not an independent check.
4. Native: `ns4 render-patch --note 60 --velocity 100`.

## 2. Results, 10 programs (key 60, vel 100, 1.5 s, SY 43402 Hz)

Valid = ARM sent a prepare message in the k60 window. Level = peak dBFS.

| Program | Valid | oracle peak | native peak | oracle f0 / native f0 Hz | oracle centroid / native Hz | pitch vs native (spectral slide, 25 c grid) |
|---|---|---|---|---|---|---|
| Acoustic Guitar | yes | -38.2 | -18.5 | 263.0 / 263.7 | 1287 / 1898 | 0 c |
| Mm-Oh Choir Whl | yes | -41.1 | -21.4 | 261.5 / 260.9 | 960 / 1637 | 0 c |
| Solo Flute | yes | -38.4 | -16.1 | 261.5 / 260.9 | 995 / 1452 | 0 c |
| Bell Pad | yes | -41.1 | -19.4 | 263.0 / 262.3 | 442 / 428 | 0 c |
| Finger Bass | yes | -38.6 | -15.8 | 493 / 131 | 773 / 300 | f0 pair implies about +2290 c; spectral slide saturates at its 1200 c limit (see below) |
| Hybrid Str Whl | no | -43.7 | -18.6 | invalid | | +375 c artefact |
| Pop Brass 3 | no | -41.7 | -16.9 | invalid | | +375 c artefact |
| Cathedral Pipes | no | -37.5 | -15.4 | invalid | | +375 c artefact |
| 3 Vlns Mellotron | no | -39.5 | -23.1 | invalid | | +375 c artefact |
| NS4 Palette | no | silent | -12.7 | | | no voice |

- All five valid programs are non-silent for 1.5 s (clipped by the run length). Level is 19.7 to 22.8 dB under native (19.7, 19.7, 22.3, 21.7, 22.8): scale, layers, FX and master are not modelled (unknown, EMPIRICAL gap).
- Centroid: lower than native on 3 (Guitar, Choir, Flute), about equal on Bell Pad (442 vs 428 Hz), higher on Finger Bass (773 vs 300 Hz; that voice runs about two octaves sharp). Only the first layer and no FX are simulated.
- No hardware clip of SAMPLE audio exists, so `tools/emu_vs_hw.py` has nothing to compare; the comparison is oracle vs native only.

Notes on precision:

- `vs_native_cents` is a spectral slide on a 25 cent grid capped at +-1200 c: "0 c" means "within one grid step", not exact. The pitch agreement claim rests on the autocorrelation f0 pairs, which agree to about 5 cents (Guitar 263.0 / 263.7, Choir and Flute 261.5 / 260.9, Bell 263.0 / 262.3 Hz).
- Finger Bass: f0 493 Hz vs 131 Hz is about +2290 c, so the slide value of +1100 c is its cap, not the shift. One of the two autocorrelation f0 values may also be an octave error; not resolved. Either way the oracle voice is far sharp, consistent with `e` lacking octave / transpose (step ratio 1.52).
- Duration is **not compared**: every run is 1.5 s (a fixed run length, the guitar stroke lasts about 3 s), so "non-silent for 1.5 s" says nothing about decay.

## 3. The programs that queue no voice

PR #499 listed 9 (6 scene-II-only, 2 with a split range that excludes key 60, Unclocked Pad unexplained). The 119-program capture (prepare `0x92414 <- 0x1c07cd`, keys 36 / 60 / 84) finds **18** with no prepare at any of the three keys, a superset by this measure (different probe: one key per phase, no gate hack). Re-capture of the three scene-I ones: Grand Vibra Bs prepares at k36 only (its range excludes 60, as diagnosed); B3 Old Forest and Unclocked Pad prepare at no key, so scene-I is not the cause there and both stay **unexplained**. EMPIRICAL; no firmware change tried.

## 4. Coverage of the 119 programs (`data/sample-stream-coverage.json`)

| Class | Programs |
|---|---|
| Voice prepare seen at key 60 | 50 |
| Voice prepare only at key 36 or 84 | 51 |
| No prepare at any of the 3 keys | 18 |
| No sample member resolved | 0 |
| Layers 1 / 2 / 3 | 45 / 40 / 34 |

Blockers to full coverage, in order of reach:

| # | Blocker | Programs |
|---|---|---|
| 1 | Real stream unpacker desync / trap (D112): `stream` mode silent | 119 |
| 2 | Writer of pitch entry `e` unfound; modelled `e` is key only (no octave / transpose) | 119 (Finger Bass: f0 pair about +2290 c) |
| 3 | Only voice 0 and layer 1 simulated; 2 and 3 layer programs lose layers | 74 |
| 4 | Prepare not in the k60 window: needs another key or voice allocation understood | 51 |
| 5 | No voice at any key (zone range / scene II / unknown) | 18 |
| 6 | Level 20 to 23 dB under native (scale, FX, master) | 119 |
| 7 | Harness quirks, causes unknown: prepare must be the first message; per loop `poke 0x92410 0x200` (EMPIRICAL, `sy_voice_sample.scn`) keeps the refill bookkeeping above 0x52; the `pcm` feed refills 96 frames ahead (ASSUMED) | 119 |
| 8 | No hardware clip of SAMPLE audio | 119 |

## Implications

- The SY resampler path produces real sample audio for programs with a k60 prepare (5 of 10 sampled, 50 of 119 eligible) when the unpacker is bypassed. This is not yet a 1:1 parity result: pitch is by construction, level is off.
- Nothing changes in the native engine; `--sample-pitch fw` stays opt-in.

## Follow-ups

1. Fix the unpacker desync (D112) so `stream` mode runs.
2. Find the writer of `e` (OQ22) or capture `e` from the ARM for the played key.
3. Simulate all voices and layers; measure level against a hardware clip.
4. Explain the 18 no-voice programs and the 51 that voice only at other keys.
