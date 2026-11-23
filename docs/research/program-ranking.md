# Program ranking: where the native engine is farthest from the firmware paths (384 factory programs)

- **Checked:** 2026-11-17
- **Sources (local, never committed):** `Nord Stage 4 Factory Restore RevK v1.66.ns4b`, OS v1.66 `target/fw_unpacked/bank0_CODE.bin` and `bank0_SRAM.bin`, factory restore EEPROM, `target/PNO/core{1,2}_sram.bin`, `target/oracle/sy.bin`. Tools: `crates/nord-tools/src/bin/program_rank.rs` (new), `rank_metrics.rs` (new), `sweep_load.rs` (code moved out of `program_sweep.rs`), `tools/program_rank_chain.py` (new), `tools/program_rank.py` (new), `tools/arm_dsp_live.py` (D138, D144, D153), SAMPLE harness notes D143, D150, D153, D154. Data (numbers only, no audio, firmware or sample content): [program-ranking.csv](data/program-ranking.csv), [program-ranking-ablation.csv](data/program-ranking-ablation.csv).
- **Question:** for each of the 384 programs, one held note (key 60, velocity 100), how far is the native render from the firmware chain, which programs does the chain not voice or lose a layer on, and which cause carries the worst gaps?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by the commands in "Reproduce", **EMPIRICAL** = measured in this harness, **ASSUMED** = chosen convention.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Can the whole chain (ARM + SHARC oracle) render all 384 programs to audio? | **No.** The live link has one audio-like tap per engine and none for the piano: PNO core 1 lane `2:0` (B3 / organ), PNO core 2 voice window `0xC1F89` (synth voice), SY ring peak (SAMPLE, D150). Piano programs: since D216 the SMP chip tap carries the piano for 152 of the 155 programs that have a piano layer in scene I (the other 3: s3.3). FX, pan, layer mix and master are not in any tap. A per-program level and spectrum comparison against native is therefore only possible at taps that do not exist for most programs. | VERIFIED (runs, s3) |
| What is the ranked distance then? | **Native default against native with every oracle-verified firmware option** (`--firmware-all --firmware-routing`: firmware tone wheels, firmware filter engine, six FX ports). It measures how far the default path is from the firmware paths that are already verified against the oracle, not from the hardware. Score = `abs(level delta dB) + RMS of band share deltas dB + 6 x abs(centroid delta in octaves)`; median 3.9, p90 12.6, max 22.2; 61 programs above 10, 60 below 1. | EMPIRICAL (numbers), ASSUMED (weights) |
| What carries the worst 20? | **FX is the only group that moves them; one FX group does not always explain them.** Only single-group reverts were run. Reverting the best single FX group leaves a score of at most 2 in 7 programs, 2 to 5 in 7, and **above 5 in 6** (Melody Pad 11.4, PineScape Synth 10.9, Time Warp Saw 8.0, Crackle n Pd Whl 7.6, Bounce Clav 6.3, Roto EP Ondre J 5.0), so those need two or more FX groups together, which was not tested. Best-single-group carriers: Mod 2 12 programs, Mod 1 3, delay 2, EQ, reverb, compressor 1 each. Organ tone wheels and the synth filter engine reduce the score in none of the 20. | EMPIRICAL (ablation, s4) |
| Which programs does the chain not voice? | After the L2 wavetable forwarding, the firmware expectation and the arp hold (census rerun 2026-11-19, D170, [silent-programs](silent-programs.md)): **0 synth programs** (Synth Engine was a right-panned arp step that the left-lane window missed; it voices with the two-lane window, census rerun 2026-11-20, D175), **5 organ programs that the firmware itself keeps silent at key 60** (`organ_expected_silent`), **3 SAMPLE programs** with no voice prepare (B3 Old Forest, Grand Vibra Bs, Unclocked Pad, as D154). The first census (13 synth programs flagged) was a harness artefact: 12 of them now voice. | EMPIRICAL |
| Which programs lose a layer? | **24 SAMPLE programs** start fewer voices than they have scene I layers (22 have 3 layers and 2 voices, D154: the ARM starts 2 voices for 3 bound layers); native plays all layers. | EMPIRICAL |

## 1b. Re-run on the D163 to D166 defaults (2026-11-19)

The tables of s3 below are the first run (2026-11-17, before the Mod 2, Touch, arp walk and morph defaults). The committed CSV files are regenerated on the current defaults: the engine default is amp + firmware Mod 2 (D163), piano Touch (D165), arp walk and firmware morph (D166), against `--firmware-all --firmware-routing`. Same 384 renders (key 60, velocity 100), same chain columns as on `main` (`tools/program_rank_chain.py` is not touched).

