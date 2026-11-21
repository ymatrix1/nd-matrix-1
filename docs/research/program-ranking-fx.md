# Program ranking, FX combinations: which FX groups carry the gap, and what the firmware path buys

- **Checked:** 2026-11-18
- **Sources (local, never committed):** `Nord Stage 4 Factory Restore RevK v1.66.ns4b` (all 384 programs, key 60, velocity 100, same renders as [program-ranking](program-ranking.md)). Tools: `program_rank --ablate-fx` and `--flip` (new), `tools/program_rank_combo.py` (new), `nord_tools::fx_parity` and `fx_parity_null` (new); the oracle nulls of the ports are D158 / [fx-vs-oracle](fx-vs-oracle.md). Data (numbers only, no audio, firmware or sample content): [program-ranking-fx-combos.csv](data/program-ranking-fx-combos.csv), [program-ranking-fx-flips-worst20.csv](data/program-ranking-fx-flips-worst20.csv), [program-ranking-fx-flips-384.csv](data/program-ranking-fx-flips-384.csv).
- **Question:** the single-group ablation of [program-ranking](program-ranking.md) left 6 of the worst 20 programs above a score of 5. Which combination of FX groups explains each gap, and how far does the ranked distance fall when the verified firmware FX path is switched on for that combination?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by the commands in "Reproduce", **EMPIRICAL** = measured in this harness, **ASSUMED** = chosen convention. Score = `abs(level delta dB) + RMS of band share deltas dB + 6 x abs(centroid delta in octaves)` as in [program-ranking](program-ranking.md); it is the distance of one render to another, **not** to the hardware.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Which combination explains the worst 20? | Mod 2 is in the smallest explaining set (score left at most 2) of 14 of 20 programs, EQ in 10, compressor in 7, Mod 1 in 6, delay and reverb in 2 each. 7 programs need one group, 7 a pair, 6 three or more. The partner of Mod 2 is **EQ** (best pair for 8 programs; `fx_eq+fx_mod2` takes Melody Pad 21.5 to 1.6, Babylon EP Whl 22.0 to 1.5, Logical Wurly 18.7 to 0.6). A single-group revert hid it: Mod 2 and EQ each carry about half of such a score and the score is not additive. | EMPIRICAL (63 renders per program) |
| The six programs single reverts left above 5 | Melody Pad (11.4): EQ + Mod 2 (1.6). PineScape Synth (10.9): delay + compressor + EQ (1.2). Time Warp Saw (8.0): compressor + EQ + Mod 1 + Mod 2 (1.8). Crackle n Pd Whl (7.6): compressor + EQ + Mod 2 (0.8). Bounce Clav (6.3): compressor + EQ + Mod 1 + Mod 2 (0.5). Roto EP Ondre J (5.0): compressor + EQ + Mod 2 (1.3). | EMPIRICAL |
| Is it only FX? | Reverting all six FX groups leaves 0.00 in all 20: organ tone wheels and the synth filter engine move none of them. | EMPIRICAL |
| What does the firmware FX path buy? | Switching on every port but reverb (`FirmwareFx::PARITY`: delay laws and time, compressor, EQ, Mod 1, Mod 2) takes the **worst 20 from mean 18.14 to 2.19** (median 18.03 to 0.98, 2 of 20 above 5) and **all 384 programs from mean 5.35 to 1.66** (median 3.92 to 1.04, p90 12.72 to 3.84, max 22.15 to 16.61; programs above 5: 162 to 22; above 10: 61 to 3). 251 programs improve by more than 0.5, 127 stay within 0.5, 6 rise by more than 0.5 (largest +1.87, J3 80 Underscore 2.91 to 4.78). | EMPIRICAL |
| Does the firmware path null against the oracle? | The stages do: every port that runs on the chip matches the oracle (D158: level delta 0.00 dB, null -72 dB or better; golden vectors per routine). The new test `nord_tools::fx_parity` shows the PARITY chain nulls against the single-port firmware model for every Mod 1, Mod 2, EQ and compressor case (-314 dB or better, compressor -86 dB) while the engine path does not (Mod 1 -10.7 to +3.7 dB, Mod 2 -7.3 to +3.1 dB at amounts 64 and 127, EQ -27.4 to -7.7 dB, compressor -15.1 to -2.9 dB). The PARITY null is the same stage code, so it proves the wiring, not the stage. | VERIFIED (wiring); stages VERIFIED by D158 |
| Default flipped? | **Mod 2 only** (D163, owner approval relayed 2026-11-18): `FirmwareFx::DEFAULT` is amp + Mod 2, `--legacy-mod2` (and `--legacy-all`) turn it off. It is the one family whose live FX-chip null covers all 12 types (null -72 dB or better, level delta 0.00 dB, D158) and the default chain nulls against the port for all 36 Mod 2 cases (`nord_tools::fx_parity`; the legacy chain does not). All 384 programs, score before / after, against the all-firmware render: mean 5.35 to 3.78, median 3.92 to 2.76, p90 12.72 to 8.24, above 5 162 to 106, above 10 61 to 24; 111 programs improve by more than 0.5, 15 get worse (s3.4). `render_hash` unchanged (`eb13063d4fca837a613670168a005b564c7f235a1578136d03955fa968d2ca28`) but **not exercised by the hash scenario**: the golden FX sweep sets Mod 2 by type without the raw words, so it stays on the engine path and cannot show the flip (test `engine::tests_mod2_default` does). EQ, Mod 1, compressor, delay and reverb stay opt-in (s4). | VERIFIED |

