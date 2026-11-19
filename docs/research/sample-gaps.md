# SAMPLE audio gaps of the oracle comparison: level, layers, no-voice programs, harness quirks

- **Checked:** 2026-11-17
- **Sources:** [sample-stream-audio](sample-stream-audio.md) (D150), [arm-dsp-capture](arm-dsp-capture.md), [sample-voice-gate](sample-voice-gate.md), [sy-smp-audio](sy-smp-audio.md); tools `crates/nord-tools/src/bin/smp_stages.rs`, `smp_stream.rs` (`zones`), `crates/nord-sharc/src/bin/sharc_ref/pcm_ring.rs` (`ahead=`, `credit=`), `tools/sample_gaps_{cap,cov,layers,probe,scenes}.py`, `tools/sample_stream_{audio,scn}.py`; data `data/sample-gaps-{audio,coverage,scenes,zones}.json`. Numbers only: no firmware, sample or audio content is committed.
- **Question:** the oracle SAMPLE audio of D150 sat 20 to 23 dB under native, lost layers in 74 of 119 programs, never voiced 18 programs, and used harness tricks (per-loop poke, prepare first, 96-frame refill-ahead, Finger Bass octave). Which of these are harness omissions and which are a native error?

## 1. Answer in one table

| Gap (D150) | Cause found | Kind | Fixed / explained |
|---|---|---|---|
| (a) level 20 to 23 dB under native | the oracle ring is `pcm x 2^-6`, native voice is `... x 0.25`: 24.08 dB; the oracle has no SY receiver stage, no layer gain and no program level | harness omission (not a native error) | explained, stage table s2 |
| (b) layers lost (74 programs with 2 or 3 layers) | the ARM harness served only the first 4 MiB of the member; strokes beyond it were not resident (EMPIRICAL: the member size is the only change between the two captures). With the whole member: 86 of 113 programs start every scene I layer at key 60, 24 start some (22 of them with 3 layers: 2 voices) | harness gap, 3rd layer open | fixed (`--member-bytes`), s4 |
| (b) 6 programs without a scene I sample layer | their SAMPLE layers sit in scene II, the harness plays scene I | harness scope | explained, s4 |
| (c) 18 programs never voice, 51 only at other keys | of the 18: 10 now voice at key 60 (member size), 6 are scene II only, 1 only at other keys, 1 none; of the 51: 50 now voice at key 60. Left: 3 single layer programs (B3 Old Forest, Grand Vibra Bs, Unclocked Pad), no cause found in the paths run | harness gap, 3 open | s4.3 |
| (d) per-loop `poke 0x92410 0x200` | replaced by `pcm ... credit=on` (the unpacker frame credit the ring pass adds to `rec+0x10`), output bit-identical | harness quirk removed | s3.1 |
| (d) prepare message first | it is the ARM order: reset, parameter words, **prepare**, then `0x92433`, `0x9243b`, pitch word, `0x92432`; the scenario drops only the reset and the unpacker words | not a quirk | s3.2 |
| (d) 96-frame refill-ahead | any value 24 to 120 gives the same output, bit-identical | irrelevant, now an option (`ahead=N`) | s3.3 |
| (e) Finger Bass f0 pair octave | the ARM picked the zone with root 49 (effective key 48), not key 60; f0 130.7 Hz against native 131.1 Hz (0 cents on the 25 cent grid) | harness reading error (D150 compared against key 60) | fixed, s5 |

No native-engine change: nothing here shows the native render wrong by a measured margin.

## 2. Level, stage by stage (3 programs, key 60, velocity 100)

`smp_stages` runs the native Synth layer path and prints the peak of the mono mean `(L + R) / 2` (what the oracle comparison measures) at four stages; the oracle numbers are the `sharc-ref` ring peaks of D150 (decoded PCM `x 2^-6` through the ARM one-pole), SY output mono.