| Statistic over 384 | 2026-11-17 | 2026-11-19 |
|---|---|---|
| mean score | 5.35 | 3.77 |
| median | 3.92 | 2.74 |
| p90 | 12.72 | 8.24 |
| max | 22.15 | 21.15 |
| programs above 5 / above 10 / below 1 | 162 / 61 / 60 | 104 / 24 / 80 |

New worst 20 and carrier (the one group whose revert to the engine default leaves the smallest score, `--ablate`; the fx_mod2 revert is a no-op now):

| # | Program | Engines | dLevel dB | dBands dB | dCentroid oct | Score | Carrier (score left when reverted) |
|---|---|---|---|---|---|---|---|
| 1 | Bank A/Bubbles Mk I | piano | -2.54 | 15.10 | +0.59 | 21.15 | fx_mod1 (0.27) |
| 2 | Bank E/PineScape Synth | synth | -12.29 | 6.99 | +0.29 | 21.03 | fx_delay (10.96) |
| 3 | Bank B/Clav LFO Wah Whl | piano | +2.16 | 15.14 | +0.15 | 18.21 | fx_mod1 (2.34) |
| 4 | Bank A/Classic Wah Clav | piano | -1.25 | 16.04 | -0.04 | 17.55 | fx_mod1 (0.39) |
| 5 | Bank E/Sine Rain Pad | synth | -5.13 | 11.45 | -0.01 | 16.62 | fx_reverb (0.01) |
| 6 | Bank C/Crackle n Pd Whl | piano+synth | -2.28 | 9.44 | +0.71 | 15.95 | fx_eq (3.30) |
| 7 | Bank C/TapePiano AT | piano+synth | +2.62 | 6.49 | -0.94 | 14.79 | fx_delay (0.00) |
| 8 | Bank F/DistantGrand | piano | -3.76 | 5.71 | -0.88 | 14.73 | fx_delay (4.97) |
| 9 | Bank A/Stereo Wah Clav | piano | +2.26 | 11.82 | -0.02 | 14.18 | fx_mod1 (1.26) |
| 10 | Bank C/Bounce Clav | piano+synth | +7.88 | 5.95 | -0.04 | 14.05 | fx_compressor (6.81) |
| 11 | Bank A/Jazz Guitar II | sample | +6.69 | 5.99 | -0.14 | 13.54 | fx_eq (3.33) |
| 12 | Bank F/Time Warp Saw | synth | -2.52 | 10.08 | +0.09 | 13.16 | fx_mod1 (6.45) |
| 13 | Bank E/24 Frames J3 | organ+piano+sample | +8.30 | 4.02 | +0.13 | 13.12 | fx_compressor (1.58) |
| 14 | Bank E/J3 Super Sawz | synth | -2.76 | 9.95 | +0.03 | 12.92 | fx_mod1 (3.77) |
| 15 | Bank E/B3 Squabble | organ | -0.18 | 11.27 | -0.21 | 12.72 | organ_tone (0.42) |
| 16 | Bank E/Wah EP Ondre J | piano | -2.13 | 7.58 | +0.45 | 12.42 | fx_mod1 (4.99) |
| 17 | Bank F/Synth Orchestra | synth | -2.26 | 9.45 | +0.01 | 11.75 | fx_mod1 (0.68) |
| 18 | Bank C/Gray Pad Whl | synth | -2.42 | 8.97 | +0.02 | 11.51 | fx_reverb (1.23) |
| 19 | Bank B/Fan Club Piano | piano | +9.28 | 0.88 | +0.18 | 11.25 | fx_compressor (2.77) |
| 20 | Bank E/RoomUpright Whl | piano | -2.79 | 3.76 | -0.70 | 10.78 | fx_delay (4.12) |

Carriers of the worst 20: **Mod 1 in 8** (Bubbles Mk I, Clav LFO Wah Whl, Classic Wah Clav, Stereo Wah Clav, Wah EP Ondre J, Time Warp Saw, Synth Orchestra, J3 Super Sawz; the Wah and A-WAH types among them), **delay in 4** (PineScape Synth, TapePiano AT, DistantGrand, RoomUpright Whl), compressor in 3 (Bounce Clav, 24 Frames J3, Fan Club Piano), EQ in 2 (Crackle n Pd Whl, Jazz Guitar II), reverb in 2 (Sine Rain Pad, Gray Pad Whl), organ tone wheels in 1 (B3 Squabble, 12.72 with 0.42 left). Mod 2 left the list: it carried 12 of the old 20. Combined revert sets (63 per program) are in [program-ranking-fx-combos.csv](data/program-ranking-fx-combos.csv) and [program-ranking-fx](program-ranking-fx.md) s6. `program-sweep.json` is regenerated too (`program_sweep`, both modes, notes 48 55 64 72).

