# Synth amp attack velocity scaling, decay sustain level and the settled attack table (oracle)

**Checked:** 2026-11-21.
**Sources:** `tools/synth_env_law.py` (ORG core 2 `sharc-sys` oracle, ARM-word injection and taps of the slot 0 chains `F-20` amp, `F-8` filter, `F+6` osc, one run per row; same captures as [synth-env-oracle-trace](synth-env-oracle-trace.md)), `crates/nord-tools/src/bin/synth_env_native.rs` (native chains), data `data/synth-env-law-attack.csv` (333 rows), `data/synth-env-law-keys.csv` (80 rows), `data/synth-env-law-sustain.csv` (48 rows), `data/synth-env-attack-table.csv` (128 indexes), `data/synth-env-law-programs.csv` (4 programs), [synth-env-oracle-trace](synth-env-oracle-trace.md), [synth-adr-and-strip-law](synth-adr-and-strip-law.md), [synth-velocity](synth-velocity.md), D200, D142.
**Question:** D200 left two items recorded but not implemented: the amp attack velocity scaling (`word * g / 127`, EMPIRICAL) and the decay sustain 0.5. Does the oracle support them exactly, on every key, and does a native opt-in then reproduce the oracle per program?

## 1. Method

`python tools/synth_env_law.py grid` injects one ADR word triple `attack:decay:release` (the same index in the three chains) at pass 0, keys MIDI 36 / 60 / 84 of the capture with the key velocity byte `0x80 + g` replaced (VERIFIED, [synth-velocity](synth-velocity.md) s1), holds 2600 passes (9000 for the sustain runs) and records per chain the attack slope (`(y[r+3] - y[r+1]) / 2` of the first ramp, the record coefficient `b`), the peak, the level at key off and the release ratio. The filter chain does not see `g`, so every run also measures an unscaled reference at the same word with the same method. About 570 oracle runs: attack 37 words x 9 `g` (key 60), every integer word -64..=63 at `g = 127`, keys 36 / 84 with 13 words x 4 `g`, the sustain grid (12 decay words x 2 `g`).

## 2. Amp attack velocity scaling (VERIFIED, law corrected)

The measured amp slope equals the unscaled slope of the table entry `floor(w * G / 128)`, `G` the effective key byte (`g`, and 128 for `g = 127`, the same `G` as the velocity gain of [synth-velocity](synth-velocity.md)):

| Law for a negative word `w` | rows that select exactly that entry (of 256: key 60, `g` 10..=110, 32 words) |
|---|---|
| `floor(w * G / 128)` = `(w * G) >> 7` | **256** |
| `round(w * g / 127)` (the D200 EMPIRICAL guess) | 192 |
| `floor(w * g / 127)` | 178 |
| `trunc` or `ceil` of `w * g / 127` | 78 each |

(`python tools/synth_env_law.py fit` over the grid CSVs; the committed rows are in `data/synth-env-law-attack.csv`, test `law_matches_the_oracle_rows`.) Further:

- 77 of 77 rows with a positive word or `g = 127` give the amp slope of the filter chain: positive words are not scaled.
- Keys 36 and 84 follow the same law: 80 of 80 negative-word rows (60 with `g < 127`), 24 of 24 positive-word rows unscaled. No note dependence.
- Decay and release are not scaled (their words are identical in the amp and the filter chain at every `g`).
- A soft key attacks slower: word -64 at `g = 98` selects index -49, at `g = 10` index -5.

The old guess `word * g / 127` was right to plateau accuracy (D200) but one index off in a quarter of the rows.

## 3. Decay sustain 0.5 (VERIFIED oracle, hardware audibility ASSUMED)

Level at key off is `0.5000` in both the amp and the filter chain for decay words -64, -48, -40, -32, -24, -16, -8, 0 at `g` 127 and 64 (32 rows). Slower words are still falling at key off (word 24: 0.745, 40: 0.882, 63: 0.958 at `g = 127`; the Hold word 63 droops 4.2 % in 9000 passes). The level does not depend on the word or `g`. The threshold is the image default of the decay record (`DM(0xC0DCE)`, [synth-adr-and-strip-law](synth-adr-and-strip-law.md) s1); the recall and key streams of the 5 captured programs write no core 2 word in `0xC0D00..0xC0FFF` other than the `0xC0FD6..` block (`arm_dsp_capture` rows), so no ARM word changes it, and the strip rate words are the only ADR words the edit path writes ([synth-knob-laws](synth-knob-laws.md)).

