# Treble excess by engine on current `main`: the common excess is gone, one stage error remains (the firmware compressor position)

- **Checked:** 2026-11-22
- **Sources (local, never committed):** the Part A demo video and the factory backup `.ns4b` under `.reference/`; `tools/emu_vs_hw.py` ([emulator-vs-hardware-audio](emulator-vs-hardware-audio.md)) on the six clips with the specs `data/emu_vs_hw_treble_excess.json`, `_a25.json`, `_a25_vel.json`, `_piano.json`, `_compfirst.json` (results next to them as `*_results.json`); `program_rank` over the 384 factory programs (`data/treble_by_engine_rank60.json`, `tools/rank_band_by_engine.py`); the prior hunt [brightness-downstream-oracle](brightness-downstream-oracle.md) (D137). Third octave figures come from the scratch WAVs of those runs. No firmware byte, sample or audio is printed or committed.
- **Question:** audio-remaining row 23 reported a common 3 to 5 dB treble excess above 1.5 to 3 kHz. Since then the synth envelope tables (D212, `--synth-fw-adr-law`), the piano `kEq` timbre gain (D209, `--firmware-piano-timbre`), the FX defaults (D203, `--fx-defaults-firmware`) and the Amp LP24 port (D188 / D193, `--firmware-fx-ports lp24`) landed. Which engine or stage carries an excess now, and is there an evidence-backed fix?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Item | Result | Tag |
|---|---|---|
| Common excess | **Gone on current `main`.** Of six hardware clips, three read above +2 dB at 3..8 kHz (piano_dry +3.0, Farfisa +6.9, A:25 +14.8), two are inside 2 dB (Vox +1.8, Smooth Lead +0.2) and the B3 clip reads 12.8 dB **dark** (Leslie, row 9). The "3 to 5 dB for every engine" of the 2026-11-11 tables no longer exists | VERIFIED (run, s2) |
| Native engine against the oracle chain | `program_rank` native default against the all-firmware render, 384 programs, key 60, velocity 100: the 2..20 kHz share differs by **+0.05 dB (piano, 77 programs), +0.20 (synth, 90), +0.07 (piano + synth, 173)** on average; organ programs read -0.4 (12) and the organ + piano or synth groups read 3 dB darker (Leslie, not brighter). **No engine's native treble is above its firmware reference on average** | VERIFIED (run, s3) |
| The opt-ins of the task | `--firmware-piano-timbre`, `--synth-fw-adr-law`, `--synth-oracle`, `--fx-defaults-firmware`, `--firmware-fx-ports lp24`: none lowers 3..8 kHz by more than 0.3 dB on any of the six clips (`--firmware-piano-timbre` and `--synth-fw-adr-law` -0.3 dB on A:25), and `--fx-defaults-firmware` is **worse**: +2.2 dB on A:25 and a worse score on Vox (2.25 to 3.29) and the Farfisa clip (3.23 to 3.90) | VERIFIED (run, s2) |
| Piano excess | piano_dry brilliance is **+3.0 dB at velocity 100, +1.4 at 88, -8.5 at 64** (confirmed notes); the transcribed velocities read +4.3. The excess is the velocity assumption of the replay, not a stage: the clip has no FX | VERIFIED (run, s3) |
| A:25 excess (+5.2 presence, +14.8 brilliance, +29.9 air) | The clip was read as **piano only** (`synth_off` changes nothing; **corrected by D221: the edit does not silence the synth layer, which is a SuperSaw LP24 1600 Hz and carries 8 dB of the brilliance, [a25-brilliance-bisect](a25-brilliance-bisect.md)**) through the Amp **LP24** and the layer compressor. It does not depend on velocity (brilliance +14.3 to +14.8 for velocity 56 to 88). **A broadband floor created by the firmware compressor** carries the +29.9 dB air and about +7 dB of the 3..8 kHz band: with `comp_enabled` off the air band reads +3.0, and with the compressor moved ahead of the amp (`--fx-compressor-first`, new, opt-in) it reads **+3.7** and the 8 to 16 kHz third octaves fall 14 to 22 dB, onto the engine-compressor curve | VERIFIED (run, s4) |
| Fix | `--fx-compressor-first` (`FirmwareFx::compressor_first`, D215): opt-in, default off, no default flips. One clip supports it, so the position is `EMPIRICAL` (one clip), the gain computer itself stays `VERIFIED` (oracle golden) | EMPIRICAL (one clip) |
| Remaining on A:25 | brilliance +14.2 (3..8 kHz) after the fix: the 3 to 6 kHz third octaves stay 10 to 16 dB above the recording; `--legacy-all` with the FX parity ports and the compressor first reads +6.6. The other 8 dB: the SUPER LP24 type factor 4.4 of the synth layer (D221, [a25-brilliance-bisect](a25-brilliance-bisect.md)) | closed by D221 |