## 2. Method

1. **Native** (`program_rank`, release build): every program of the backup is rendered twice for one note (key 60, velocity 100, held 100,000 frames, 131,072 frames analysed at 48 kHz, mono mean `(L + R) / 2`): with the engine defaults and with `FirmwareOptions::firmware_all()` plus `firmware_routing`. Per render: RMS and peak in dB, power-weighted spectral centroid (20 Hz to 20 kHz, Hann periodogram), power share of 8 bands (edges 20, 100, 250, 500, 1k, 2k, 4k, 8k, 20k Hz) in dB relative to the total. Definitions in `rank_metrics.rs` (ASSUMED conventions, identical for every program).
2. **Score** (`tools/program_rank.py`): `abs(d_level) + d_band + 6 abs(d_centroid_oct)`; `d_band` is the RMS of the band share differences over the bands that hold more than -50 dB in either render. The weight 6 dB per octave is ASSUMED (a convention to put three kinds of distance on one scale; the CSV keeps the three parts).
3. **Chain** (`tools/program_rank_chain.py`): per program the ARM firmware boots (500.2 M instructions), the program is recalled, key 60 (velocity 100) is held 1,500 ticks and released, 2,700 ticks in total (`arm_dsp_live.py`). Chips by the engines of scene I: PNO core 1 (`pno` preset, lane `2:0`) for organ and as the piano / none placeholder, PNO core 2 (`org` preset, voice window `0xC1F89:4:2`) for synth and SAMPLE programs, SY (`sy` preset, D153, `--sy-timeline`) for SAMPLE programs. ORG core 2 is not run for the new runs (D144: it never receives a key word; the first 100 runs carried it and it stayed silent). A second run with `--return-path` gives the SY ring peak for the 31 SAMPLE programs listed in the CSV (`sy_audio_peak` not empty).
4. **Flags** (`program_rank.py`): `organ_chain_silent` (organ in scene I, PNO core 1 lane peak 0), `synth_chain_silent` (synth in scene I, PNO core 2 window peak 0), `sample_no_voice` (SAMPLE layers, no SY voice prepare), `sample_layer_lost` (fewer prepares than scene I SAMPLE layers), `sample_voiced_no_audio` (prepare seen, return-path run has ring peak 0), `piano_no_chain_tap` (piano layer in scene I: no chip carries piano audio, so no verdict).
5. **Ablation** (`program_rank --ablate`, worst 20 only): the firmware options with one group put back to the default: organ tone wheels, synth filter engine, FX delay, compressor, reverb, EQ, Mod 1, Mod 2. The test `groups_cover_every_difference` checks that the eight groups are all the fields in which `firmware_all` + routing differs from the default. The carrier is the group whose revert leaves the smallest score.

## 3. Results

### 3.1 Distribution by engine (scene I, `data/program-ranking.csv`)



(`sample` = a synth layer plays a SAMPLES multisample; `synth` = analog oscillators.) By dominant term: bands 233 programs (median 5.2), level 130 (median 2.3), centroid 21 (median 2.9). The level delta of firmware options against the default spans -12.3 .. +9.5 dB (median +0.3). No program is silent in one render and not in the other.

### 3.2 Worst 20

