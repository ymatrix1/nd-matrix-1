---
# ns4-fxdf
title: 'FX parity default flip: chorus delay closed, rate-match evidence, 384 ranking'
status: completed
type: task
priority: normal
created_at: 2026-11-20T12:00:00Z
updated_at: 2026-11-20T12:00:00Z
parent: ns4-trkb
---

## Context
D163 flipped Mod 2 and left the rest of `FirmwareFx::PARITY` opt-in with three blockers: the chorus delay 1.7 times the hardware clip, the `--fx-rate-match` default (D137) and a re-run of the 384-program ranking. The owner approved the PARITY flip on 2026-11-20 (reverb ports out, opt-out flag).

## Higher Goal
Audio 1:1 with the hardware from firmware and oracle evidence only; the verified FX path is what a user hears by default.

## Acceptance Criteria
- [x] Chorus delay investigated with the SHARC oracle and a hardware audio fit; cause documented with a unit test (`tests_mod2_hw_delay`)
- [x] `--fx-rate-match` evaluated on six hardware clips and 384 programs (not flipped, recommended)
- [x] 384-program ranking re-run: mean 3.72 to 1.66, 8 programs worse, listed in D173
- [x] `FirmwareFx::DEFAULT` = PARITY, `--legacy-fx-ports` opt-out, tests for both
- [x] D173, spec, ROADMAP, audio-remaining; golden `render_hash` re-baselined with the changed segments named
- [x] All AGENTS.md checks

## Out of Scope
Reverb ports, the `--fx-rate-match` default, compressor words from the FX image, Mod 1 call count, GUI / plugin defaults beyond the engine tests.

## Summary of Changes
- Default flip and opt-out (`render_opts.rs`, `fx/firmware_fx*.rs`), ranking tools (`--legacy-fx-ports`, `fx_rate_match` flip group, `--moves`), chorus tools and test, research notes, D173, golden re-baseline.