| Program | native voice | native after FX | native after fader x program level | native Main out | oracle decoded PCM | oracle ring = SY out | gap voice stage | gap at output |
|---|---|---|---|---|---|---|---|---|
| Acoustic Guitar | -13.91 | -14.21 | -17.19 | -17.20 | -1.78 | -38.23 | 24.32 | 21.03 |
| Solo Flute | -14.24 | -14.72 | -14.72 | -14.72 | -2.11 | -38.44 | 24.20 | 23.72 |
| Finger Bass | -14.09 | -14.41 | -14.41 | -14.41 | -1.95 | -38.55 | 24.03 (ring -38.12: 24.46) | 24.14 |

dBFS, peak. Layer gain: Guitar 0.7098 (-3.0 dB, `(v/127)^2`), Flute and Finger Bass 1.0; program level 0 dB in all three.

1. The voice stage gap is 24.0 to 24.3 dB; the constant `2^-6 / 0.25` is 24.08 dB. Residual +0.24, +0.12, -0.05 dB (the native envelope and filter against the ARM one-pole; EMPIRICAL, three programs).
2. FX and fader move the native signal by at most 3.1 dB (Guitar: the layer gain), so they are not the dominant stage.
3. The oracle side stops at the SY resampler ring. The SY gain target (`rec+0x34`, slewed 0.979 per frame) is never sent in the scenario, the SY to core 2 receiver stage (`0x124C3A`, reads `DM(0xB2EF4 / 0xB2EF7)`, clip +-3.0) is not run, and the layer level and program level words are not applied. So the absolute oracle level is **not comparable** to the native level yet; the comparison says the oracle is a ring level, not that native is 24 dB too loud. Not found in the paths run: a measurement that puts native wrong.
4. Decision: harness omission. Closing it needs the SY gain target message and the receiver stage in the scenario (follow-up 1).

## 3. Harness quirks

### 3.1 Per-loop poke -> `credit=on`

The decoded frame credit `rec+0x10` grows by `+0x20` per unpacker block (listing `0x20A5`, `DM(0x95C68)`) and falls by 128 per ring pass; the waiting state tests `+0x10 + +0x1A > 0x52`. Without the credit the oracle traps at `pc 0x1c12dd` (unpacker). The old scenario poked `0x92410 = 0x200` once per loop. `pcm ... credit=on` adds the frames the `pcm` model wrote to the field itself (`CREDIT_FIELD = 0x10`), the same arithmetic as the unpacker. Output: bit-identical to the poke on the programs run (Guitar, Flute, Finger Bass). The scenario builder no longer emits pokes (`sample_stream_scn.build_pcm_scenario`).

### 3.2 Prepare first

In the ARM capture the SY path 1 order for a voice is: reset `0x92414 <- 0x1c0794`, `0x92416`, the coefficient pair `0x92420 / 0x92421`, the unpacker words `0x9240A..0x92413`, **prepare `0x92414 <- 0x1c07cd`**, then `0x92433`, `0x9243B`, the pair block `0x921A6`, `0x92432`. The scenario keeps the order of the kept words; it drops the reset (the oracle starts from reset state), `0x92416`, the unpacker words (replaced by the `pcm` ring) and carries `c0 / c1` into the ring model. "Prepare first" therefore equals the ARM order; D150 saw failures only when words that follow prepare in the ARM stream were put in front of it.

### 3.3 Refill-ahead

`pcm` keeps the ring `ahead` frames ahead of the read position (default 96, ASSUMED, ring 128). Runs with `ahead` 24, 40, 64, 96, 120 give bit-identical output. The default stays 96 and is now documented as not output relevant.

## 4. Layers and voices

### 4.1 Cause 1: the member was truncated (4 MiB)

`bind_samples` served the first `0x400000` bytes of one library member to every flagged layer. Samples whose zone for the played key lies beyond 4 MiB were not resident, so the voice did not start. `arm_dsp_capture.py` now has `MEMBER_BYTES = 0x400000` (EMPIRICAL default, kept so old captures reproduce) and `--member-bytes`; `tools/sample_gaps_cap.py` captures all 119 programs with 16 MiB (`0x1000000`, EMPIRICAL: the whole member is served up to that size; the three programs left without a voice have members of 1.0, 3.5 and 5.3 MB, so size is not their cause).

### 4.2 Coverage with the whole member (119 programs, scene I, keys 36 / 60 / 84)

