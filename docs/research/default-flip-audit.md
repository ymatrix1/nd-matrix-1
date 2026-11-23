# Default flip audit of the validated opt-ins (D223)

- **Checked:** 2026-11-22 (`main` @ `80ddde30`)
- **Sources (local, never committed):** the Part A video clips and the factory backup under `.reference/` (`tools/emu_vs_hw.py`), `target/release/{ns4,program_sweep}`, the DECISIONS entries and research notes cited per row. Committed data: [default-flip-clip-scores.csv](data/default-flip-clip-scores.csv).
- **Question:** the owner said on 2026-11-22 "flip what was validated". Which of the recent opt-ins are VERIFIED against the firmware or oracle with no ASSUMED or EMPIRICAL dependency left, **and** keep every hardware clip at or below its old score (the owner rule of 2026-11-17, [firmware-flag-audit](firmware-flag-audit.md) s1)?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Verdicts

| Opt-in | Evidence | Remaining ASSUMED / EMPIRICAL | Hardware clips | Verdict |
|---|---|---|---|---|
| `--synth-fw-resonance` (D214) | VERIFIED: the damping tables and the per-type selection equal the oracle stage for all 128 knobs | none in the flag (the 0.995 per pass smoother is not ported, no clip shows it) | s3: mean 2.671 to 2.668, no segment worse than 0.01, `synth_filterbank` 0.08 better | **FLIP** (`--legacy-synth-resonance`) |
| settled envelope tables, `DEFAULT_TABLES = SETTLED_TABLES` (D212) | VERIFIED: installer words after the 16 pass refresh, 129 of 129 indexes | linear interpolation between integer indexes; the pass period (one pass is two frames) and the panel time inverse still read the stale table | s2: 7 of 17 segments worse with the law off (4 by 0.09 or more), `synth_12db` 1.38 to 3.15, `super_lp24_oopad` 4.88 to 5.44 | **KEEP OPT-IN** (clips worse) |
| `--synth-fw-env-law` (D200 / D207) | VERIFIED: base within 0.6 %, envelope within 0.5 % in `R`; LFO term within 0.01 semitone | engine `level` = oracle chain level times velocity peak, key tracking before the law, LFO stage full scale (all ASSUMED) | s2: `synth_lead` 0.80 to 3.52, `synth_silkpad` 1.36 to 3.95, `synth_horizon` 2.30 to 4.16 (law on against off, old tables) | **KEEP OPT-IN** (clips worse; the ASSUMED level link is the likely cause) |
| `--synth-fw-adr-law` (D210) | VERIFIED attack index 256 of 256 rows, sustain 0.5 on the chain | the audible half level sustain (ASSUMED) | s2: mean 3.162 to 3.350, `synth_filterbank` 3.98 to 5.64, `synth_leaky` 2.18 to 2.76 | **KEEP OPT-IN** (clips worse; hardware check of the sustain) |
| `--synth-osc-env-morph` (D206 / D213 / D217) | VERIFIED dispatch 27 of 27, DSP word, pitch law 46 rows | key velocity scaling ASSUMED; layers B / C open; no listening A/B of the three programs that move | none | **KEEP OPT-IN** (needs owner listening and hardware) |
| `--firmware-piano-timbre` (D209) | VERIFIED 108 of 108 gain words | classes 2 / 3 family map EMPIRICAL; Clav rocker modes open; octave shift and flag `0xC059868C + 0x904` ASSUMED | none | **KEEP OPT-IN** (needs hardware clip of Soft / Mid / Bright) |
| `--fx-compressor-first` (D215) | one clip, EMPIRICAL position | position contradicts the ASSUMED D173 position | one clip (A:25) | **KEEP OPT-IN** (needs a second compressor-on clip) |
| `lp24` FX port (D193) | VERIFIED routine and type 9 words, 24 call golden under 1e-5 | Mid mapping, Mid Freq index, rate scale ASSUMED | none | **KEEP OPT-IN** (needs an LP24 / HP24 clip) |
| `--organ-level chain` (D189) | VERIFIED words | `K = 2.8` EMPIRICAL | [firmware-flag-audit](firmware-flag-audit.md): organ scores worse than legacy | **KEEP OPT-IN** |