Caveat: a plateau at half level on the chain output does not prove a half level sustain on the audio output. The Nord manual lets the sound fade while the key is held, and the layer gain stage behind the chain is not isolated. So the native port is opt-in and the audio claim stays ASSUMED.

## 4. New finding: the native attack table is a stale sweep

`fw_env_rates.rs` (`ATTACK_BITS`, default strip chain) equals `data/synth-env-rate-table.csv`, which read each coefficient 3 passes after the poke. The installer rewrites the coefficients every 16th pass, so the sweep captured the previous value and the table holds one value over 5 or 6 indexes. The settled coefficient (every integer index measured here) is a smooth curve:

- 104 of 128 indexes differ by more than 2 % from the old table; the old value is up to 2.56 times too high (index -59: 0.0922 against 0.0361; -60: 0.0922 against 0.0431).
- Only the first index of each plateau matches (-64, -58, -53, ...).
- The decay and release table (`DECAY_BITS`) has the same shape: the committed velocity sweep measures release ratio 0.9606 at word -60 against 0.9317 in the table (`1 - a` 1.7 times too large), 0.99617 at -40 against 0.99530.

`fw_adr_law::ATTACK_B` holds the settled attack table (128 entries, test `attack_table_equals_the_oracle_csv`) and only the opt-in law uses it. The default tables are unchanged (no flip). The settled decay and release table was measured in D212 ([synth-env-rates-settled](synth-env-rates-settled.md)): the same shape, 104 of 129 indexes stale.

## 5. Native against oracle per program (key 60, velocity 100, `g = 98`; `data/synth-env-law-programs.csv`)

| Program | chain | attack `b` oracle / native default / native law | level at key off oracle / default / law |
|---|---|---|---|
| 12dB Sweep | amp | 0.00728 / 0.0922 / 0.00728 | 0.950 / 1.000 / 0.947 |
| 12dB Sweep | filter | 4.87e-05 / 6.68e-05 / 4.87e-05 | 0.528 / 0.724 / 0.528 |
| 80s Sci-Fi | amp | 0.00728 / 0.0922 / 0.00728 | 0.950 / 1.000 / 0.947 |
| 80s Sci-Fi | filter | 0.0922 / 0.0922 / 0.0922 | 0.500 / 0.213 / 0.500 |
| Crunch Synth | amp | 0.00728 / 0.0922 / 0.00728 | 0.950 / 1.000 / 0.947 |
| Crunch Synth | filter | 0.000849 / 0.000849 / 0.000849 | 0.500 / 0.395 / 0.500 |
| Smooth Lead | amp | 0.000949 / 0.00280 / 0.000949 | 0.954 / 1.000 / 0.952 |
| Smooth Lead | filter | 0.00119 / 0.00151 / 0.00119 | 0.937 / 0.930 / 0.931 |

All 8 attack coefficients agree to 3 digits; the level at key off agrees within 0.006 (the residue is the Hold droop of the old decay table, section 4). The default plays the amp attack 8 to 13 times too fast on the three Hold programs (word -64 at `g = 98` is index -49, the default plays -64 from the stale table) and misses the filter level at key off by 0.1 to 0.3. The cutoff in Hz is not part of this table: it depends on the filter envelope amount law (`--synth-fw-env-law`, D200).

## 6. Implications

- Opt-in `--synth-fw-adr-law` (`synth/fw_adr_law.rs`, `SynthLayer::fw_adr_law`, D210) applies the three facts: per voice amp attack `b = ATTACK_B[floor(w G / 128)]`, filter attack `ATTACK_B[w]`, decay record `thr = 0.5` in both chains (also Hold). Default unchanged.
- The default strip chain attack and decay / release times are off between plateau anchors (section 4). Fixing them changes every default render: owner's call after a clip score.

## 7. Follow-ups

- (Done, D212) Measure the settled decay / release table at every integer index (release ratio of the 128 words, slow words by the level at key off) and replace the plateaus; then score `--synth-fw-adr-law` together with `--synth-fw-env-law` on the hardware clips before any default flip.
- The amp chain peak exceeds 1 (4.0 at `g = 127`, 3.8 at 126, 1.2 at attack -50 and `g = 127`) when the decay word is -64 (not with decay -20); `peak = 1` for `g <= 98`. Explained as a fitted law in D212: the chain limit `DM(F - 22)` is `1 + 0.2 k` (592 of 592 rows), no factory program reaches it ([synth-env-rates-settled](synth-env-rates-settled.md) s3).
- Hardware check of the audible sustain (finite Decay on a held key).
- The 80s Sci-Fi base cutoff offset and the LFO / input gain terms stay open (D200).