## 2. Method

1. **Combined ablation** (`program_rank --ablate-fx`, worst 20 only): the firmware options (`--firmware-all --firmware-routing`) with every non-empty combination of the six FX groups (delay, compressor, reverb, EQ, Mod 1, Mod 2; 63 sets) put back to the engine default, one render each (key 60, velocity 100, held 100,000 frames, 131,072 analysed, mono mean). `tools/program_rank_combo.py` scores each against the default render and derives per program the best single group, the best pair, the greedy chain (add the group that lowers the score most), the exact best set (all 63 are rendered, so no greedy approximation is needed), the floor (all six reverted) and the smallest set whose score is at most 2 (ASSUMED threshold: the ranking median is 3.9 and 60 of 384 programs are below 1). Ties go to the set with fewer groups, then to the name order, so a pair partner a program does not use (reverb in a dry program) can show a score equal to the single group.
2. **Flip** (`program_rank --flip SETS`): the *engine default* with a set of groups switched to the firmware value, which is what a default flip renders, scored against the firmware render. It is not the same render as the revert of 1 (the revert changes the firmware side), so the two tables differ in places.
3. **Null test** (`nord_tools::fx_parity`, `fx_parity_null`): per FX case of `fx_cases` the engine chain, the chain with the single port on (the firmware model, oracle checked) and the PARITY chain on the same noise; null = `10 log10(sum (a - b)^2 / sum b^2)` after the best alignment within 24 frames (each stage adds 2 to 4 frames of latency, so a chain with more ports on is the same model plus a fixed delay).

## 3. Results

### 3.1 Which FX groups explain the worst 20

Score left when the set is reverted to the default (smaller = the set explains more), from [program-ranking-fx-combos.csv](data/program-ranking-fx-combos.csv):

| # | Program | Score | Best single | Best pair | Best set | Floor (all FX) | Smallest set <= 2 |
|---|---|---|---|---|---|---|---|
| 1 | Bank A/Bubbles Mk I | 22.15 | fx_mod1 (0.25) | fx_reverb+fx_mod1 (0.00) | fx_compressor+fx_reverb+fx_eq+fx_mod1 (0.00) | 0.0 | fx_mod1 (0.25) |
| 2 | Bank E/Babylon EP Whl | 22.01 | fx_mod2 (3.92) | fx_eq+fx_mod2 (1.51) | fx_compressor+fx_reverb+fx_eq+fx_mod2 (0.00) | 0.0 | fx_eq+fx_mod2 (1.51) |
| 3 | Bank B/Roto EP Ondre J | 21.74 | fx_mod2 (5.02) | fx_eq+fx_mod2 (2.66) | fx_compressor+fx_reverb+fx_eq+fx_mod2 (0.00) | 0.0 | fx_compressor+fx_eq+fx_mod2 (1.34) |
| 4 | Bank D/Melody Pad | 21.54 | fx_mod2 (11.37) | fx_eq+fx_mod2 (1.60) | fx_delay+fx_compressor+fx_reverb+fx_eq+fx_mod1+fx_mod2 (0.00) | 0.0 | fx_eq+fx_mod2 (1.60) |
| 5 | Bank E/Spin Wurly II | 21.27 | fx_mod2 (2.93) | fx_compressor+fx_mod2 (1.24) | fx_delay+fx_compressor+fx_reverb+fx_eq+fx_mod2 (0.00) | 0.0 | fx_compressor+fx_mod2 (1.24) |
| 6 | Bank E/PineScape Synth | 20.87 | fx_delay (10.85) | fx_delay+fx_eq (2.84) | fx_delay+fx_compressor+fx_reverb+fx_eq+fx_mod1+fx_mod2 (0.00) | 0.0 | fx_delay+fx_compressor+fx_eq (1.22) |
| 7 | Bank C/Crackle n Pd Whl | 19.04 | fx_eq (7.64) | fx_eq+fx_mod2 (2.81) | fx_compressor+fx_reverb+fx_eq+fx_mod1+fx_mod2 (0.00) | 0.0 | fx_compressor+fx_eq+fx_mod2 (0.83) |
| 8 | Bank B/Logical Wurly | 18.69 | fx_mod2 (2.49) | fx_eq+fx_mod2 (0.60) | fx_compressor+fx_reverb+fx_eq+fx_mod2 (0.00) | 0.0 | fx_eq+fx_mod2 (0.60) |
| 9 | Bank B/Clav LFO Wah Whl | 18.21 | fx_mod1 (2.34) | fx_compressor+fx_mod1 (0.00) | fx_compressor+fx_eq+fx_mod1 (0.00) | 0.0 | fx_compressor+fx_mod1 (0.00) |
| 10 | Bank F/Hip split | 18.15 | fx_mod2 (3.54) | fx_mod1+fx_mod2 (0.89) | fx_compressor+fx_reverb+fx_eq+fx_mod1+fx_mod2 (0.00) | 0.0 | fx_mod1+fx_mod2 (0.89) |
| 11 | Bank A/Classic Wah Clav | 17.91 | fx_mod1 (0.39) | fx_reverb+fx_mod1 (0.00) | fx_compressor+fx_reverb+fx_eq+fx_mod1 (0.00) | 0.0 | fx_mod1 (0.39) |
| 12 | Bank A/Spin EP Whl | 17.4 | fx_mod2 (2.05) | fx_eq+fx_mod2 (0.71) | fx_compressor+fx_reverb+fx_eq+fx_mod2 (0.00) | 0.0 | fx_eq+fx_mod2 (0.71) |
| 13 | Bank F/Time Warp Saw | 17.28 | fx_mod2 (7.99) | fx_eq+fx_mod2 (5.65) | fx_delay+fx_compressor+fx_reverb+fx_eq+fx_mod1+fx_mod2 (0.00) | 0.0 | fx_compressor+fx_eq+fx_mod1+fx_mod2 (1.82) |
| 14 | Bank E/Sine Rain Pad | 16.62 | fx_reverb (0.01) | fx_compressor+fx_reverb (0.00) | fx_compressor+fx_reverb+fx_eq (0.00) | 0.0 | fx_reverb (0.01) |
| 15 | Bank A/Sine-Saw Plk Whl | 15.45 | fx_mod2 (0.80) | fx_reverb+fx_mod2 (0.00) | fx_compressor+fx_reverb+fx_eq+fx_mod2 (0.00) | 0.0 | fx_mod2 (0.80) |
| 16 | Bank F/Resonant Peaks | 15.41 | fx_mod2 (0.73) | fx_eq+fx_mod2 (0.73) | fx_delay+fx_compressor+fx_reverb+fx_eq+fx_mod1+fx_mod2 (0.00) | 0.0 | fx_mod2 (0.73) |
| 17 | Bank D/Space Bass | 14.83 | fx_mod2 (1.66) | fx_reverb+fx_mod2 (0.77) | fx_delay+fx_compressor+fx_reverb+fx_eq+fx_mod2 (0.00) | 0.0 | fx_mod2 (1.66) |
| 18 | Bank F/DistantGrand | 14.77 | fx_delay (4.52) | fx_delay+fx_reverb (3.76) | fx_delay+fx_compressor+fx_reverb+fx_eq (0.00) | 0.0 | fx_delay+fx_reverb+fx_eq (0.01) |
| 19 | Bank C/Bounce Clav | 14.74 | fx_compressor (6.29) | fx_compressor+fx_reverb (6.20) | fx_delay+fx_compressor+fx_reverb+fx_eq+fx_mod1+fx_mod2 (0.00) | 0.0 | fx_compressor+fx_eq+fx_mod1+fx_mod2 (0.54) |
| 20 | Bank C/J3 Warm 80s Brs | 14.66 | fx_mod2 (1.51) | fx_reverb+fx_mod2 (0.00) | fx_compressor+fx_reverb+fx_eq+fx_mod2 (0.00) | 0.0 | fx_mod2 (1.51) |

