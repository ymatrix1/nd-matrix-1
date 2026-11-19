# Arpeggiator of A:16 Horizon Whl: ARM firmware against the native engine

- **Checked:** 2026-11-17
- **Sources (local, never committed):** factory program `Program/Bank A/Horizon Whl.ns4p` from `.reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b`; ARM image `target/fw_unpacked/bank0_CODE.bin` + `bank0_SRAM.bin` through the Unicorn harness (`tools/arm_arp_events.py`); Part A demo video `.reference/NEW NORD STAGE 4 - PROGRAMAS PRESETS DEMO (PART A) [_zksT69H30c].webm` (audio only, 48 kHz mono). Committed numbers: `docs/research/data/arp-*.json`.
- **Question:** The owner hears A:16 very far from the hardware video; synth layers A and C run the arpeggiator (rate raw 90 / 96, file BPM 266 / 318, global tempo 100). Does the native arpeggiator (`synth/arp.rs`, `arp_order.rs`, `arp_rate.rs`, D66) play the same notes at the same step and gate as the ARM firmware, and how do rate raw, file BPM, tempo, MST CLK, hold and KB sync combine?

Reproduce:

```bash
uv run --with unicorn python tools/arm_arp_events.py target/fw_unpacked/bank0_CODE.bin target/fw_unpacked/bank0_SRAM.bin \
    --eeprom target/oled/eeprom.bin --tempo 100 --out docs/research/data/arp-fw-events-a16.json "Horizon Whl.ns4p" 40 40,43,47 60 60,64,67
cargo run --release -p nord-tools --bin arp_trace -- "Horizon Whl.ns4p" --keys 60,64,67 --seconds 1.5 --tempo 100 [--arp-fw-walk]
python tools/arp_fw_compare.py docs/research/data/arp-fw-events-a16.json docs/research/data/arp-native-a16-60-64-67.json --layer C --keys 60,64,67
python tools/arp_video_onsets.py <horizon_285_345.wav> --t0 285 --window 287:300 --window 300:315 --out docs/research/data/arp-video-onsets.json
```

## 1. Short answer

| Item | ARM firmware | Native before D166 (= `--legacy-arp`) | Native with the firmware walk (default since D166; the gate row needs `--arp-fw-gate`) |
|---|---|---|---|
| Step (A and C, 100 BPM) | 150.0 ms (input, see s3) | 150.0 ms | 150.0 ms |
| Video step | 150.0 ms (comb fit 0.859) | | |
| Gate | **100 ms = 2/3 step**, note-off then next note-on 50 ms later | full step, off and next on at the same instant (1.0) | unchanged |
| Single held note, layer A (key 40) | note-on every step (40 x 10 in 1.5 s) | **one** note-on, no retrigger | unchanged |
| Layer C chord 60 64 67 (UpDown, range 2 octaves) | 64 67 72 76 79 84 79 76 72 67 ... (mirror over 7 pool notes, top root 84) | 60 64 67 **76 72 76** 60 ... (cycle 6) | 60 64 67 72 76 79 84 79 76 ... (same cycle as the ARM) |
| Layer C single key 60 | 72 84 72 60 72 84 72 60 (cycle 60 72 84 72) | 60 72 60 72 (cycle 2) | 60 72 84 72 (same cycle) |
| Layer A chord 40 43 47 (Up, 1 octave) | 43 47 40 43 47 40 (cycle 3) | 40 43 47 40 ... (cycle 3) | unchanged |
| First note after key-on | 149.9 ms (phase of the capture, see s4) | 150.0 ms | unchanged |

Only the gate and the retrigger of the plain arpeggio and the UpDown walk over several octaves differ; the step time agrees. The video shows the same 150 ms step.

## 2. How the controls combine (A:16)

| Control | A | C | Effect |
|---|---|---|---|
| MST CLK (`kArpClkEnable`) | on | on | the layer clock `0xC01E6368` and the dial rate `R[raw]` are unused; the pattern object is driven by the Master Clock position (`0xC023DE74`) |
| rate raw | 90 | 96 | `floor(9 * raw / 128)` = 6 for both: grid index 6 (`0xC02BDF60`, 32 points per 4 firmware beats, step = 2 points = 1/4 beat, "1/16") |
| file BPM (dial table) | 266 | 318 | irrelevant under MST CLK (the dial would give 30 / 266 = 112.8 ms and 30 / 318 = 94.3 ms) |
| global tempo | 100 | 100 | step = 15 / tempo s = 150.0 ms; the native engine copies the program tempo into `perf.master_bpm` (`sync_synth_clock`) |
| KB sync | off | off | no restart on key-on |
| HOLD | global bit, not decoded | | **not determined**: the release run (`arp-fw-events-a16-release.json`, notes 40 43 47) shows the player stepping to 1200 ms but the release time is not stored in the file; no claim |

