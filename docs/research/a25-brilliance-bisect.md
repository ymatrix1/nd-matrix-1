# A:25 brilliance bisect: the other 8 dB is the SUPER LP24 type factor 4.4 of the synth layer, not a piano default

- **Checked:** 2026-11-22
- **Sources (local, never committed):** the Part A demo video and the factory backup `.ns4b` under `.reference/`; `tools/emu_vs_hw.py` ([emulator-vs-hardware-audio](emulator-vs-hardware-audio.md)) with the specs `data/emu_vs_hw_a25_bisect.json`, `_a25_super.json` (results next to them as `*_results.json`); the prior hunt [treble-excess-by-engine](treble-excess-by-engine.md) (D215) and [synth-super-lp24-clips](synth-super-lp24-clips.md) (D116). No firmware byte, sample or audio is printed or committed.
- **Question:** after `--fx-compressor-first` the A:25 clip still reads +14.2 dB at 3..8 kHz; `--legacy-all` with the FX parity ports and the compressor first reads +6.6. Which member of `FirmwareOptions::legacy_all` (or other non-FX default) carries the other 8 dB, and what velocity did the hardware clip use?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Item | Result | Tag |
|---|---|---|
| Responsible default | **`SynthChain::FirmwareStrip` against `Legacy`**, and inside it the SUPER wave 0 / LP24 cutoff type factor `SUPER_LP24_TYPE_SCALE = 4.4` (`synth/p_type.rs`). Of the 14 members of `legacy_all` bisected, only `--synth-chain legacy` moves the clip (+14.2 to +6.1 dB at 3..8 kHz); every other member is within 0.6 dB, except `synth_layer_law` (+3.1 dB the other way) | VERIFIED (run, s2) |
| The clip is **not piano only** | D215 s4 wrote "`synth_off` changes nothing" and "the synth layer is inaudible". Wrong: A:25 layer A of the synth is a SuperSaw on LP24 1600 Hz (level 0.61) and sounds. The edit `synth.layer_a.enabled = false` does not silence it (s5); `synth.layer_a.level = 0` does, and gives +6.0, the same as legacy chain | VERIFIED (run, s3) |
| Fix | the existing opt-in `--synth-super-lp24 <f>` (D113 / D116): factor 1.0 to 2.2 gives **+5.7 to +6.0 dB** (4.4: +14.2), score 4.92 to 3.16 .. 3.24. No new flag, no default flip (D22; the evidence is the same two clips as D116) | EMPIRICAL (one clip, two known) |
| Piano velocity (hardware reference) | the clip is not fixed by one velocity: with 2.0 the brilliance reads +5.8 at 100, **+2.3 at 88**, **-3.9 at 64**; piano_dry (D215) reads +3.0 / +1.4 / -8.5 at 100 / 88 / 64. A linear interpolation to 0 dB gives **79** (A:25) and **85** (piano_dry): the hardware notes sit near **80 to 85**, not 100 (single velocity per replay, two clips) | EMPIRICAL |
| Remaining after the fix | +5.7 to +6.0 dB at 3..8 kHz at velocity 100 equals the piano alone (`synth level 0`: +6.0); at velocity 88 it reads +2.3 | open (velocity, s6) |

## 2. The bisect (A:25, `--fx-compressor-first`, velocity 100, confirmed notes)

`tools/emu_vs_hw.py --confirmed-only`, emulator minus hardware, dB re RMS. Forward = default plus one legacy member; reverse = `--legacy-all --firmware-fx-ports parity --fx-compressor-first` (the +6.6 reference) plus the one firmware flag that re-enables the member. Full rows: `data/emu_vs_hw_a25_bisect_results.json`.