Counts over the 20 (smallest explaining set): Mod 2 14, EQ 10, compressor 7, Mod 1 6, delay 2, reverb 2. Mean left score: best single group 3.82, best pair 1.60. Still above 2 after the best pair: Roto EP Ondre J 2.66, PineScape Synth 2.84, Crackle n Pd Whl 2.81, Time Warp Saw 5.65, DistantGrand 3.76, Bounce Clav 6.20. The score is not monotone in the number of groups: Resonant Peaks reads 0.73 for Mod 2 alone, 3.53 with five groups reverted and 0 only with all six; Bounce Clav rises from 6.29 (compressor) to 7.17 with delay, compressor, reverb and Mod 2 reverted. A firmware stage after an engine stage sees a different signal than in the all-firmware chain, which is why a group-by-group reading is not enough.

### 3.2 Before and after of switching the firmware FX path on

Worst 20 (the engine default with the listed groups switched to firmware, score against the firmware render; all 63 sets per program in [program-ranking-fx-flips-worst20.csv](data/program-ranking-fx-flips-worst20.csv)):

| Flipped set | n | Mean | Median | p90 | Max | > 5 | > 10 | Worse by > 0.5 |
|---|---|---|---|---|---|---|---|---|
| none (default) | 20 | 18.14 | 18.03 | 21.74 | 22.15 | 20 | 20 | - |
| fx_mod2 | 20 | 9.67 | 6.52 | 18.21 | 22.15 | 12 | 9 | 0 |
| fx_eq+fx_mod2 | 20 | 8.04 | 3.86 | 18.21 | 22.15 | 9 | 8 | 1 |
| fx_mod1+fx_mod2 | 20 | 6.20 | 2.89 | 15.83 | 21.03 | 9 | 4 | 0 |
| fx_eq+fx_mod1+fx_mod2 | 20 | 4.74 | 2.30 | 12.96 | 16.61 | 6 | 4 | 0 |
| fx_compressor+fx_eq+fx_mod1+fx_mod2 | 20 | 3.25 | 1.29 | 9.50 | 16.61 | 4 | 2 | 0 |
| fx_delay+fx_compressor+fx_eq+fx_mod1+fx_mod2 | 20 | 2.19 | 0.98 | 4.21 | 16.61 | 2 | 1 | 0 |
| fx_delay+fx_compressor+fx_reverb+fx_eq+fx_mod1+fx_mod2 | 20 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | 0 | 0 |

All 384 programs ([program-ranking-fx-flips-384.csv](data/program-ranking-fx-flips-384.csv)):