The native engine reads the same four fields (`apply_arp_modes`: `arp_clock_sync`, `arp_division = floor(9 * raw / 128)`), so rate raw, file BPM and tempo give the same 150 ms as the ARM. D66's step law is not the cause of the A:16 difference.

## 3. ARM capture method and limits

- Timer counter `0x80001DC0` = 24 kHz ticks (D130). Events: the pattern player `0xC023B3EC` hands records to the Glue listener `0xC0123A18` (`[type 1 = on, 0 = off, index = cycle * 128 + record tick]`); note routines `0xC01E81DC` / `0xC01E80B8` give the voice list (`[obj+0x14]` count, `[obj+0x18]` notes).
- Under MST CLK the phase comes from a 24-bit position read over SPI (`0xC0011B60`). The harness has no device behind it (RAM pattern gives 1,500 events per second), so the run **overrides the read** at `0xC023DED8` and `0xC023DFD8` with `tempo / 60 * 2^19` units per second and restarts the master accumulators. **The step time is therefore an input of the capture, not a measurement.** Note order, gate fraction and the first-note latency are the firmware's own.
- Position scale `2^19` units per beat: ASSUMED (the only scale under which grid index 6 gives the 1/16 of the manual and the video, 150 ms at 100 BPM).
- The second voice array `[obj+0x1C]` holds the note number again, not a velocity (`vel` equals `notes` in every snapshot).

## 4. Numbers

Firmware (`arp-fw-events-a16.json`, velocity 100, 1.5 s per key set):

| Layer | Keys | notes_on | first_on_ms | step_ms | gate_ms | gate_fraction | notes (in playing order) |
|---|---|---|---|---|---|---|---|
| A | 40 | 10 | 149.88 | 150.0 | 100.0 | 0.667 | 40 x 10 |
| A | 40 43 47 | 10 | 149.88 | 150.0 | 100.0 | 0.667 | 43 47 40 43 47 40 43 47 40 43 |
| C | 60 | 10 | 149.88 | 150.0 | 100.0 | 0.667 | 72 84 72 60 72 84 72 60 72 84 |
| C | 60 64 67 | 10 | 149.88 | 150.0 | 100.0 | 0.667 | 64 67 72 76 79 84 79 76 72 67 |

Native default (`arp-native-a16-*.json`, comparison in `arp-cmp-*.json`): first on 150.0 ms, step 150.0 ms (spread 0.02 ms), gate 150.0 ms (1.0); single key A: one note-on. Chord C: 60 64 67 76 72 76 repeating. Single key C: 60 72 60 72.

The root note 60 of the cycle is **inferred, not observed**: the 1.5 s firmware lists show 64 67 72 76 79 84 79 76 72 67 and never the root; the pool is read from the mirror shape. The firmware starts on the second pool note after a key-on because the capture restarts the master 3 ticks before the key-on; the first record is the pseudo note-off at 99.88 ms (`index 0`), then the first note-on at 149.88 ms. The cycle (not the first note) is the comparable quantity. The phase of a real key-on against the master clock is not sampled (open).

Cause of the native chord order (`synth/arp.rs`): UpDown without a fractional range uses `n_idx` mirrored over the chord only and `oct_offset = step / count`, so the octave never mirrors: 60 64 67, then 64+12, 60+12, 64+12. The ARM mirrors over the whole pool (`count * octaves` notes plus the root of the top octave) without repeating the end points.

## 5. Video

- `tools/arp_video_onsets.py` on the mono 48 kHz audio of 285 to 345 s (never committed): spectral-flux onsets (387), autocorrelation peaks at **150, 300, 450, 600, 750 ms** in every window, comb fit **150.0 ms** (strength 0.859).
- Horizon Whl is on screen from **287 s**; the `synth_horizon` clip of `emu_vs_hw_segments.json` is 290 to 320 s. The arp period is audible in 287 to 345 s: windows 287-300 and 300-315 s (peak strength 0.77 / 0.75, mostly the arp on its own with the wheel), windows 315-330 and 330-345 s weaker (0.47 to 0.54, more material on top).
- 150.0 ms per step = 1/4 beat at 100 BPM, as in the ARM capture and the native engine. Tags: video EMPIRICAL (lossy audio, one take).

