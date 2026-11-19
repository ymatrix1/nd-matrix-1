---
# ns4-y7lf
title: SHARC JIT differential test & 8/32-voice polyphony benchmark
status: completed
type: task
priority: normal
created_at: 2026-11-16T15:55:15Z
updated_at: 2026-11-16T15:58:32Z
parent: ns4-trkb
blocked_by:
    - ns4-a7vs
---

Implemented differential harness and 8/32-voice polyphony benchmark in crates/nord-sharc/src/core/jit_diff.rs.

## Summary of Changes
- crates/nord-sharc/src/core/jit_diff.rs: added assert_core_state_eq, jit_differential_multi_voice_every_n_steps (N=7 step-chunk differential test), and bench_jit_polyphony_8_and_32_voices (128 frames @ 48 kHz = 2.67 ms budget).
- crates/nord-sharc/src/core/mod.rs: wired jit_diff under cfg(all(test, feature = "jit")).