| Flipped set | n | Mean | Median | p90 | Max | > 5 | > 10 | Worse by > 0.5 |
|---|---|---|---|---|---|---|---|---|
| none (default) | 384 | 5.35 | 3.92 | 12.72 | 22.15 | 162 | 61 | - |
| fx_delay+fx_compressor+fx_eq+fx_mod1+fx_mod2 | 384 | 1.66 | 1.04 | 3.84 | 16.61 | 22 | 3 | 6 |
| fx_eq+fx_mod1+fx_mod2 | 384 | 2.70 | 1.71 | 6.26 | 16.61 | 54 | 16 | 20 |
| fx_mod2 | 384 | 3.78 | 2.76 | 8.24 | 22.15 | 106 | 24 | 15 |

After the PARITY set 22 programs stay above 5: 10 organ programs, 8 synth programs, and one each of organ+piano+synth, organ+sample, piano and sample. The largest are Sine Rain Pad 16.6 (its gap is the reverb: reverting `fx_reverb` alone leaves 0.01, and the reverb ports are not in the preset), B3 Squabble 12.7, Gray Pad Whl 10.6, B3 Dual Samba 9.9, Synth Orchestra 9.8, B3 Bassline 9.4. The organ ones are tone wheel and Leslie programs: their distance sits in the organ tone source (`--tone-source firmware`) and the rotary, not in the FX groups.

### 3.3 Null of the PARITY chain and of the engine against the firmware model

`fx_parity_null` (noise, 65,536 frames at 43,402 Hz, the cases of `fx_cases` that have a port):

| Family | Cases | Engine null dB | PARITY null dB |
|---|---|---|---|
| Mod 1 | 36 | -323.3 (amount 0) to +3.7; -10.7 to +3.7 at amounts 64, 127 | -325.5 to -316.3 |
| Mod 2 | 36 | -323.3 (amount 0) to +3.1; -7.3 to +3.1 at amounts 64, 127 | -327.5 to -320.2 |
| EQ | 6 | -27.4 to -7.7 | -334.2 to -314.5 |
| Compressor | 4 | -15.1 to -2.9 | -89.7 to -85.6 |
| Delay (laws, time) | 3 | -6.2 to +0.5 | -326.9 to -324.4 |
| Reverb (not in PARITY) | 24 | -322.9 to +17.0 | -58.1 to +17.0 (the PARITY chain runs the engine reverb) |

At amount 0 the engine and the port are identical (modulation off); the engine differs as soon as the effect is on. Amp rows are not part of the preset (the amp port is already the default). The drive 127 rows read silent in the port, as D158 and OQ38 record.

### 3.4 The Mod 2 default flip

`program_rank --flip fx_mod2` on the pre-flip default (`--legacy-mod2` reproduces it): 384 programs 5.35 to 3.78 mean, worst 20 18.14 to 9.67. 111 programs improve by more than 0.5, 258 stay within 0.5, 15 get worse by more than 0.5: Wah EP Ondre J 6.78 to 12.93, TapePiano AT 10.04 to 14.79, Stereo Wah Clav 10.37 to 15.07, Melody Pearls 5.40 to 9.43, Gravity Whl 2.92 to 5.79, Taped Wurly 0.81 to 3.54, Aerial Gaze 4.63 to 7.15, Long EP Ondre J 7.65 to 9.98, J3 80 Underscore 2.91 to 4.79, Breath Pad 5.60 to 6.86, Jazz Guitar II 12.50 to 13.54, Frozen Time Whl 1.26 to 2.30, Boxxx 6.23 to 7.10, 24 Frames J3 12.76 to 13.33, Ostinato Whl 1.64 to 2.16. Cause: a firmware Mod 2 after an engine Mod 1, EQ or compressor sees a different signal than in the all-firmware chain (the pairing result of s3.1: Mod 2 needs EQ), so those programs need the next flips, not a revert. Data: `docs/research/data/program-ranking-fx-flips-384.csv` (`after_flip_fx_mod2`). After the flip the ranking tool scores against the new default, where the `fx_mod2` group is a no-op; `--legacy-mod2` reproduces the committed pre-D163 numbers.

## 4. Why the rest stays opt-in

Mod 2 passed on the owner's rule for this PR (oracle null for every type, ranked distance and `render_hash` shown, opt-out kept; the chorus clip stays an open item). D22 needs the algorithm and the ARM to engine mapping VERIFIED and `render_hash` re-baselined; D136 adds the owner's rule that no hardware clip may get worse and that a missing clip needs the owner's OK. Per group of the PARITY set:

| Group | VERIFIED | Open (blocks a default flip) |
|---|---|---|
| Mod 2 (**flipped**) | routines (oracle 500/500 per type), laws on the dense ARM grid (D90, D100) | not blocking after the flip; open: chorus delay 1.7 times the clip (fx-chorus-hw-clip s5, scope calibration ASSUMED); ENS and SPIN have no clip; the D22 bar "no side worse than 3 dB" is unmet for CHOR |
| EQ | tone stack routine, gain law, bass and treble words | joint mid words, interpolation between captured indices and the organ unit routine layout ASSUMED; the recurrence runs at the engine rate (frequency factor 1.106, `--fx-rate-match`, D137) |
| Mod 1 | A-PAN, TREM, RM, A-WAH, WAH (oracle) | RM / A-WAH / WAH / PUMP run at 2 frames per call (OQ38: the live pass is 4); PUMP free-runs at 6.11 Hz against 2.11 Hz on the hardware (OQ36) |
| Delay | time law (oracle, 264 words) | `delay_laws` role EMPIRICAL, no delay clip |
| Compressor | routine | slope and log gain ASSUMED, no compressor clip |
| Reverb (left out) | routines (D145, D157) | the owner hears the firmware reverb worse by ear (A:21, [firmware-flag-audit](firmware-flag-audit.md)); tail clip wanted (OQ21c) |