| Member of `legacy_all` | Forward flag | presence | brilliance | air | Reverse flag | brilliance |
|---|---|---|---|---|---|---|
| (none: default + comp first) | | +5.2 | **+14.2** | +3.7 | | |
| `piano_firmware_level` | `--legacy-piano-level` | +5.5 | +14.3 | +4.4 | `--firmware-piano-level` | +6.2 |
| `piano_touch` | `--legacy-piano-touch` | +5.2 | +14.2 | +3.7 | `--firmware-piano-touch` | +6.6 |
| `fx_rate_match` | `--legacy-fx-rate` | +5.2 | +14.2 | +3.7 | `--fx-rate-match` | +6.6 |
| `firmware_morph` | `--legacy-morph` | +5.2 | +14.2 | +3.7 | `--firmware-morph` | +6.6 |
| `firmware_routing` | `--legacy-routing` | +5.2 | +14.2 | +3.7 | `--firmware-routing` | +6.6 |
| `synth_velocity_fw` | `--legacy-synth-velocity` | +5.2 | +14.2 | +3.7 | `--firmware-synth-velocity` | +6.6 |
| `arp_firmware` (+ gate) | `--legacy-arp` | +5.2 | +14.2 | +3.7 | `--arp-firmware`, `--arp-fw-gate` | +6.6 |
| `measured_tick` | `--legacy-1khz-tick` | +5.2 | +14.2 | +3.7 | (no enabling flag) | |
| `synth_layer_law` | `--legacy-synth-layer-law` | +5.0 | **+17.3** | +5.4 | `--firmware-synth-layer-law` | +6.5 |
| `synth_stages` | `--no-synth-stages` | +5.3 | +13.6 | +4.2 | `--synth-stages` | +6.6 |
| `synth_knob_laws` | `--no-synth-knob-laws` | +5.2 | +14.2 | +3.7 | `--synth-knob-laws` | +6.6 |
| `sample_kernel_key` | `--sample-kernel ratio` | +5.2 | +14.2 | +3.7 | `--sample-kernel key` | +6.6 |
| **`synth_chain`** | **`--synth-chain legacy`** | +5.3 | **+6.1** | +1.4 | `--synth-chain strip` | **+6.4** |
| `fx` (`FirmwareFx::LEGACY`) | `--legacy-fx-ports --legacy-mod2` (no comp first) | +5.0 | +13.1 | +4.9 | `--firmware-fx-ports parity` (no comp first: +10.1, air +30.4) | +10.1 |
| all non-FX members at once | | +5.5 | +6.6 | +2.4 | `ref_legacy_all` | +6.6 |

Reading:

- **Exactly one member carries the 8 dB: the synth chain.** Forward it takes +14.2 to +6.1; reverse it puts +6.6 back to +6.4 (the 0.2 dB left is `synth_layer_law`, +6.5 alone). The piano members move it by 0.1 dB (`piano_firmware_level` +0.1 forward, -0.4 reverse). `legacy_all` with all non-FX members equals the reference (+6.6) to 0.1 dB.
- **The FX members do not carry it**: `FirmwareFx::LEGACY` reads +13.1 (no firmware compressor and no EQ port: the air band moves, the 3..8 kHz band by 0.9 dB).
- **`synth_layer_law` (the default firmware layer law) raises it** by 3.1 dB when turned off (+17.3): the opposite sign, so the default is closer to the hardware than the legacy law.
- Nine members are bit-identical to the default on this clip (no arpeggiator is audible, the morphs and rate words do not touch an audible parameter, the sample kernel is not used); they cannot carry anything.

## 3. Why a synth member moves a "piano only" clip

A:25 is `piano.layer_a` plus `synth.layer_a`: SuperSaw (SUPER wave 0), `Lp24` at 1600 Hz, resonance 0.10, level 0.61, unison 3, PUMP on Mod 1 ([synth-super-lp24-clips](synth-super-lp24-clips.md) s2 listed it as "SUPER LP24 1600 Hz res 0.1"). Variants on the clip (compressor first, 3..8 kHz brilliance):

| Variant | brilliance | presence | RMS gap |
|---|---|---|---|
| default (4.4) | +14.2 | +5.2 | +2.0 |
| `--synth-chain legacy` | +6.1 | +5.3 | +1.5 |
| synth layer level 0 | **+6.0** | +5.3 | +1.5 |
| synth layer level 0, chain legacy | +6.0 | +5.3 | +1.5 |
| piano off (strip chain, synth alone) | +11.8 | -8.2 | +3.5 |
| piano off, chain legacy (synth alone) | -13.0 | -20.1 | +3.1 |
| synth layer level 0, piano off | -16.8 | -20.8 | +3.1 (silence floor) |

The synth layer alone, on the strip chain, is 11.8 dB too bright at 3..8 kHz and 8.2 dB too quiet at 1.5..3 kHz ; on the legacy chain it is 13 dB too dark. The hardware clip sits between: the piano supplies the 1.5..3 kHz energy and the synth should add little above 3 kHz.

## 4. The type factor grid (A:25, compressor first, confirmed notes)

