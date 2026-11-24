---
# ns4-lle1
title: 'Phase 8: Full Real-Time ARM + SHARC Emulation (LLE)'
status: todo
type: epic
priority: normal
created_at: 2026-11-23T21:40:00Z
updated_at: 2026-11-23T21:40:00Z
---

## Context
Track B (`ns4-trkb`, `crates/nord-sharc`) already boots all five DSP images (`ORG`, `PNO`, `FX`, `SY1SY2`, `SMP`), matches the interpreter 316/316 passes in Cranelift JIT on PNO core 1 (`docs/research/pno-jit-arm-profile.md`), and runs 152/155 piano programs through `SMP` CNSP page refill (`ns4-pnoc`), while the ARM0 firmware boots in Unicorn (`tools/arm_*.py`).

This epic **supersedes the earlier unmerged D118 proposal (PR #466 / epic `ns4-zp6a`)** and folds its key compiler/scheduler findings into a unified end-to-end plan:
- PR #466 + `docs/research/sharc-aot-multi-block.md` showed that basic-block JIT/AOT leaves too many steps in `Core::step()` fallback (~11–16 Msteps/s vs ~93–129 Msteps/s needed for `46.08 us/pass`), so **Whole-Pass AOT (`sharc-aot` → `rustc -O`)** with native `DirectMem`/`DagMem`/SIMD/`RegMove`/`RegSwap` lowering and a **lock-free spin-barrier multi-core pass scheduler** (building on PR #460 `ns4-z2fq` and PR #461 `ns4-spj2`) are required for Gate 1b (`ns4-lle3`).
- Promoting Track B to an **opt-in real-time ARM + SHARC low-level emulator (LLE)** delivers fidelity that tracks future OS updates (Exact mode: bit-exact) while containing zero vendor DSP routines or lookup tables. Real-time play may use the cheaper Fast (`f32`) mode, which is not bit-exact and is accepted only within a measured tolerance against Exact.

This epic gates the full wiring behind a **Gate 1 feasibility & speed benchmark** (`ns4-lle2`, `ns4-lle3`) to prove real-time 48 kHz viability before committing to full GUI/plugin integration.

## Higher Goal
Run the complete Nord Stage 4 hardware architecture (ARM0 + all 5 SHARC DSP images) in real time inside a single Rust engine from the user's own OS update and `.ns4b` backup, with zero vendor code or tables in the repository.

Two precision modes share one lowered code path (compile-time switch, not two implementations): **Exact** (emulated 40-bit extended-precision float; bit-exact; the oracle and the default for offline renders, tests and CI comparisons) and **Fast** (native `f32`; real-time play; audibly equivalent within a stated error bound, for example max dB error over a reference render, measured and recorded per release). The user picks the mode in the UI (Settings and plugin parameter), as other emulators expose accuracy modes; Fast is never used for oracle comparison.

## Gate 1 outcome (2026-11-24)
SHARC whole-pass AOT prototype: bit-exact, NO-GO for real time (slowest core x7.0 to x8.0 over the 46.08 us budget; `docs/research/sharc-lle-realtime.md`). ARM0 Rust runtime: mean 0.20 to 0.34 ms per 1 ms tick, tail unproven (`docs/research/arm0-rust-runtime.md`). Gates 2 and 3 continue as slower-than-real-time LLE (oracle, offline render, null tests) plus a hybrid that runs LLE only for cores that fit; real-time pure LLE stays open until a redesign shows x2 or better.

## Acceptance Criteria
Gate 1 — Real-time feasibility & speed:
- [ ] `ns4-lle2` Rust in-process ARM0 emulator with idle-skipping (replaces Python/Unicorn harness; hits < 1 ms per 1 ms tick)
- [ ] `ns4-lle3` Multi-core real-time SHARC JIT/AOT benchmark at 48 kHz (PNO + ORG + FX + SY1SY2 + SMP in parallel within a 5–10 ms audio buffer), in both Exact and Fast modes, reporting the Fast-vs-Exact error on a real pass

Gate 2 — Unified multi-chip audio & memory:
- [ ] `ns4-lle4` Live chip-to-chip `SPORT`/audio DMA bus (`ORG`/`PNO`/`SY`/`SMP` → `FX Core1/Core2` → stereo out), FX Core 2 poll-loop fix, and external SDRAM (`0x20xxxxxx`) delay/reverb window
- [ ] `ns4-lle5` Unified Rust multi-chip LLE engine orchestrating ARM0 + all 5 DSP images + DSP→ARM return path (`ns4-nds8`) + `SMP` CNSP page refill (`ns4-pnoc`)

Gate 3 — Playable instrument & architectural decision:
- [ ] `ns4-lle6` Wire live MIDI, pedals, panel controls, `.ns4b` program store (`ns4-pyqg`), and real OLED framebuffer (`ns4-2gdc`) into the LLE engine
- [ ] `ns4-lle7` Expose opt-in LLE audio backend in `nd-matrix-1` / `nord-plugin` and record the D2/D119 architectural decision (whether LLE stays opt-in or replaces firmware-derived HLE paths, allowing deletion of all Class B/C files); include a user-facing precision selector (Exact 40-bit / Fast f32) in Settings and as a plugin parameter, persisted in the user config

## Out of Scope
Shipping vendor firmware, samples, or presets; bypassing any encryption or DRM; removing the clean-room `CBIN`/`.ns4p`/`.ns4b` file-format toolkit.