This restates the audit of [firmware-flag-audit](firmware-flag-audit.md) per group. The ranking adds the size of the prize: the oracle-verified FX path moves 251 of 384 programs by more than 0.5 and takes the median score from 3.9 to 1.0, so each open item above is worth more than its row in the audit suggested.

## 5. What this does not say

1. **Not hardware.** The reference is the firmware-derived path. A program that reaches 0 can still be far from the hardware; the chorus clip says the Mod 2 delay is not right in absolute terms.
2. **One note** (key 60, velocity 100, 2.08 s held): FX that act on arpeggios, pedals or long tails are measured only by this note.
3. **The firmware render is the target on both sides**, so flipping every group reaches 0 by construction; the informative numbers are those of the partial sets and of the programs that stay above 2.
4. The 384 run covers three sets (PARITY, EQ + Mod 1 + Mod 2, Mod 2); the other 60 combinations ran for the worst 20 only.
5. The score weights are ASSUMED ([program-ranking](program-ranking.md) s2).

## Implications

- The FX path, not an engine, carries the largest distance of the factory set to the oracle-verified model; **Mod 2 plus EQ** is the pair behind most of the worst programs, and Mod 1, the compressor and the delay carry the ones that remain.
- Mod 2 is the default since D163; `--firmware-fx-ports parity` applies the rest of this evidence in one switch, and D163 proposes making it the default once the owner has decided the open items of s4. What stays high after it is reverb (Sine Rain Pad, DistantGrand) and the organ programs.
- The cheapest items that would let the flip pass D136: a dry-reference Mod 2 clip (chorus delay), the EQ joint mid words, the Mod 1 call count.

## 7. Update 2026-11-20 (D173): PARITY becomes the default; rate match and the chorus delay

- **Checked:** 2026-11-20. Same sources as above plus `emu_vs_hw.py` on the six hardware clips ([emu_vs_hw_parity_rate_match.json](data/emu_vs_hw_parity_rate_match.json), results [emu_vs_hw_parity_rate_match_results.json](data/emu_vs_hw_parity_rate_match_results.json)). New: `program_rank --flip fx_rate_match`, `--legacy-fx-ports`, `program_rank_combo.py --moves`. Data: [program-ranking-fx-parity-flip-384.csv](data/program-ranking-fx-parity-flip-384.csv).
- **Question:** with the chorus delay explained ([fx-chorus-hw-clip](fx-chorus-hw-clip.md) s8), does the rest of `PARITY` pass as the default, and should `--fx-rate-match` follow?

### 7.1 384 programs, default of D163 to D170 against PARITY (score against the all-firmware render)

| Flipped set | n | Mean | Median | p90 | Max | > 5 | > 10 | Worse by > 0.5 | Tag |
|---|---|---|---|---|---|---|---|---|---|
| none (amp, PUMP, Mod 2) | 384 | 3.72 | 2.74 | 8.18 | 21.15 | 100 | 23 | - | EMPIRICAL |
| delay + compressor + EQ + Mod 1 (= PARITY) | 384 | 1.66 | 1.04 | 3.84 | 16.61 | 22 | 3 | 8 | EMPIRICAL |

213 programs improve by more than 0.5. The 8 that get worse: Silent Upright 2.62 to 4.89, Innocense 0.83 to 2.54, NS4 Palette 1.80 to 3.50, White Chorus 0.72 to 2.28, Marimba 0.54 to 1.65, Phil Pad 0.21 to 1.08, Detuned Fanfare 1.94 to 2.66, Arctic Choir Whl 1.39 to 2.01. The all-firmware render is the target on both sides, so these eight differ from the firmware render with the new set in a way the chosen subset does not explain (the missing pieces are the reverb ports, kept out by the owner).

### 7.2 Six hardware clips (score of `emu_vs_hw.py`, smaller is better)

| Clip | default | rate-match | parity | parity + rate-match |
|---|---|---|---|---|
| organ B3 | 5.42 | 5.42 | 5.45 | 5.45 |
| organ Vox + Twin (default tone) | 2.53 | 2.02 | 2.74 | 2.24 |
| organ Vox (vox tone) | 4.90 | 4.57 | 4.69 | 4.34 |
| piano dry | 2.02 | 2.02 | 2.02 | 2.02 |
| synth lead | 0.69 | 0.69 | 0.69 | 0.69 |
| synth SUPER LP24 sidechain | 4.47 | 4.47 | 3.78 | 3.78 |
| FX amp (Tiny Transistor) | 5.37 | 3.80 | 4.29 | 3.23 |

No clip gets worse by more than 0.21 under PARITY (Vintage Vox 2.53 to 2.74; recovered to 2.24 with the rate match). EMPIRICAL (one take per clip). The clip run is from before the merge of the PUMP default (D134) and was not repeated; whether a clip program uses a PUMP was not checked.

### 7.3 `--fx-rate-match`

Flipped alone on the 384 programs it moves the render from the default by a mean of 0.29 score units (median 0.00; 43 programs by more than 1, 12 by more than 3; largest Farfisa II 7.0, Texas Nights J3 6.4, NS4 Palette 5.2); the moves are the amp and cabinet programs, as D137 predicted. No firmware render exists for it (it is a converter), so the direction comes from the clips in 7.2: it never worsens a clip and takes the amp clip from 5.37 to 3.80. The owner has not been asked yet: it stayed opt-in in D173 and became the default in D180 (section 8).