## 2. The six clips on current `main`

`tools/emu_vs_hw.py --confirmed-only`, emulator minus hardware, dB re RMS (the air band is not scored: YouTube audio is low-passed). Flags per column: the five opt-ins of the task, then the all-firmware render. Full rows: `data/emu_vs_hw_treble_excess_results.json`.

| Clip (engine) | default pres / brill | `--firmware-piano-timbre` | `--synth-fw-adr-law` | `--synth-oracle` | `--fx-defaults-firmware` | `lp24` | `--firmware-all` |
|---|---|---|---|---|---|---|---|
| piano_dry (piano, no FX) | +2.2 / +3.0 | same | same | same | same | same | same |
| synth_lead (synth, LP24 filter) | -2.0 / +0.2 | same | -2.0 / +0.4 (score 0.69 to 0.79) | same | same | same | -1.8 / +0.4 |
| organ_b3 (B3) | -10.0 / -12.8 | same | same | same | -10.1 / -12.7 | same | -10.7 / -12.2 |
| organ_vox (Vox, Twin amp) | +0.1 / +1.8 | same | same | same | -1.4 / +2.1 (score 2.25 to 3.29) | same | -0.1 / +2.3 |
| fx_amp (Farfisa, Small II amp) | +2.1 / +6.9 | same | same | same | +3.2 / +5.9 (score 3.23 to 3.90) | same | +2.0 / +6.9 |
| A:25 (piano, LP24 amp, compressor) | +5.2 / +14.8 | +4.2 / +14.5 | +5.2 / +14.5 | +5.2 / +15.4 | +4.7 / +17.0 | +5.1 / +15.3 | +5.2 / +14.7 |

"same" = identical to the default to 0.1 dB: the opt-in does not touch the clip's engine. Reading:

- **No opt-in closes an excess.** `--synth-oracle` and `--synth-fw-adr-law` act on the synth layer and move only the synth clip (inside 2 dB already); `--firmware-piano-timbre` moves A:25 presence by -1.0 dB (the `kEq` timbre gain of the A:25 piano sound) and nothing else; the LP24 port equals the engine biquads in the treble (+0.5 dB brilliance).
- **The previous excess numbers fell.** The Farfisa clip went from +13.1 to +6.9 dB brilliance and piano_dry from +6.3 to +3.0 since D137: the firmware organ level and routing work, the FX rate match default (D180) and the piano velocity work moved them; the table of the earlier note is the pre-change state.
- **B3 is the opposite sign** (presence -10.0, brilliance -12.8 dB): the Leslie, audio-remaining row 9 and D190. It is not part of this hunt.

## 3. Per engine against the oracle chain

`program_rank` renders each factory program twice (engine defaults, then the firmware-derived options `--firmware-all --firmware-routing`, the oracle-verified model: [program-ranking](program-ranking.md)) for key 60, velocity 100. Programs are grouped by the engines enabled in either scene; band numbers are the power share in dB re total. Mean native minus firmware, `tools/rank_band_by_engine.py` on the 384 lines (`data/treble_by_engine_rank60.json`):

| Engine group | n | 1..2 kHz | 2..4 kHz | 4..8 kHz | 8..20 kHz | 2..20 kHz share, mean (worst) | programs over 1 dB |
|---|---|---|---|---|---|---|---|
| Piano | 77 | -0.06 | +0.05 | -0.05 | -0.24 | +0.05 (-2.26) | 8 |
| Synth | 90 | +0.14 | +0.19 | -0.17 | -0.09 | +0.20 (+23.70, Unclocked Pad) | 11 |
| Piano + synth | 173 | +0.16 | +0.12 | -0.41 | -0.43 | +0.07 (-7.16) | 26 |
| Organ | 12 | -0.81 | -0.57 | +0.06 | +0.42 | -0.44 (-7.21, B3 Chords n Solo) | 1 |
| Organ + synth | 10 | -0.28 | -3.29 | -2.42 | -1.62 | -3.22 (-16.63) | 4 |
| Organ + piano | 6 | -2.28 | -3.64 | -7.77 | -6.88 | -3.28 (-20.55, B3 Bassline) | 1 |
| Organ + piano + synth | 16 | +0.07 | -0.07 | -0.11 | 0.00 | -0.06 (-6.99) | 5 |

