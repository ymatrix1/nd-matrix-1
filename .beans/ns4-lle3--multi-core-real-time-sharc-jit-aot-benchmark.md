---
# ns4-lle3
title: 'Multi-core real-time SHARC JIT/AOT benchmark at 48 kHz'
status: in-progress
type: task
priority: high
created_at: 2026-11-23T21:40:00Z
updated_at: 2026-11-23T22:30:00Z
parent: ns4-lle1
---

## Context
`crates/nord-sharc` already has a Cranelift JIT that matches the interpreter 316/316 passes on PNO core 1 (`sharc-sys --jit-diff --strict`, improved by PR #460 `ns4-z2fq` loop SSA promotion) and a 1-thread-per-core `MultiCoreScheduler` (PR #461 `ns4-spj2`). However, PR #466 (`D118`) and `docs/research/sharc-aot-multi-block.md` showed that basic-block JIT/AOT still leaves ~56.7% of steps in `Core::step()` fallback (~11–16 Msteps/s vs ~93–129 Msteps/s needed for `46.08 us/pass`). Real-time 48 kHz execution across all active SHARC cores (`ORG c1/c2`, `PNO c1/c2`, `FX c1/c2`, `SY1SY2`, `SMP`) therefore requires evaluating **Whole-Pass AOT (`sharc-aot` → `rustc -O`)** with native lowering of `DirectMem`/`DagMem`/SIMD/`RegMove`/`RegSwap`/`ShiftImm`/`Multifunction` paired with a lock-free spin-barrier multi-core pass scheduler.

## Higher Goal
Establish a hard go/no-go real-time CPU budget for running all SHARC cores concurrently at 48 kHz using Whole-Pass AOT (or lowered JIT) and a lock-free multi-core pass scheduler.

## Acceptance Criteria
- [x] Prototype **Whole-Pass AOT** (`sharc-aot` → `rustc -O`) with native lowering for `DirectMem`/`DagMem`, `SIMD`, `Move(Other/RegSwap)`, `ShiftImm`, and `Multifunction`, verified 1:1 against the interpreter on a real pass (absorbing PR #466 `D118` milestones)
- [ ] (partial: PNO c1/c2 and ORG c2 measured; ORG c1, FX, SY1SY2, SMP blocked on presets/ISR job, see research note) Run a 300-pass warmed-up multi-core benchmark across `PNO`, `ORG`, `FX`, `SY1SY2`, and `SMP` on the lock-free 1-thread-per-core `MultiCoreScheduler` (PR #461 `ns4-spj2`, spin-barrier pass sync)
- [ ] (partial: exact and fast-f32 measured on PNO c1/c2 and ORG c2; best 8.0x over budget) Measure per-core pass latency (target `< 46.08 us/pass` or within 128/256/512-frame 48 kHz buffers), p99 jitter, and total CPU usage in both exact-40-bit and fast-`f32` modes
- [x] Record measurements, risks (compile time, dynamic SRAM code invalidation, computed jumps), and go/no-go verdict in `docs/research/`
- [x] Unit tests verifying Whole-Pass AOT lowering and the multi-core benchmark harness on synthetic SHARC programs

## Out of Scope
Full ARM0 integration (`ns4-lle5`); SPORT audio routing (`ns4-lle4`).

## Summary of Changes

- `core/aot_whole.rs`, `core/aot_mem.rs`: whole-pass block discovery and native lowering of DirectMem/DagMem/SIMD/Move/RegSwap/ShiftImm/Multifunction; unit tests compare each lowering with the interpreter on random states.
- `core/aot_emit.rs`, `aot_hot.rs`, `aot_glue.rs`, `build.rs`, `sharc-aot --whole-pass`: chunked module emission, multi-module embedding (`NORD_SHARC_AOT_HOT=TAG=file;...`), `Core::enable_aot_tag`.
- `runner/pass_sched.rs` and `nord-tools` bin `lle_sharc_bench`: lock-free 1-thread-per-core spin-barrier scheduler and the 300-pass benchmark.
- Result: ORG Core2 AOT 1:1 with the interpreter over 1 854 395 steps; x1.34 per pass; 8.0x over the 46.08 us budget. Verdict NO-GO for real time, see `docs/research/sharc-lle-realtime.md`.
- Follow-up in the same PR: `core/aot_prof.rs` profile (`lle_sharc_bench --profile`), compute fields of lowered transfers on generated `Pe` kernels (one per distinct op, exact/approx by generation-time tier), gap analysis table and curve in the research note. A block-splitting variant (v3) verified 1:1 but was slower and was reverted.

## Left to do (status stays in-progress)

- Presets/jobs for ORG Core1 (P4I start, own idle pc), FX Core1/Core2 (`FX1`/`FX2` consts exist; add presets + `FxPeer` tick hook), SY1SY2/SMP (ISR job kind in `pass_sched`): see "Missing cores" in `docs/research/sharc-lle-realtime.md`.
- Fast-vs-Exact error in dB on a real pass for the current module build.
- Follow-up lowering lines the profile shows (per-site folded code, registers in locals, cheaper compute) only if a redesign is wanted; two rounds moved ORG Core2 from x8.0 to x7.0 of budget, so the verdict stays no-go.