### 7.3b Program sweep

**Program sweep (merged with PUMP, D134; `program_sweep` against `--legacy-fx-ports`):** 384 programs, no panic and no non-finite sample; the default render changes in 297 programs (128 by more than 1 dB RMS, 40 by more than 3 dB, largest +9.8 dB 16th Note Bass and -9.5 dB PineScape Synth), the firmware render in none. The default-to-firmware level gap shrinks from mean 1.29 to 0.42 dB (programs above 3 dB 48 to 3, above 6 dB 10 to 0); one program's gap grows by more than 1 dB (Night Gaunts Whl 0.2 to 3.0). 18 programs clip more samples (29 programs with a clipped sample against 24; Q and A 0 to 862 samples; the firmware render clips 27 programs by itself), the higher level is the firmware's. `docs/research/data/program-sweep.json` and `program-coverage.md` are regenerated.

### 7.4 Remaining blockers (not waived by the flip)

Compressor words (6.2; row ported opt-in in D187, s9), Mod 1 call count (OQ38; per unit kind in D186, s9), PUMP clock (OQ36), EQ joint mid words, CHOR type 5 / ENS / SPIN clips.

Reproduce (the ranking tools now compare against the new default; `--legacy-fx-ports` reproduces the D163 baseline, `--legacy-mod2` the older data):

```bash
cargo build --release -p nord-tools --bin program_rank
target/release/program_rank "$B" --jobs 12 --legacy-fx-ports   --flip "fx_rate_match,fx_delay+fx_compressor+fx_eq+fx_mod1,fx_delay+fx_compressor+fx_eq+fx_mod1+fx_rate_match" > flip384.jsonl
python tools/program_rank_combo.py --flip flip384.jsonl --md --flip-sets "fx_delay+fx_compressor+fx_eq+fx_mod1"
python tools/program_rank_combo.py --flip flip384.jsonl --moves fx_rate_match
uv run python tools/emu_vs_hw.py --spec docs/research/data/emu_vs_hw_parity_rate_match.json --video <Part A .webm> --backup "$B" --workdir <scratch> --confirmed-only
```

## 8. Update 2026-11-21 (D180): the FX rate match becomes the default

- **Checked:** 2026-11-21, on `main` at `0af9c66a` plus the D180 change. Sources: the factory restore backup (384 programs, key 60, velocity 100) and the Part A video (six clips), both local. Tools: `program_rank --legacy-fx-rate --flip fx_rate_match` (new `--skip N --every K` shards a run), `program_sweep` with and without `--legacy-fx-rate`, `tools/emu_vs_hw.py --spec docs/research/data/emu_vs_hw_rate_match_default.json`. Data: [program-ranking-rate-match-default-384.csv](data/program-ranking-rate-match-default-384.csv), [emu_vs_hw_rate_match_default_results.json](data/emu_vs_hw_rate_match_default_results.json), `program-sweep.json` regenerated.
- **Question:** the owner approved the flip (2026-11-21); does the re-run on current `main` (PUMP D134, Mod 2 D163, PARITY D173) show a clip or many programs clearly worse?

### 8.1 Six hardware clips (score of `emu_vs_hw.py`, smaller is better; EMPIRICAL, one take per clip)

| Clip | `--legacy-fx-rate` (D173 default) | rate match (new default) |
|---|---|---|
| organ B3 | 5.45 | 5.45 |
| organ Vox + Twin (default tone) | 2.74 | 2.25 |
| organ Vox (vox tone) | 4.70 | 4.35 |
| piano dry | 2.02 | 2.02 |
| synth lead | 0.69 | 0.69 |
| synth SUPER LP24 sidechain | 3.78 | 3.78 |
| FX amp (Tiny Transistor) | 4.29 | 3.23 |

No clip is worse; three improve and the others score the same (no firmware amp in the program). The numbers repeat the table of 7.2 on the tree after the PUMP default, which that table lacked.

### 8.2 384 programs: the ranking score cannot judge the converter

`program_rank` scores a render by its distance to the all-firmware render, and that render has no converter (it is a converter, not a firmware port, D137). Flipping it therefore can only move an amp program **away** from the target in this metric, whatever the hardware does. Result, same method as 7.3 (target rendered with the same converter setting as the baseline): 44 programs change (the others have no firmware amp); mean move 0.29 score units (median 0.00), 43 programs by more than 1, 13 by more than 3, largest Farfisa II 7.0, Texas Nights J3 6.2, Vibra-Spin Whl 4.6; score mean 1.66 to 1.84, median 1.04 to 1.14, programs above 5 from 22 to 27, above 10 stay 3. 36 programs score more than 0.5 worse **in this metric** (list in the CSV; the largest: Farfisa II 0.93 to 6.57, Texas Nights J3 3.36 to 7.78, Distorted Whl II 2.40 to 6.07, Jealous Upright 1.44 to 5.00, Q and A 0.00 to 3.42, Silent Upright 4.89 to 7.97, Jazz Guitar II 1.02 to 4.05, LoFi Piano 0.65 to 3.57); all of them use a firmware Amp type. These are the same moves as in 7.3 (43 over 1, 13 over 3, mean 0.29); the direction is decided by the clips in 8.1, where every amp clip improves. The effect is the one D137 predicted (cabinet features 1.106 times too high in Hz without the converter).