Reading: native default and the firmware render agree to within 0.2 dB on average for every group without an organ (the FX parity ports and the engine defaults were aligned by D173 to D187, so most of the firmware options are now defaults), and the groups with an organ read darker, not brighter. The one large positive outlier is Unclocked Pad (+23.7 dB, a HP24 synth program: the LP24 / HP24 default path, D193 flip proposal). **The common excess is therefore not a difference between an engine and the stage the oracle reproduces; what remains is the oracle model itself against the hardware.** This table cannot see an error that both renders share: that is what the clips of s2 and s4 are for.

## 4. A:25: where the treble sits (piano, LP24, compressor)

A:25 (Sidechain Whl II) has `piano.layer_a` on, `synth.layer_a` on and nothing else; at the confirmed notes the synth layer was read as inaudible (`synth_off` renders the same; wrong, D221: the `enabled` edit has no effect and the layer sounds). The piano amp is `LP24` (type word 424: mode 8, cutoff index from 2000 Hz, resonance 3) with a mid dip, the layer compressor is on at amount 0.50, the reverb is on, `eq_mid_freq` carries a wheel morph. Third octave level re total power, emulator minus hardware (third octaves of the scratch WAVs of `data/emu_vs_hw_treble_a25_results.json` and `_compfirst_results.json`; the hardware levels above 12 kHz are the YouTube low-pass):

| Third octave centre, Hz | 1587 | 2000 | 2520 | 3175 | 4000 | 5040 | 6350 | 8000 | 10079 | 12699 |
|---|---|---|---|---|---|---|---|---|---|---|
| hardware, dB | -18.7 | -27.3 | -36.8 | -48.8 | -58.8 | -67.4 | -77.4 | -81.0 | -81.1 | -83.3 |
| emulator default, minus hardware | +3.0 | +6.7 | +8.3 | +11.3 | +15.1 | +17.3 | +22.3 | +26.6 | +30.1 | +32.9 |
| compressor off, minus hardware | +3.1 | +6.7 | +8.0 | +10.2 | +13.5 | +14.5 | +15.6 | +12.0 | +9.4 | +10.4 |
| `--fx-compressor-first`, minus hardware | +3.0 | +6.8 | +8.2 | +10.8 | +14.1 | +15.2 | +16.3 | +12.7 | +10.0 | +11.0 |
| engine compressor (`--legacy-fx-ports`), minus hardware | +3.2 | +6.7 | +8.0 | +9.9 | +13.1 | +14.1 | +15.5 | +12.7 | +10.6 | +11.6 |

Findings:

1. **The default has a flat HF floor, the hardware has none.** The default render falls from -15.7 dB at 1.6 kHz to -55 dB at 6.3 kHz and then **stays at -54 to -51 dB up to 16 kHz**; the hardware keeps falling to -77 dB at 6.3 kHz and sits at -81 dB. Switching the layer compressor off (edit `comp_enabled`) or putting the engine peak compressor back (`--legacy-fx-ports`) removes it (air band +29.9 to +3.0 and +4.9 dB).
2. **The cause is the firmware compressor with the row words (D187), after the amp.** With the ASSUMED slope words (`--legacy-compressor-row`) the floor is gone (air +1.9) but the level is 8.05 dB off (against +1.9 dB with the row words), so the row words are right for level and the floor comes from the gain-step modulation of the gain computer (it multiplies four-frame blocks by a block-constant, smoothed gain; the steps land on a signal that is no longer band-limited). `FirmwareFx::compressor` has been default since D173 on the assumption that no hardware recording disagrees ([effects-and-routing](../specs/effects-and-routing.md) s4.0c said "no hardware recording to compare against"); A:25 is one.
3. **Moving the compressor ahead of the amp removes the floor** (`--fx-compressor-first`): the 8 to 16 kHz third octaves fall 14 to 22 dB, the air band reads +3.7, the RMS gap stays at +2.0 dB (+1.9 before) and the score goes 5.05 to 4.92. The unit test `tests_fw_comp_first.rs` shows the mechanism with a stepped 220 Hz tone through a 2 kHz LP24: less than half the high-frequency energy with the compressor first.
4. **The residual brilliance is not the floor.** After the fix the 3 to 6 kHz third octaves are still +11 to +16 dB above the hardware. The hardware falls about 10 dB per third octave from 2 kHz (24 dB per octave plus the resonance dip of the Amp); the emulator LP24, engine biquads or the verified firmware pair alike, falls about 8 dB (2.5 to 4 kHz: 11 against 8 dB per third octave). Both LP24 implementations read the same in the treble (`lp24`: +15.3 against +14.8), so the difference is not between them.