Not touched (no flip evidence, per the brief): Mod 2, unison and per-type gain, `--synth-filter-gain` (D201), the FX default words of D203 (the delay start state is ASSUMED, `reverb_mix` excluded), the reverb ports (owner veto).

## 2. Clip evidence of the envelope paths

`tools/emu_vs_hw.py` over the 17 synth segments of Part A (`synth_*`, `super_lp24_*`, `pianopad_grand`, `organ_pipe`), score = gap against the hardware clip, lower is better. One build with the three candidates as defaults; each column switches some off with the `--legacy-*` flags of that experiment (raw table: [default-flip-clip-scores.csv](data/default-flip-clip-scores.csv)). Selected rows:

| Clip | all off | tables on | law on | law and tables on | adr-law added |
|---|---|---|---|---|---|
| `synth_lead` | 0.80 | 0.86 | 3.52 | 3.39 | 3.67 |
| `synth_silkpad` | 1.36 | 1.45 | 3.95 | 4.12 | 4.23 |
| `synth_horizon` | 2.30 | 2.35 | 4.16 | 4.21 | 4.13 |
| `synth_12db` | 1.38 | 3.15 | 1.44 | 1.57 | 1.77 |
| `super_lp24_oopad` | 4.88 | 5.44 | 4.01 | 4.57 | 4.97 |
| `synth_filterbank` | 4.77 | 4.61 | 4.04 | 3.98 | 5.64 |
| `synth_leaky` | 1.90 | 2.02 | 2.07 | 2.18 | 2.76 |
| mean of 17 | 2.671 | 2.821 | 3.116 | 3.162 | 3.350 |

The firmware law is exact at the oracle but moves the three synth clips with a wide filter envelope 2 to 3 points away from the hardware; the settled tables fix some clips (`filterbank`) and break others (`12db`, `oopad`). Both point at an ASSUMED link between the verified word and the audible result (the level that scales the envelope, the pass period of the chains), not at the words.

## 3. Resonance position A/B (the flip)

Final build, default against `--legacy-synth-resonance`, same 17 segments ([default-flip-resonance-clip-scores.csv](data/default-flip-resonance-clip-scores.csv)): mean 2.671 to 2.668; 7 segments 0.01 worse (rounding level: `organ_pipe`, `synth_crunch`, `synth_horizon`, `super_lp24_horizon`, `super_lp24_sidechain`, `synth_80s`, `synth_leaky`), 4 better (`synth_filterbank` -0.08, `synth_12db` -0.03, `synth_sineplk` and `synth_megaline` -0.01). Program sweep over the 384 factory programs (`program_sweep`, before and after): 142 default renders change, 4 by 0.1 dB or more, 1 by 1 dB or more (`Glow Chords Whl` -1.10 dB, `Unclocked Pad` +0.10 dB); panic, NaN, Inf and silent renders stay 0, clipping programs stay 1. Golden `render_hash`: segments `engine` and `synth` change (peak 0.424921 to 0.424988, RMS 0.140556 to 0.140422 for `synth`), `organ`, `piano`, `fx` do not; whole file `18fafeee...a799` to `eb5f8f49...bbcc`.

## 4. Implications

- The oracle proves the words; the clips say the engine around them is not yet equal. A flip needs both.
- The resonance position is the one candidate where the engine already sat within one table index, so the flip is nearly free and measurable.

## 5. Follow-ups

- Done in D236 ([synth-env-level-link](synth-env-level-link.md)): the level link is VERIFIED; the regression is the fitted row and type cutoff factors. Open: pad source gap. Original item: find the ASSUMED link of the envelope law: oracle trace of the voice level that multiplies the filter envelope amount (D207 open: key tracking word `w38`, layers B / C strips, running LFO scale).
- Re-measure the pass period (D212 follow-up: invert the settled table in `strip_map.rs`) and re-score the settled tables.
- Split the attack velocity scaling (VERIFIED) out of `--synth-fw-adr-law` so it can flip without the audible sustain.
