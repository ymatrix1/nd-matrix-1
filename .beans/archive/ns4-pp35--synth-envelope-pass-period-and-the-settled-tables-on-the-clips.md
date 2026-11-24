---
# ns4-pp35
title: 'Synth envelope pass period and the settled tables on the hardware clips (D237)'
status: completed
type: task
created_at: 2026-11-23T00:00:00Z
updated_at: 2026-11-23T00:00:00Z
---

## Context
D212 committed the settled attack and decay / release tables; D223 kept them opt-in because 7 of 17 hardware clip segments scored worse (`synth_12db` 1.38 to 3.15) and the pass period behind the tables (2 DSP frames) was unconfirmed.

## Higher Goal
1:1 audio parity with the firmware: decide whether the settled envelope tables can become the default and what blocks them.

## Acceptance Criteria
- [x] Pass period measured with the firmware display tables and the oracle coefficients on all 128 knobs (2.000 frames, median 1.9995 / 2.0007), VERIFIED tag in `strip_map.rs`, tool `tools/synth_pass_period.py`, Rust and Python tests
- [x] Per index comparison of the stale and settled tables (`synth-env-rates-legacy-vs-settled.csv`)
- [x] Settled tables corrected where wrong: none needed; the panel time to index inverse now follows the tables
- [x] Opt-in `--synth-settled-rates`; default and `render_hash` unchanged
- [x] 17 clip segments scored before and after, plus the combinations with `--synth-fw-adr-law` and `--synth-fw-env-law` (`synth-env-pass-period-clips.csv`)
- [x] Flip proposal stated: no flip (8 of 17 segments worse)
- [x] Research note, D237, spec `synth-engine.md` 4.8, ROADMAP

## Out of Scope
Flipping `DEFAULT_TABLES`; fixing the envelope to cutoff level link of the firmware cutoff law; the `super_lp24_oopad` brightness gap; re-fitting `DECAY_TAU_FRACTION`.

## Summary of Changes
`strip_map.rs` (`*_with` inverse functions, tag), `strip_path.rs`, `fw_adr_law.rs`, `synth/mod.rs` (`settled_rates`), `render_opts.rs` (`--synth-settled-rates`), `tests_pass_period.rs`, `tools/synth_pass_period.py`, data CSVs, `docs/research/synth-env-pass-period.md`, D237, spec and ROADMAP. Result: the pass period and tables are one verified pair; the clip regressions come from the envelope to cutoff path, so the flip waits for that fix.