## 5. What is not explained, and why this stops here

| Question | Evidence | Next step |
|---|---|---|
| Other 8 dB of A:25 brilliance | `--legacy-all` with the FX parity ports and the compressor first reads +6.6 dB (score 3.56); the same without legacy reads +14.2. Of the individual `--legacy-*` flags only `--legacy-piano-level` moves it (+0.2), and `--legacy-piano-touch`, `--legacy-fx-rate`, `--legacy-morph`, `--legacy-routing` and `--sample-kernel ratio` are bit-identical to the default on this clip. The member of `FirmwareOptions::legacy_all` that carries the 8 dB is not found | bisect the remaining members (`measured_tick`, the `synth_*` group, `synth_layer_law`, `arp_*`) one at a time on A:25; the clip is piano only, so a synth member would be a surprise |
| Wheel position on A:25 | the cutoff has a wheel morph of +0.33 of the range; the hardware wheel position during the clip is unknown, and a raised wheel would make the hardware brighter, i.e. the real excess larger | read the wheel from the video OLED frames |
| Piano velocity | piano_dry brilliance goes -8.5 / +1.4 / +3.0 dB at velocity 64 / 88 / 100; the transcribed velocities (Transkun) give +4.3 dB; a velocity-calibrated replay is follow-up 3 of D137 | not a stage |
| Compressor position evidence | one clip (A:25). The A:25 hardware spectrum is the only observation of a compressor-on layer; no oracle run of the full chain with the compressor | an oracle run of a layer chain with comp and amp on noise: [fx-chain-install](fx-chain-install.md) rows give the install order of the units (slot roles are EMPIRICAL) |

## 6. Implications

- **Row 23 changes from "common excess, unknown stage" to three separate things:** piano velocity (replay assumption), the Farfisa source (+6.9 dB, a source spectrum not a stage: D137 s4.2), and A:25 (a stage position and an unlocated 8 dB).
- **`--fx-compressor-first` is a candidate for the owner after more clips:** it changes nothing on 5 of 6 clips (every band identical to 0.1 dB in `data/emu_vs_hw_treble_compfirst_results.json`), A:25 air +29.9 to +3.7, brilliance -0.6, RMS +0.1, score 5.05 to 4.92. No default flip is made: one clip.
- **D173's compressor default has a counter-observation** the owner may want to weigh: with the firmware compressor on and after the amp, a piano layer through a low-pass amp carries a floor 15 to 20 dB above the hardware at 5 to 16 kHz.

## Follow-ups

1. Bisect `legacy_all` members on A:25 for the 8 dB (s5).
2. Read the wheel position of A:25 from the clip and replay it with the morph.
3. A second compressor-on clip (any factory program with `comp_enabled` on an audible layer; list with `program_rank` census) to confirm or reject the position.
4. Compare the compressor and amp order against the oracle with a noise run of one layer chain (comp and amp on).
5. Velocity-calibrated replay of piano_dry (D137 follow-up 3).

Reproduce:

```bash
cargo build --release -p nord-core -p nord-tools
uv run python tools/emu_vs_hw.py --spec docs/research/data/emu_vs_hw_treble_excess.json --video <Part A .webm> --backup <factory .ns4b> --workdir <scratch> --confirmed-only --out docs/research/data/emu_vs_hw_treble_excess_results.json
uv run python tools/emu_vs_hw.py --spec docs/research/data/emu_vs_hw_treble_compfirst.json --video <Part A .webm> --backup <factory .ns4b> --workdir <scratch> --confirmed-only --out docs/research/data/emu_vs_hw_treble_compfirst_results.json
target/release/program_rank <factory .ns4b> --note 60 --jobs 4 > rank60.jsonl
uv run python tools/rank_band_by_engine.py rank60.jsonl --out docs/research/data/treble_by_engine_rank60.json
```