### 8.3 Program sweep (default render, rate match against `--legacy-fx-rate`)

384 programs, no panic, no non-finite sample. The default render changes in 44 programs (35 by more than 0.1 dB RMS, 2 by more than 1 dB: Vibra-Spin Whl +1.75 dB, LoFi Piano +1.19 dB; none by 3 dB). The default-to-firmware level gap stays mean 0.42 dB (3 programs above 3 dB, 0 above 6, no gap grows by more than 1 dB). Programs with a clipped default sample 29 to 28 (Vox Inna-Gadda 2,987 to 3,167 clipped samples). The converter adds about 0.8 ms of latency on layers whose amp runs the firmware stage (not measured in the sweep).

## Follow-ups

1. Owner decision on the rest of D163 (flip `FirmwareFx::DEFAULT` from amp + Mod 2 to `PARITY`, GUI default); `eq` + `mod1` + `mod2` is the smaller step: `eq` + `mod1` + `mod2` takes the worst 20 from 18.14 to 4.74 and all 384 from 5.35 to 2.70 (the tables above).
2. Ablate the other 364 programs with `--ablate-fx` for a carrier per program (about 15 s each with `--jobs 20`).
3. After the owner decides the reverb ports, re-run with `all-fx` and look at the 22 programs above 5 (organ tone and Leslie).
4. Amp drive 127 reads silent in the port (OQ38); the amp port is on by default, so count the factory programs that set the drive to maximum.

## Reproduce

```bash
cargo build --release -p nord-tools --bin program_rank --bin fx_parity_null
B=".reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b"
# worst 20: 63 combinations each (about 15 s per program with 20 jobs)
target/release/program_rank "$B" --only "Bank D/Melody Pad.ns4p" --ablate-fx --jobs 20 >> fx.jsonl
python tools/program_rank_combo.py --ablate-fx fx.jsonl --md \
  --csv docs/research/data/program-ranking-fx-combos.csv
# flips on the worst 20 (every combination) and on all 384 (three sets, about 90 min, 14 GB RAM)
target/release/program_rank "$B" --only "Bank D/Melody Pad.ns4p" --flip every-fx --jobs 20 >> flip20.jsonl
target/release/program_rank "$B" --jobs 8 \
  --flip "fx_delay+fx_compressor+fx_eq+fx_mod1+fx_mod2,fx_eq+fx_mod1+fx_mod2,fx_mod2" > flip384.jsonl
python tools/program_rank_combo.py --flip flip384.jsonl --md \
  --csv docs/research/data/program-ranking-fx-flips-384.csv
target/release/fx_parity_null > parity_null.csv
cargo test --release -p nord-tools --lib fx_parity
uv run pytest tools/tests/test_program_rank_combo.py
cargo run --release -p nord-tools --bin render_hash -- rh.f32le && sha256sum rh.f32le
```

## 6. Re-run on the D163 to D166 defaults (2026-11-19)

- **Checked:** 2026-11-19. Same sources and tools as above plus `program_rank --flip fx_delay_time,fx_delay_laws` (new sub-groups of `fx_delay`, flip only, `rank_groups::SUBGROUPS`). Data: the five `program-ranking*.csv` files regenerated, `program-sweep.json` regenerated.
- Engine default now = amp + Mod 2 (D163), Touch (D165), arp walk and morph (D166). Ranking: mean 3.77, median 2.74, p90 8.24, max 21.15; above 5: 104, above 10: 24 ([program-ranking](program-ranking.md) s1b).

Flip of one more group on top of the current default, all 384 programs (score against the all-firmware render):

| Flipped set | Mean | Median | p90 | Max | > 5 | > 10 | Worse by > 0.5 |
|---|---|---|---|---|---|---|---|
| none (default) | 3.77 | 2.74 | 8.24 | 21.15 | 104 | 24 | - |
| fx_mod1 | 3.33 | 2.31 | 7.25 | 21.03 | 85 | 15 | 7 |
| fx_compressor | 3.29 | 2.15 | 6.84 | 21.15 | 79 | 18 | 14 |
| fx_delay | 3.56 | 2.61 | 7.63 | 21.15 | 93 | 20 | 11 |
| fx_delay_laws only | 3.58 | 2.67 | 7.67 | 21.15 | 95 | 20 | 11 |
| fx_delay_time only | 3.75 | 2.72 | 8.18 | 22.35 | 104 | 25 | 7 |
| fx_eq | 3.15 | 1.98 | 7.20 | 21.15 | 70 | 22 | 24 |
| fx_eq+fx_mod1 | 2.70 | 1.71 | 6.30 | 16.61 | 54 | 15 | 24 |
| fx_delay+fx_compressor+fx_eq+fx_mod1 | 1.66 | 1.04 | 3.84 | 16.61 | 22 | 3 | 8 |

The `fx_delay_laws only` and `fx_delay_time only` rows come from a run of `program_rank --flip fx_delay_time,fx_delay_laws,fx_delay` whose per-program lines were **not committed** (only the summary is kept; rerun with the command in s6 header to reproduce, about 8 min with six per-bank shards). The delay gain sits in the mix law (`delay_laws`, role EMPIRICAL, no delay clip); the time law alone (oracle verified) moves nothing and makes PineScape Synth worse (21.03 to 22.35).

### 6.1 Why no further group is flipped in this change

