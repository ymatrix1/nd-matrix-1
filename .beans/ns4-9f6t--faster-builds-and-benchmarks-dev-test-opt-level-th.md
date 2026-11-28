---
# ns4-9f6t
title: 'Faster builds and benchmarks: dev-test opt-level, thread CPU clock, A/B driver'
status: in-progress
type: task
created_at: 2026-11-28T03:59:52Z
updated_at: 2026-11-28T03:59:52Z
parent: ns4-lle1
---

## Context
Test runs of the three emulator crates took about 76 s at opt-level 0, and every benchmark A/B needed 7 to 9 rounds on a quiet host. Seven sessions share one machine, so wall-clock numbers are noisy and slow to get.

## Higher Goal
Shorten the implement-measure loop for the real-time ARM and SHARC work (epic ns4-lle1).

## Acceptance Criteria
- [x] Dev/test profile runs nord-core, nord-sharc and nord-arm at opt-level 1; tests 76 s to 20 s, rebuild time unchanged (docs/research/build-bench-speed.md)
- [x] nord_sharc::cpu_clock (thread CPU time) used by pass_sched, arm_lle_bench, kernel_bench
- [x] tools/bench_ab.py interleaved A/B driver with band verdict and compiler-load note, with tests
- [x] D259 and the research note record the measurements and what did not help (aot profile: no gain)
- [ ] Merged

## Out of Scope
sccache as a global wrapper, a per-image AOT crate split, changing CI.

## Summary of Changes
See PR.