| # | Program | Engines | dLevel dB | dBands dB | dCentroid oct | Score | Dominant | Carrier (score left when reverted) |
|---|---|---|---|---|---|---|---|---|
| 1 | Bank A/Bubbles Mk I | piano | -3.95 | 14.50 | +0.62 | 22.15 | bands | fx_mod1 (0.25) |
| 2 | Bank E/Babylon EP Whl | piano+sample | +3.79 | 17.49 | -0.12 | 22.01 | bands | fx_mod2 (3.92) |
| 3 | Bank B/Roto EP Ondre J | piano | +6.02 | 13.45 | +0.38 | 21.74 | bands | fx_mod2 (5.02) |
| 4 | Bank D/Melody Pad | synth | -6.04 | 10.37 | +0.86 | 21.54 | bands | fx_mod2 (11.37) |
| 5 | Bank E/Spin Wurly II | piano | +3.20 | 17.93 | +0.02 | 21.27 | bands | fx_mod2 (2.93) |
| 6 | Bank E/PineScape Synth | synth | -12.29 | 6.84 | +0.29 | 20.88 | level | fx_delay (10.85) |
| 7 | Bank C/Crackle n Pd Whl | piano+synth | -1.76 | 10.71 | +1.10 | 19.04 | bands | fx_eq (7.64) |
| 8 | Bank B/Logical Wurly | piano | +2.61 | 16.06 | +0.00 | 18.69 | bands | fx_mod2 (2.49) |
| 9 | Bank B/Clav LFO Wah Whl | piano | +2.16 | 15.14 | +0.15 | 18.21 | bands | fx_mod1 (2.34) |
| 10 | Bank F/Hip split | synth | -2.60 | 15.47 | +0.01 | 18.16 | bands | fx_mod2 (3.54) |
| 11 | Bank A/Classic Wah Clav | piano | -0.64 | 16.81 | -0.08 | 17.91 | bands | fx_mod1 (0.39) |
| 12 | Bank A/Spin EP Whl | piano | +1.25 | 14.81 | +0.22 | 17.40 | bands | fx_mod2 (2.05) |
| 13 | Bank F/Time Warp Saw | synth | +0.03 | 16.74 | +0.08 | 17.28 | bands | fx_mod2 (7.99) |
| 14 | Bank E/Sine Rain Pad | synth | -5.13 | 11.45 | -0.01 | 16.62 | bands | fx_reverb (0.01) |
| 15 | Bank A/Sine-Saw Plk Whl | synth | -1.03 | 14.42 | +0.00 | 15.45 | bands | fx_mod2 (0.80) |
| 16 | Bank F/Resonant Peaks | synth | +3.97 | 9.76 | +0.28 | 15.41 | bands | fx_mod2 (0.73) |
| 17 | Bank D/Space Bass | sample | +4.26 | 9.73 | -0.14 | 14.83 | bands | fx_mod2 (1.66) |
| 18 | Bank F/DistantGrand | piano | -3.59 | 5.84 | -0.89 | 14.77 | bands | fx_delay (4.52) |
| 19 | Bank C/Bounce Clav | piano+synth | +8.48 | 5.49 | +0.13 | 14.74 | level | fx_compressor (6.29) |
| 20 | Bank C/J3 Warm 80s Brs | synth | +2.13 | 12.01 | -0.09 | 14.66 | bands | fx_mod2 (1.51) |

In all twenty, only FX groups reduce the score when reverted (s4); for 6 of them a single group leaves more than 5. Their scores come from band shape (18 of 20 are dominated by `dBands`, 2 by level), not from level.

### 3.3 Chain flags

| Flag | Programs | Notes |
|---|---|---|
| `piano_no_chain_tap` | 0 | was 155. The SMP chain tap (D205, `program_rank_chain.py --smp`) was run over all 155 piano programs (D216, 2026-11-21): 152 give audio (`smp_peak_dbfs` -34.5 to -14.3, median -18.4 dBFS, post stage ring, layer A, before the chain; `smp_cycles` is the fundamental in cycles per frame, 0.00600 for 140 of them). The tap level is **not** a program level: it is layer A only, before FX, pan and master, and the SMP interrupt rate and layers 2 and 3 are open ([smp-voice-pitch](smp-voice-pitch.md)). |
| `piano_chain_no_audio` | 3 | Chord N Melody and Forever Split (piano key zone gain 0 at key 60: the firmware keeps the piano silent, `tools/program_expect.py`, expected) and Simple Tune (piano on layer B only; D230: it plays on piano core 2 and its voices write bus `0x9405A`, which the tapped ring does not carry; a bus tap is open). |
| `synth_chain_silent` | 0 | Synth Engine (a right-panned arp step, key word lane bit `0x01`) was the last one, read as silence by the old left-lane window; it voices with the two-lane window `0xC1F89:8:1` ([arp-pan-lanes](arp-pan-lanes.md), D175, census rerun 2026-11-20). The 12 other programs of the first census (Dance Lead, Gravity Pattern, 70s CP Strings, Broken Chip, Calling Whl, Super Org Punch, 90s Organ, Dreamy Syn Tines, Gravity Whl, Moving Chords, Scream Lead AT, Shiny Synth Keys) voice after the L2 forwarding and the arp hold (D170). |
| `organ_expected_silent` | 5 | Ballad Layer Whl, Wormhole Organ (Pipe 1), Nox Split (Pipe 2), 24 Frames J3, J3 Grand Layers: the firmware keeps the organ layer silent at key 60 (zone gain 0 or stored level 0; `tools/program_expect.py`). The other 33 organ programs voice. |
| `sample_layer_lost` | 24 | 22 with 3 layers (2 voices), Momentum Whl (2 layers, 1 voice), Quintet Bass Spl and Pizz Quintet Spl (1 voice). Same cause as D154 section 4.3: the third layer gets no voice in the harness. |
| `sample_no_voice` | 3 | B3 Old Forest, Grand Vibra Bs, Unclocked Pad (D154 follow-up 3). |
| `sample_voiced_no_audio` | 19 | of the 31 programs with a return-path run, 19 prepare a voice and the SY ring peak is 0 (the desync of D112 / D150); the 12 with audio all read 0.05394 (5 digits, identical across programs): that is the stand-in sample bound by `--ns4b`, **not** a program level. The SY ring peak carries no per-program level information. |