| Group | Evidence for a flip | Blocker |
|---|---|---|
| Mod 1 (8 of the worst 20) | A-PAN, TREM, RM, A-WAH, WAH, PUMP routines null the oracle (fx-vs-oracle s3.2) | OQ38: call length. Boot words say 1 block of 2 frames for the piano unit and 2 blocks for the synth unit; the port uses 2 frames for types 4 to 11 and the oracle pass is 4 frames (null -16 to -26 dB at 2). Also a possible factor 2 in the LFO Hz. Needs a call-count trace |
| Delay (4) | time law VERIFIED on 264 words | the part that moves the score is the mix law, role EMPIRICAL, no clip |
| Compressor (3) | routine nulls the oracle (-117 dB or better, level delta 0.00) | **new finding below**: the context words around the routine were ASSUMED and are not the firmware's |
| EQ (2) | tone stack routine | not run live (2-frame port against the 4-frame pass), joint mid words ASSUMED |
| Reverb (2) | routines null the oracle | owner veto by ear (D163) |

### 6.2 Finding: the firmware compressor row is in the FX image and differs from `FwCompressor::new` (VERIFIED, local image)

Compressor instance of FX core 1 (`I1 = 0xB0168`, attack pair at `DM(0xB0148)` as written by the ARM boot log), read from the static FX image (`tools/ns4_dsp_ldr.py dsp_image_0_FX.bin --sram`, word at `4 * address` of `core1_sram.bin`):

| Word | Offset | Firmware image | `FwCompressor::new` / `compressor_for_amount` (ASSUMED) |
|---|---|---|---|
| attack pair | -0x20 | 0.1021, 1.0 | 0.102071 (ARM), 1.0 |
| release pair | -0x1E | 0.0004889, 0.04 | 0.000460684 (ARM), 0.04 |
| SLOPE | -0x17 | **0.75** | 0.05 |
| REDUCTION_SCALE | -0x15 | **0.0104167 (1/96)** | 0.5 |
| TABLE1 | -0x14 | **1.52588e-05 (2^-16)** | 0.5 |
| LOG_GAIN | -0x0B | **6.0** | 1.0 |
| TABLE0 | -0x0A | 1.0 | 1.0 |
| SMOOTH | -0x09 | 0.99078 | 0.9908 |
| MANT_SCALE, FIX_SCALE | -0x0D, -0x0C | -31, 31 | -31, 31 |

So the port, which nulls the oracle for its own words, does not reproduce the firmware gain law: `fx-vs-oracle` s3.4 proves the routine, not the words. **Update 2026-11-21 (D187, s9):** confirmed unmodified at runtime (the ARM writes none of the gain-law words, the oracle chain with the row nulls the port at -102 to -108 dB, [fx-compressor-row](fx-compressor-row.md)); the port is opt-in `--compressor-row`. Following change (not in this PR): take SLOPE, REDUCTION_SCALE, TABLE1, LOG_GAIN from the image in `compressor_for_amount`, rerun `fx_vs_oracle.py --only COMP`, then ranked before/after and the default flip decision (opt-out `--legacy-compressor`), with an `ns4 bench` number and the apply_program, edit, render tests. Tracked in `docs/OPEN-QUESTIONS.md` OQ38 note and bean follow-up.

## 9. Update 2026-11-21 (D186, D187): compressor row and Mod 1 unit calls, opt-in, flip proposed

`program_rank --flip fx_compressor_row,fx_mod1_unit_calls,fx_compressor_row+fx_mod1_unit_calls` on the current default (4 shards), score against the all-firmware render, which now includes both switches (`rank_groups::firmware_options`). Data: [program-ranking-compressor-row-mod1-units-384.csv](data/program-ranking-compressor-row-mod1-units-384.csv).

| Flipped set | Mean | Median | p90 | Max | > 5 | > 10 | Worse by > 0.5 |
|---|---|---|---|---|---|---|---|
| none (default) | 2.12 | 1.28 | 4.66 | 28.04 | 35 | 7 | - |
| `fx_compressor_row` | 1.66 | 1.05 | 3.84 | 16.61 | 22 | 3 | 1 |
| `fx_mod1_unit_calls` | 2.12 | 1.27 | 4.66 | 28.04 | 35 | 7 | 1 |
| both | 1.66 | 1.04 | 3.84 | 16.61 | 22 | 3 | 1 |

- **Compressor row:** 50 programs improve by more than 0.5 (16th Note Bass 28.04 to 0.00, Burning Lead 24.25 to 0.65, Bounce Clav 21.20 to 0.59, Fan Club Piano 13.57 to 1.23, Filter Bank Ld 8.77 to 0.00), 1 gets worse (Silent Upright 5.12 to 6.09). Caveat: the target contains the row, so the table says which programs carry the compressor, not that the row is right; the evidence that it is right is the oracle chain (-102 to -108 dB, [fx-compressor-row](fx-compressor-row.md)).
- **Mod 1 unit calls:** only the synth chains change, so the ranking sees 1 program move over 0.5 (Time Warp Saw 5.87 to 6.83, worse) and the mean moves 0.011. The ranking cannot judge this switch; the evidence is the ARM words (organ and piano 1 block, synth 2 blocks, [fx-mod1-call-count](fx-mod1-call-count.md)). No hardware clip.
- No panic, no non-finite sample in the 384 renders; `render_hash` and the golden hash are unchanged (defaults unchanged).
