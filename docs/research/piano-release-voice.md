# Piano release voice (SMP stroke 523): decode, key-up record, native delta

- **Checked:** 2026-11-22
- **Sources (local, never committed):** SMP image of OS v1.66 (`target/oracle/smp.bin`), Factory Restore RevK v1.66 `.ns4b`, `Program/Bank A/Astoria Grand.ns4p`; ARM + SMP live run `tools/arm_dsp_live.py ... --key-ticks 8000` (note-off at pass 8092); `tools/smp_voice_replay.py`; `ns4 render-patch` of five programs. Related: [smp-interrupt-rate](smp-interrupt-rate.md) s3, [smp-slew](smp-slew.md) s6, [piano-attack-fw](piano-attack-fw.md), [piano-extra-voices](piano-extra-voices.md).
- **Question:** what does the SMP play at key off (stroke 523 of Astoria Grand, voice 17, "order 4 page"), and how do its level, timing and shape differ from the native key-up sample?

## 1. Result

| Item | Result | Tag |
|---|---|---|
| Decoder row of the release stroke | Stroke 523 (kind 2 key-up, 46,359 frames at 35,000 Hz, 20 pages, 2 channels, `vel_low` 45, start history `[0, 0, 2, 4]`) uses predictor order 4. With the row `(-1, 4, -6, 4)` (FLAC fixed order 4, already in `smp_unpack::FIXED_LPC_COEFFS`) the live history ring of voice 17 equals `decode_page` of the bound file: counter 293 at pass 8300, 8 of 8 words equal (`smp_voice_replay.py --stroke 523 --voice 17 --passes 8300`, `MATCH`). The tool had rows 0 to 3 only | VERIFIED (run) |
| Native decode of the same stroke | `smp_stroke_dump` (the `nord-core` decoder) and the SMP agree on the ring above, so the native key-up sample equals the firmware sample | VERIFIED (run) |
| Voice start | Note-off writes the same step word as the note (`0x92529 = 0xCC5D4800`, 0.79829836), `F-2 = 0x3E800000` (0.25), `F15 = 0x3F7FFFFE`, `F17 = 0x3F7FF38E`, voice level `0x3F72EA88`, and the voice request `0x93CBE`; the note-on voices `0x93928`, `0x9395E` stay in the list but are reset by the writes `F11 = F12 = 0`, `F14 = 1.0`, `F16 = 0x3F7FBB6E` | VERIFIED (message list) |
| Record trajectory | The oracle record `0x93CBE` (read after each pass, one update per pass): `F15 = 0x3F7FC1C8` and `F0 = F1 = 0x1BF931EC` (0.2186 of full scale) at update 5, `F0 = 0x1A07C6A3` at update 384. The Rust `SlewRecord` started with `initial_record(F15, F17)` equals the oracle in `F15` (bit exact) and `F0` (within 1 LSB, the documented multiplier ties) at 6 of 6 sampled updates (`slew_voice::tests::key_up_record_equals_the_oracle_trajectory`) | VERIFIED (run) |
| Time constant | `2 / F / -ln F17 = 0.2426 s` at the DSP frame rate `F` (D211); native default 1.15 s | VERIFIED (arithmetic on the verified record) |
| Level against the note | Firmware gain of a record is `0.21875 * F15`: key-up `0.21875 * 1.0` (`0.922` at velocity 127), note `0.21875 * level` (Astoria Grand, default key velocity: `0.4719`, word `0x3EF1A1C0`). The key-up sample is `1 / level` times the note in the same units; native default plays `0.30 * level` (soft release 0.45). For Astoria Grand that is +6.5 dB firmware against -10.5 dB native: **17.0 dB** | VERIFIED (words) for the firmware, native constants ASSUMED |
| Release voice in the SMP tap | **Not measured.** The harness tap (`smp_tap.f32`) is silent from about frame 3,800 on in this run (held note, 8,000 ticks) although voices 0 and 1 keep decoding (counter 12,036 at pass 8399) and the record gain `F0` is 0.2186 at the key-up. A tap captured from pass 8088 on (`C` command) stays silent for 1,224 frames, so the level of the release in the tap is open | open |

## 2. Native against the firmware law, five programs

`ns4 render-patch ... --note 60 --velocity 100 --frames 120000 --hold 24000` (note off at 0.5 s), RMS in dB per window after the note-off, default engine / `--piano-keyup-fw`:

| Program | 0-20 ms | 20-100 ms | 100-300 ms | 0.3-1 s | 1-2 s |
|---|---|---|---|---|---|
| Astoria Grand | -30.4 / -24.6 | -32.2 / -20.3 | -43.4 / -33.1 | -64.1 / -57.8 | -84.7 / -79.5 |
| Felt Upright | -22.6 / -20.0 | -25.2 / -17.6 | -40.4 / -31.2 | -68.7 / -66.1 | -88.9 / -102.0 |
| Velvet Grand | -24.5 / -20.5 | -26.9 / -17.8 | -39.1 / -29.9 | -68.7 / -63.8 | -105.8 / -117.6 |
| Bubbles Mk I | -23.8 / -21.5 | -22.1 / -19.1 | -35.7 / -35.7 | -62.7 / -62.6 | -141.1 / -140.1 |
| Classic Wah Clav | no key-up stroke (D123): identical | | | | |

Reading: the firmware ratio is 0 to +11.8 dB louder in the first 300 ms (Bubbles Mk I only +2 to +3 dB in the first 100 ms, its key-up is already near the engine value) and decays 4.7 times faster (0.24 s against 1.15 s), so the tail is lower after 1 s on Felt Upright and Velvet Grand (-12 to -13 dB). EMPIRICAL (native renders; no hardware clip isolates the key-up).

## 3. Verdict and leads

- The law is VERIFIED on the oracle (words, decode, record trajectory); the unit mapping from the record gain to the engine amplitude (`amp = level` against `0.21875 * level`) is ASSUMED, so the opt-in flag keeps only the **ratio** key-up / note (`F15 / level`) and the decay. It does not flip any default (D22, D103: two clips disagree on the slew path).
- Ranked leads: (1) a hardware clip of one note off at low velocity to check the +17 dB ratio (a real key-up thump is quieter than the note, which argues against `1 / level`); (2) find why the harness tap is silent after frame 3,800 (voice list, output gate or FIFO) and capture the release in the tap; (3) the velocity law of `F15` between 100 and 127 (only 20, 64, 100, 127 sampled); (4) the pedal-down key-up placement (`0.979` records, [piano-extra-voices](piano-extra-voices.md)); (5) `layer_b` on a second core.

## Reproduce

```bash
python tools/arm_dsp_live.py ... --chip "SMP:0=smp=target/oracle/smp.bin" --piano-bind "$B" --key-ticks 8000 --log run --out run/out.json   # as smp-interrupt-rate
python tools/smp_voice_replay.py run target/oracle/smp.bin target/release/sharc-live --stroke 523 --voice 17 --passes 8300   # MATCH
cargo test -p nord-core --lib key_up
target/release/ns4 render-patch "Program/Bank A/Astoria Grand.ns4p" --backup "$B" --note 60 --velocity 100 --frames 120000 --hold 24000 [--piano-keyup-fw] --wav out.wav
```

## Implications

- The release voice item of OQ41 is closed for decode, record and timing; the level in the tap stays open.
- `--piano-keyup-fw` plays the firmware key-up ratio and decay on the default engine envelope path.

## Follow-ups

1. Hardware clip of an isolated note-off and a listening check of `--piano-keyup-fw` before any flip.
2. Silent tap after frame 3,800 in the live harness.
3. `F15` between velocity 100 and 127.