### 3.3a SMP chain-tap census (D216)

Run 2026-11-21 on the 155 programs of the former `piano_no_chain_tap` flag: `program_rank_chain.py --smp target/oracle/smp.bin` (key 60 velocity 100, held 3,000 ticks, `--piano-bind`, about 3 min of CPU per program, 8 workers, 8 batches of 20), then `tools/program_rank_smp.py` merged `smp_peak_dbfs`, `smp_cycles` and the flags into `data/program-ranking.csv` (the other columns are unchanged: the native renders were not repeated, no default changed).

| Outcome | Programs | Why |
|---|---|---|
| tap with audio | 152 (83 piano, 42 piano+synth, 22 piano+sample, 3 organ+piano+sample, 2 organ+piano+synth) | peak -34.5 to -14.3 dBFS, median -18.4 |
| no audio | 3 | Chord N Melody, Forever Split: piano zone gain 0 at key 60 (expected); Simple Tune: piano layer B only, plays on core 2 but its bus is not in the ring (D230) |
| no tap | 0 | 5 programs (Tines Layer, LoFi Piano, Rising Sun, Piano Soft Orch, 70s CP Strings) store both piano enable bits 0 and sound on layer A; the bind read "no layer" until it used the loader rule (layer A, `engine/routing.rs::default_layer_a`) |

`smp_cycles` (fundamental, cycles per frame) is 0.00600 for 140 programs (the same key 60). Nine outliers (D230, [piano-smp-pitch-outliers](piano-smp-pitch-outliers.md)): four are real layer octaves (x2: Momentum Whl, Shoreline Whl, Big Felt Pad Whl, piano layer A octave +1; x0.5: J3 Split Special, octave -1), five were a failing pitch detector and are corrected in the CSV (French Harpsi, Gradient Whl, LoFi Piano, Clean Marimba AT, DistantGrand: 0.00602 to 0.00607). `tools/smp_pitch_audit.py` checks the column against `2^octave` of the played layer: 148 `ok`, 4 `octave_shift`, 0 `mismatch`. Simple Tune (piano layer B only) plays on piano core 2; its voices write bus `0x9405A`, which the tapped ring does not carry, so it stays `piano_chain_no_audio`.

## 4. Cause of the worst 20 (ablation)

| Carrier | Programs (of 20) | Reading |
|---|---|---|
| `fx_mod2` | 12 | Mod 2 firmware port (PHAS, FLANG, CHOR, VIBE, ENS, SPIN) against the engine Mod 2 |
| `fx_mod1` | 3 | Mod 1 firmware A-PAN / TREM against the engine |
| `fx_delay` | 2 | delay laws and delay time |
| `fx_eq`, `fx_reverb`, `fx_compressor` | 1 each | |
| `organ_tone`, `synth_filter` | 0 | never reduce the score |

Carrier = best single-group revert; **only single-group reverts were run**, no pairs. Residual after the best revert: at most 2 in 7 programs, 2 to 5 in 7, above 5 in 6 (largest: Melody Pad 11.4 of 21.5, PineScape Synth 10.9 of 20.9, Time Warp Saw 8.0 of 17.3). Per program values are in [program-ranking-ablation.csv](data/program-ranking-ablation.csv) (score left when each group is reverted). The Mod 2 default (engine biquad / LFO models, D95 `ENS` ASSUMED taps) is the single largest distance to the oracle-verified path in the factory set. Not shown: the carrier of programs ranked 21 to 384 (not ablated).

