---
# ns4-lle3
title: 'Multi-core real-time SHARC JIT/AOT benchmark at 48 kHz'
status: in-progress
type: task
priority: high
created_at: 2026-11-23T21:40:00Z
updated_at: 2026-11-27T20:00:00Z
parent: ns4-lle1
---

## Context
`crates/nord-sharc` already has a Cranelift JIT that matches the interpreter 316/316 passes on PNO core 1 (`sharc-sys --jit-diff --strict`, improved by PR #460 `ns4-z2fq` loop SSA promotion) and a 1-thread-per-core `MultiCoreScheduler` (PR #461 `ns4-spj2`). However, PR #466 (`D118`) and `docs/research/sharc-aot-multi-block.md` showed that basic-block JIT/AOT still leaves ~56.7% of steps in `Core::step()` fallback (~11–16 Msteps/s vs ~93–129 Msteps/s needed for `46.08 us/pass`). Real-time 48 kHz execution across all active SHARC cores (`ORG c1/c2`, `PNO c1/c2`, `FX c1/c2`, `SY1SY2`, `SMP`) therefore requires evaluating **Whole-Pass AOT (`sharc-aot` → `rustc -O`)** with native lowering of `DirectMem`/`DagMem`/SIMD/`RegMove`/`RegSwap`/`ShiftImm`/`Multifunction` paired with a lock-free spin-barrier multi-core pass scheduler.

## Higher Goal
Establish a hard go/no-go real-time CPU budget for running all SHARC cores concurrently at 48 kHz using Whole-Pass AOT (or lowered JIT) and a lock-free multi-core pass scheduler.

## Acceptance Criteria
- [x] Prototype **Whole-Pass AOT** (`sharc-aot` → `rustc -O`) with native lowering for `DirectMem`/`DagMem`, `SIMD`, `Move(Other/RegSwap)`, `ShiftImm`, and `Multifunction`, verified 1:1 against the interpreter on a real pass (absorbing PR #466 `D118` milestones)
- [ ] (partial: PNO c1/c2 and ORG c2 measured; ORG c1, FX, SY1SY2, SMP blocked on presets/ISR job, see research note) Run a 300-pass warmed-up multi-core benchmark across `PNO`, `ORG`, `FX`, `SY1SY2`, and `SMP` on the lock-free 1-thread-per-core `MultiCoreScheduler` (PR #461 `ns4-spj2`, spin-barrier pass sync)
- [ ] (partial: exact and fast-f32 measured on PNO c1/c2 and ORG c2; best per core: PNO Core1 x1.3 to x1.5 on the interpreter; slowest ORG Core2 x3.5 with AOT, PNO Core2 x6.5 to x6.7 on the interpreter; the first prototype was 8.0x) Measure per-core pass latency (target `< 46.08 us/pass` or within 128/256/512-frame 48 kHz buffers), p99 jitter, and total CPU usage in both exact-40-bit and fast-`f32` modes
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

- Progress note (2026-11-25): exact float kernels made about 2x cheaper (add/sub 30 -> 15 cycles, mul 30 -> 12, normal range, bit-exact against `float.rs`); see `docs/research/sharc-kernel-speed.md`. Projected x1.7 -> x1.22 of budget for the exact path; not yet measured in the whole-pass AOT. Criteria unchanged.
- Bounded speed experiment (D249, `docs/research/sharc-lle-realtime.md` "Bounded redesign experiment"): blocks end at guard-changing instructions (bails 29 to 0 per pass), one shared handler per transfer shape (module code ~2.5 MB to ~0.4 MB), compute in place on the core registers (`Pe<'a>`). ORG Core2 x7.2 (min of 10 runs, no load check, a best case) to x4.6 of the 46.08 us budget (fast-f32 x4.0, not diffed 1:1), 1:1 with the interpreter over 1 854 395 steps in exact mode. Stopped at the x3.5 checkpoint: x2 is not reachable in exact mode with this design. PNO Core2 (interpreter) is now the slowest measured core at x6.7; PNO Core1 x1.5. Acceptance criteria 2 and 3 stay unticked (cores still missing, no core under budget).
- Lowering v2 stage A (`docs/research/sharc-lowering-v2.md`, research note "Lowering v2 stages"): fixed ALU, float ALU and shifter ops on `Pe`, loop-end blocks; `exec_compute_op` ops per pass 842 to 88, ORG Core2 x4.57 to x4.13 (medians of 7 interleaved runs, outside noise), 1:1 with the interpreter.
- Lowering v2 stage B (research note "Stage B"): the handler tail and pending-exception raise moved out of line, register writes inline: ORG Core2 x4.10 to x3.5 (medians of 7 interleaved runs; B1 is outside noise, B2 against B1 is inside it and no gain is claimed for B2), exact mode 1:1 with the interpreter; fast-f32 x2.93 is a min of 7, not a median, and is NOT diffed 1:1 (approximate kernels). Native locals and lazy flags not built: compute is 25% of the pass and flags are under 1% of it, and locals conflict with shared handlers (details in the note). x2 not reached (with compute free the other 75% is still about x2.6); criteria stay unticked.

- PNO Core2 AOT module (2026-11-27, `docs/research/sharc-lle-realtime.md` section "PNO Core2 AOT module"): census 4 069 pcs, module generated with `sharc-aot --whole-pass`, `AOT DIFF OK: 265 blocks, boot 100000 steps + 300 passes (1279800 steps) matched 1:1` on the first run (no lowering gap, no source change). Idle-state pass (no message script). Median of 9 interleaved runs on a loaded host (74 to 87% CPU from other sessions, no compiler running): PNO Core2 interpreter 346.1 us (x7.51) to AOT exact 135.1 us (x2.93, band 125.2..143.2), fast f32 119.2 us (bands overlap, no gain claimed); ORG Core2 AOT exact 179.5 us (x3.90), PNO Core1 interpreter 60.3 us (x1.31). Three-core need 8.1 budget periods (11.4 before). ORG Core2 is the slowest core again; real time and x2 not reached, hybrid verdict unchanged. Profile shares and their partial validation in the note; no further step was above the noise band of the loaded host, so none was built. Criteria unchanged.

## Left to do (status stays in-progress)

- Presets/jobs for ORG Core1 (P4I start, own idle pc), FX Core1/Core2 (`FX1`/`FX2` consts exist; add presets + `FxPeer` tick hook), SY1SY2/SMP (ISR job kind in `pass_sched`): see "Missing cores" in `docs/research/sharc-lle-realtime.md`.
- Fast-vs-Exact error in dB on a real pass for the current module build.
- A re-measure on a quiet host; a PNO Core2 message script (loaded pass); lowering of the 475 generic `aot_step` steps (stage A already cut the `exec_compute_op` ops from 842 to 88 per pass); the verdict stays no-go for real time in exact mode.
