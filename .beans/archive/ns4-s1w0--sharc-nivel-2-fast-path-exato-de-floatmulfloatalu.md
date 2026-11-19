---
# ns4-s1w0
title: 'SHARC Nivel 2: fast-path exato de FloatMul/FloatAlu com guarda 40-bit/RND32'
status: completed
type: task
priority: normal
created_at: 2026-11-16T16:53:08Z
updated_at: 2026-11-16T16:56:28Z
parent: ns4-trkb
blocked_by:
    - ns4-ecxt
---

Implemented exact guarded 40-bit native f64 fast path (emit_exact_float_op) for FloatMul and FloatAlu (FADD/FSUB) when fast_float is false.

## Summary of Changes
- crates/nord-sharc/src/core/jit.rs: added emit_exact_float_op with MODE1_TRUNC|MODE1_RND32 guard, normal f32 + zero extension check, |exp_x - exp_y| <= 28 for add/sub, and zero 21-bit rounding tail check; falls back to jit_float_mul / jit_exec_compute on any edge case; added jit_exact_float_matches_interp_including_edge_cases.