| Class | D150 (4 MiB) | now |
|---|---|---|
| voice at key 60 (D150: any layer) | 50 | 110 (86 every scene I layer, 24 some: 22 with 3 layers, 2 with 2) |
| voices only at other keys | 51 | 2 |
| no voice queued | 18 | 1 |
| no scene I sample layer (scene II only) | | 6 |

Layers of scene I: 201; voices started at key 60 (capped at the layer count): 173. By layer count at key 60 (layers, voices): (1,1) 42, (1,0) 3, (2,2) 27, (2,1) 2, (3,2) 21, (3,3) 2, (3,1) 1; programs with more voices than layers also exist (17, up to 5 voices for 1 to 3 layers; unison or velocity layers, not traced).

### 4.3 What decides the rest

| Finding | Evidence |
|---|---|
| A voice starts per bound layer on the key down; the key enters through the zone the ARM picks from the member map (D150 s3) | captures, `0x92414` prepare per voice record `0x38` apart |
| Scene II layers do not start in the scene I harness | `sample-gaps-scenes.json`: 6 programs have all SAMPLE layers in scene II (e.g. Dark Matter Mk I, Marimba) |
| Keyboard zones act: a layer outside its zone gives no voice. Native and ARM counts match on 4 of 27 incomplete programs; native shows 3 layers sounding where the ARM shows 2 on 19 | `sample-gaps-zones.json` (`smp_stages`, `voice_db > -200 dB` at 36 / 60 / 84) |
| The ARM starts 2 voices for any 2 of the 3 bound layers and still 2 for 3 (Guitar Concerto, key 72, bound layers `{4}` 1 voice, `{0,2}` `{0,4}` `{2,4}` `{0,2,4}` 2 voices) | `tools/sample_gaps_layers.py --bind ...` (layer table indexes `0, 2, 4` = synth A, B, C, member Nylon Guitar 2, whole member); cause not found in the paths run |
| B3 Old Forest (3 voices at key 36, none at 60 / 84), Grand Vibra Bs (1 at key 36), Unclocked Pad (none): zones, velocity range and member size do not explain it (zones exist at key 60, members are 1.0 to 5.3 MB) | `smp_stream zones`; not found in the paths run |

The third layer of 3 layer programs stays unstarted in the ARM harness (21 of 26 programs: 2 voices). That is the largest remaining layer gap; native plays all three.

## 5. Finger Bass (e)

`smp_stream zones` lists the key map of the member (`key_low`, `key_high`, `root_key`, `rate_hz`). The ARM pitch word is `zone_word(zone_root, rate)` of the zone the ARM picked; the key does not enter it (D150). For Finger Bass the zone for key 60 has root 49, so the ARM played effective key 48, not 60. `sample_stream_audio.effective_key` now derives the key from the ARM word. Result: oracle f0 130.7 Hz, native 131.1 Hz (0 cents on the 25 cent grid), replacing the octave reading of D150 (493 / 131 Hz). The table entry `e` stays modelled; this does not test its writer.

## Implications

1. The 20 to 23 dB level gap is the missing SY receiver, gain target and layer words, not a native level error; native is **not** changed.
2. The 4 MiB member cap hid the voices of 60 programs (10 of the 18 never voiced, 50 of the 51 other-key only); 113 of 119 programs have a scene I layer, 110 of them voice at key 60, 86 with every layer.
3. A second voice per note stays capped at 2 in the harness for 3 layer programs; whether hardware does the same is not known (no hardware recording used).
4. No default flips, no `render_hash` change.

## Follow-ups

1. Send the SY gain target and run the SY to core 2 receiver stage (`0x124C3A`) in the scenario, apply layer level and program level, then compare absolute level against `smp_stages`.
2. Find why a third bound layer gets no voice (voice allocator, ack hook, unison); try binding three different members through the real loader.
3. B3 Old Forest, Grand Vibra Bs, Unclocked Pad: trace the missing voice start.
4. Score oracle audio (f0, centroid, level shape) for programs beyond the 10 of D150 now that 110 voice at key 60.
5. `go` trigger: the release mask reader is still undecoded (needs a live SY, D143 / D147).