## 6. Opt-in fix (superseded by s7: default since D166, `--legacy-arp` opts out)

`SynthLayer::arp_firmware_walk` (was opt-in; `arp_trace --arp-fw-walk` is still accepted): UP/DOWN walks the whole pool (chord notes of every octave plus the root of the top octave) mirrored without repeating the end points. Matches the ARM cycle of layer C for the triad and the single key (tests `firmware_walk_*` in `synth/arp.rs`). Tag: **EMPIRICAL** (one program, one layer, range 2 octaves). The top-root rule is ASSUMED for other ranges (layer A shows no top root for Up at 1 octave; whether that is the direction or the range is not separated). Not wired into `FirmwareOptions` or the GUI yet (the struct is built literally in 17 places).

## 7. Direction, range and gate captures (2026-11-18, D166)

Question: do the gate (2/3) and the octave walk hold beyond the one A:16 setting of s4, so that the firmware arpeggiator can be the default?

Method: the original `.ns4p` bits of A:16 were patched (direction and RANGE of layers A and C; the `ns4 convert` JSON round trip is lossy and silenced the patched copies) and replayed with `tools/arm_arp_events.py` (tempo 100, MST CLK, velocity 100, 2.5 s). Layer A sounds for keys 40 43 47 and layer C for 60 64 67 (key zones). Result table: `docs/research/data/arp-fw-range.json` (30 rows: layer, keys, direction, raw RANGE, step, gate fraction, playing order; derived numbers only).

| Finding | Evidence |
|---|---|
| Gate fraction 0.667 and step 150.0 ms in **all 30 captures** (both layers, Up, Down, Up/Down, raw 0 to 127) | `docs/research/data/arp-fw-range.json`, `gate_fraction`, `step_ms` |
| Up, Down and Up/Down all walk the octave pool, not only Up/Down (D162 said Up/Down only) | Up C raw 47 triad: 64 67 72 76 79 84 60 ...; Down: 79 76 72 67 64 60 84 ... |
| Single key, pool size by raw: 0 gives 1, 16 and 32 give 2, 47 gives 3, 85 and 127 give 4 notes | rows `C [60]` |
| Pool = chord notes + 12 k whose offset from the lowest key is below a span; Up/Down mirrors without repeating the end points | all rows; unit tests `synth/arp_pool.rs` replay the 13 bracketed raw values |
| A straight span `12 + raw / 3` fails raw 7, 14 and 57; the fitted law is piecewise (`SPAN_KNOTS`) | brackets in `arp_pool.rs` |

Span brackets (semitones, pool holds offsets below the span): raw 0 and 7 (7, 12]; 14 (12, 15]; 16 (16, 19]; 28 and 32 (19, 24]; 42 (24, 27]; 47 (24, 28]; 57 and 63 (31, 36]; 95 and 126 (43, 48]; 127 above 36 and at most 48. The knots sit inside every bracket; between the captured raw values the law is interpolated (ASSUMED). The decoded step of the knob (4 semitones per detent) was not seen.

Gate source: constant 2/3 in every capture is the replayed boot grid (s1 and `arm-tick-timers.md` s6): the 30 captures are one observation of one replayed state, not 30 independent ones. Not seen on hardware; the flag writer is unknown (OQ32). So the gate is opt-in (`--arp-fw-gate`, D166) while the walk and the retrigger are the default.

Video envelope fold (the gate against the 150 ms comb of the Part A clip): inconclusive, the arp sits under the wheel and other material, so the clip cannot separate 1/2 from 2/3. It is not evidence either way.

Zig zag and Random keep the engine walk (no ARM capture); Poly and Gate modes do not use the walk.

## Implications

- The audible difference of A:16 is not the step time. It is the gate (full step against 2/3), the retrigger of a single held note, and the UpDown octave walk. The first two belong to OQ32 (the 2/3 boot grid); the third is `--arp-fw-walk`.
- The Master Clock position the ARM reads over SPI is not modelled; the tempo-to-ARM coupling (how global tempo 100 reaches the position scale) is open (OQ39). It may be the same counter as the PUMP clock word (OQ36).

## Follow-ups

- Done in D166: `arp_firmware` in `FirmwareOptions` and the GUI panel, gate 2/3 and single-note retrigger. Open: sample the key-on phase and KB sync on layers that use it, Zig zag and Random walks, a hardware clip of an arp note against its step (OQ32).
- Find the SPI position source or the `0xDA` tempo store to measure the ARM step instead of feeding it.
- HOLD: decode the global bit and run the release.