`--synth-super-lp24 <f>` replaces only the SUPER wave 0 / LP24 type factor of the cutoff (`p_type.rs`; default 4.4, the total with the row factor 0.45 is 1.98 against 0.9 at 2.0):

| factor | presence | brilliance | air | score |
|---|---|---|---|---|
| 4.4 (default) | +5.2 | +14.2 | +3.7 | 4.92 |
| 3.0 | +5.0 | +8.0 | +2.2 | 3.66 |
| 2.2 | +5.0 | +6.0 | +2.0 | 3.23 |
| **2.0** | +5.0 | **+5.8** | +1.9 | 3.19 |
| 1.5 | +5.0 | +5.7 | +2.0 | 3.16 |
| 1.0 | +5.1 | +5.8 | +2.4 | 3.24 |
| synth level 0 (no synth) | +5.3 | +6.0 | +1.4 | 3.27 |

The brilliance is flat (+5.7 to +6.0) for 1.0 to 2.2 and equal to the clip without the synth: below 2.2 the synth layer no longer contributes above 3 kHz, the remainder is the piano. The same range was the optimum of D116 (score 2.87 to 2.94 against 4.32 with the older compressor and notes); this run adds the explanation (the 8 dB excess is the synth layer) and puts the brilliance of the 4.4 default at +14.2 against +5.8.

## 5. Notes on the experiment

- **The `enabled` edit.** `emu_vs_hw.py` edits of `synth.layer_a.enabled = false` round-trip (`ns4 convert` keeps `false`), yet the render is identical to the default. The engine plays the layer anyway (cause not investigated; `engine/program.rs` has `needed = enabled || active_mask == 0 || ...`). Specs that need a silent synth layer use `synth.layer_a.level = 0.0`. [treble-excess-by-engine](treble-excess-by-engine.md) is corrected in the same PR. Follow-up 3.
- **Velocity.** The factory demo is a live recording; the hardware velocity is not in the audio. The only evidence is the brilliance of the clips against the replay velocity (single velocity per replay): A:25 with 2.0 (+5.8 / +2.3 / -3.9 at 100 / 88 / 64) and piano_dry (D215: +3.0 / +1.4 / -8.5). Zero crossings 79 and 85. EMPIRICAL: a replay velocity of 80 to 85 matches the hardware brightness of both clips; the transcribed velocities (Transkun) read +4.3 on piano_dry and are not better.

## 6. Implications

- **Row 23 of audio-remaining**: the 8 dB of A:25 is the SUPER LP24 type factor 4.4 of the synth layer (`--synth-super-lp24 2.0` removes it). What remains at 3..8 kHz is the piano at velocity 100 (+5.8) and equals the synth-off value; at velocity 88 it is +2.3.
- **D116 stays open**: 4.4 is now worse than 1.0 to 3.0 on two clips by 8 dB in brilliance, not only in score. The flip needs what D116 listed (trusted note list, wheel position, a hold-out clip). The wheel (+0.33 of the cutoff range morph) is the unread input: a raised wheel would make the hardware brighter and move the optimum factor up.
- **No default flips here.** Everything in this note is behind `--synth-super-lp24` and `--fx-compressor-first`, both opt-in.

## Follow-ups

1. Read the wheel position of A:25 from the OLED frames and replay it with the morph; re-run the factor grid.
2. A hold-out clip with a SUPER wave 0 LP24 layer for the factor (D116 criterion), then decide the 4.4 default.
3. Find why the `synth.layer_a.enabled = false` edit leaves the layer sounding (a bug in the loader or the edit path; check `engine/program.rs`, `active_mask`).
4. Velocity-calibrated replay of the piano clips at 80 to 85 (D137 follow-up 3).

Reproduce:

```bash
cargo build --release -p nord-core --bin ns4
uv run python tools/emu_vs_hw.py --spec docs/research/data/emu_vs_hw_a25_bisect.json --video <Part A .webm> --backup <factory .ns4b> --workdir <scratch> --confirmed-only --out docs/research/data/emu_vs_hw_a25_bisect_results.json
uv run python tools/emu_vs_hw.py --spec docs/research/data/emu_vs_hw_a25_super.json --video <Part A .webm> --backup <factory .ns4b> --workdir <scratch> --confirmed-only --out docs/research/data/emu_vs_hw_a25_super_results.json
cargo test -p nord-core --lib tests_super_lp24_a25
```