## 5. What this does not say

1. **Not hardware.** The reference of the score is the firmware-derived native path, which is verified against the oracle per stage, not against recordings. A program near 0 can still be far from the hardware (organ level K, D113; reverb tails).
2. **Chain audio per engine is missing** for the piano (no tap), for FX output and for the master. The ranking cannot see errors common to default and firmware paths (the piano engine, the organ K level, SAMPLE pitch entry `e`).
3. The chain census is one key (60), one velocity, 1,500 ticks held (15,000 for arp programs). Peak columns are empty where the chip of that column was not run for the program's engines (`chips_for`): a number means run, `0.0` means silent.
4. Score weights are ASSUMED; the CSV keeps level, band and centroid parts apart.

## Implications

- The largest distance to the oracle-verified paths in the factory set sits in **FX, not in the engines**: Mod 2 is the best single carrier of 12 of the worst 20, then Mod 1, delay; for 6 of the 20 a pair of FX groups is needed to explain it. Each default flip stays with the owner (D22, D136).
- The oracle chain needs a **piano tap** and the 18 flagged organ and synth programs diagnosed before a per-program chain audio comparison is possible.
- The SY ring peak must not be used as a level (stand-in sample).

## Follow-ups

1. ~~Ablate pairs of FX groups for the 6 programs with a residual above 5~~ Done in [program-ranking-fx](program-ranking-fx.md) (D163): all 63 FX combinations for the worst 20 (Mod 2 plus EQ explain most; the six residual programs need three or four groups), and the before / after of an FX default flip on all 384. Still open: programs 21 to 384 have no carrier (`--ablate-fx`, about 15 s per program with `--jobs 20`).
2. ~~Diagnose the 13 `synth_chain_silent` and 5 `organ_chain_silent` programs~~ done in [silent-programs](silent-programs.md) (D164, D170): census rerun, Synth Engine answered in [arp-pan-lanes](arp-pan-lanes.md) (D175).
3. ~~A piano tap~~ Done as the SMP chip tap (D205, census D216): 152 of 155 piano programs. Open: the tap is layer A before the chain, so it carries no level verdict against native; layer B bus tap (Simple Tune, D230); `smp_cycles` outliers answered (s3.3a).
4. Re-run after any FX default flip: `tools/program_rank.py` regenerates both CSV files (done 2026-11-19, s1b; six per-bank `program_rank --only "Bank X/"` shards give the same lines as one run).

## Refactor check (`program_sweep` -> `sweep_load.rs`)

`program_sweep` was built from `origin/main` before the change and from this branch after it; `program_sweep <backup> --limit 8 --every 48 --frames 24000` (8 programs: every 48th, both modes, full decoded program JSON) gives byte-identical stdout, md5 `70bfe03dbba1b2f0ac4267cc9de17a0f` on both (VERIFIED, 2026-11-17).

## Reproduce

```bash
cargo build --release -p nord-tools --bin program_rank -p nord-sharc --bin sharc-live
M=.   # main checkout with target/, .reference/, factory_programs/
B="$M/.reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b"
target/release/program_rank "$B" > native.jsonl                      # about 70 min, 14 GB RAM
target/release/program_rank "$B" --ablate --only "Bank A/Bubbles Mk I.ns4p" >> ablate.jsonl
PYTHONPATH=tools uv run --with unicorn --with capstone python tools/program_rank_chain.py \
  --programs $M/factory_programs/Program --out chain --workers 14 --native native.jsonl \
  --fw-dir $M/target/fw_unpacked --eeprom $M/target/oled/eeprom.bin --backup "$B" \
  --pno1 $M/target/PNO/core1_sram.bin --pno2 $M/target/PNO/core2_sram.bin \
  --sy $M/target/oracle/sy.bin --live target/release/sharc-live --audio-for audio.txt
python tools/program_rank.py --native native.jsonl --chain chain/chain.jsonl \
  --csv docs/research/data/program-ranking.csv --ablate ablate.jsonl \
  --cause-csv docs/research/data/program-ranking-ablation.csv --md-tables
# SMP chain tap of the piano programs only (D216; no native renders): add `--smp $M/target/oracle/smp.bin --entries piano.txt`
# to the chain command above (`--native` may be a file of engines only), then merge into the committed CSV:
python tools/program_rank_smp.py --chain chain/chain.jsonl --csv docs/research/data/program-ranking.csv
uv run pytest tools/tests/test_program_rank.py tools/tests/test_program_rank_chain.py
cargo test --release -p nord-tools
```
