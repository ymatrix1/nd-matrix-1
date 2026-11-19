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
