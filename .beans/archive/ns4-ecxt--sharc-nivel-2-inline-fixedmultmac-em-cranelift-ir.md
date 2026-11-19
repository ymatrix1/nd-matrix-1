---
# ns4-ecxt
title: 'SHARC Nivel 2: inline FixedMult/MAC em Cranelift IR'
status: completed
type: task
priority: normal
created_at: 2026-11-16T16:02:47Z
updated_at: 2026-11-16T16:06:38Z
parent: ns4-trkb
blocked_by:
    - ns4-y7lf
---

Lowered non-rounding FixedMult (cls 1..=3, !round) directly to Cranelift IR with 1:1 80-bit MR arithmetic, MN/MV/MU flags, and cold jit_note_mult_overflow.

## Summary of Changes
- crates/nord-sharc/src/core/jit.rs: added emit_fixed_mult, jit_note_mult_overflow, shared emit_jit_compute_op, and jit_inline_fixed_mult_matches_interp_including_mv_and_mu.
