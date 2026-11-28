---
# ns4-arm-sb
title: 'ARM0 superblock tiers (trace superblocks, optional native tier) and exact idle skipping v2'
status: in-progress
type: task
priority: high
created_at: 2026-11-28T08:00:00Z
updated_at: 2026-11-28T08:00:00Z
parent: ns4-lle1
---

## Context
The Rust ARM0 runtime (`ns4-lle2`, `crates/nord-arm`) runs the OS v1.66 image at 17 thread cycles per guest instruction (5.3 ns, EMPIRICAL, PR 689 note) with the predecode cache. Its criterion "every tick under 1 ms" is still open: heavy ticks run 6.5 M instructions of real work, and a steady idle or key tick executes only 3.3 to 3.4 k instructions (about 18 us) but costs 63 to 72 us, so 45 to 48 us per tick is a fixed cost that nothing explains yet. `docs/research/arm0-jit-design.md` picks the design (ARM-SB spine, ARM-X native path grafted): a tier of trace superblocks over the predecode cache in safe Rust (T2), an optional Cranelift tier compiled from the same IR behind feature `sb-native` (T3), and exact idle skipping v2 (idle v2 stage 1 needs no tier and ships first). Every tier is bit-exact against `Cpu::step`, off by default, and falls back to the interpreter's own handler for anything not lowered. Spec: `docs/specs/arm0-superblocks.md`.

## Higher Goal
Keep the ARM0 thread well under one host core in steady state and every heavy tick under 1 ms, bit-exact against the interpreter, on Windows, macOS and Linux, without a new dependency or `unsafe` in the default build. Feeds the hybrid verdict of `ns4-lle3` and the real-time claim of `ns4-lle1`; ARM never lifts a SHARC NO-GO.

## Acceptance Criteria
Gates of design section 13 (thresholds are ASSUMED decision thresholds; none has run). Wall-clock numbers come only from the single A/B of G2 and G3, valid below 25 % host load; every other check uses deterministic counters.
- [ ] G0 counters (`arm-sb-bench`, no tier needed): per phase `probe_executed / executed`; fixed tick cost split (tick time minus executed instructions x 5.3 ns); executed instructions per tick at the 24 kHz framing; dynamic Thumb and plain-step share; budget calibration if a delay-loop reference exists. Rules: share >= 30 % in idle and key phases means idle v2 stage 1 is switched on first; share < 30 % means W3-LOOP waits for a diagnosis; share < 30 % with fixed cost < 10 us and a true budget <= 200 k means T2 and T3 stay built but off; Thumb > 3 % opens a Thumb front end follow-up
- [ ] G1 exactness (blocks every other gate and every default-on decision): L0 >= 1 M words per family and L1 >= 2,000 programs locally with zero divergence; L2 golden firmware identical at 100/150/200/300/456 k for T2 on, off, `hot = 0`, cap 3 and idle v2 per tick (1,900 ticks); L3 lockstep clean over boot and 6 phases; L5 mutation kill rate >= 95 %. An unexplained divergence reverts its family to `GENERIC`; more than 3 reverted families, or one above 1 % of dynamic instructions, stops T2 and T3
- [ ] G2 tier 2 (after the seam): coverage >= 85 % of guest instructions in traces and `GENERIC` <= 8 % on the load phase; >= 15 guest instructions per trace entry (T1: 7.8); T1-slice share <= 15 %; A/B heavy ticks T2 >= 1.35x T1 with non-overlapping min..max ranges (below 1.25x T2 stays off and is fallback and oracle only)
- [ ] G3 tier 3 (after G1 and the three-OS spike): native exits <= 1 % of dynamic instructions on the load phase; A/B T3 >= 2.5x T1 and >= 1.8x T2; heavy tick under 1 ms at 300 k instructions in the least disturbed run; synchronous compile of the heavy region <= 50 ms or hidden by background mode; `Machine: Send` and the `native::available()` fallback tested (exits > 3 %, or < 2.0x T1, or a spike failure on any CI OS without a clean cfg fallback keeps T3 off in releases)
- [ ] G4 steady state (best enabled tier plus idle v2): steady tick thread CPU <= 10.4 us at the 24 kHz framing (0.25 core) and <= 25 us at the 1 ms framing (about 40 % of today's 63 to 72 us)
- [ ] G5 system fit (with the SHARC track's current per-core numbers): ARM thread <= 0.5 core steady and <= 1.0 core in the heavy stretch at the calibrated budget; ARM + SHARC <= 16 - 2 cores
- [ ] Stop rule after G3: one design review; default-on stays blocked until D246 Gate 3 and the D22 `render_hash` re-baseline rules are met
- [ ] One DECISIONS entry (W4-DOC) for in-memory runtime translation of the user's own image (nothing persisted, no firmware bytes, no hot addresses, no generated code in the repository; same wording as the SHARC track), superseding the deferral of D245 and D250 for the tiers that pass their gates; ROADMAP and these checkboxes updated

## Out of Scope
- A Thumb front end (only if G0 shows more than 3 % dynamic Thumb, follow-up F2).
- An MMU or cache model, a software TLB for T2 memory uops (F4), a T3 helper-call slow path (F5), zeroing the low 64 KB only when dirty (F6), an RX arena replacing whole-module reset (F7), background compile (F8).
- Calibrating the true instruction budget per ms and the 24 kHz against 1 ms framing (F1, F10); capping the producer's lead outside heavy stretches (F3); repeating every A/B on a quiet host with raised timer resolution (F9).
- The SHARC engine (D260, `docs/specs/sharc-uop.md`) and live DSP return layers (`ns4-lle5`).
- Editing the files other open ARM work touches (`machine/{mod,fastrun,consumer,producer,runstats,boot,layout,panel,system,uart,dsp_return}.rs`, PR 689, 687, 683) before the seam package (W4-SEAM), which is last and at most 30 changed lines.
- Making any tier the default.

## Summary of Changes
- Design: `docs/research/arm0-jit-design.md` (2026-11-28, nothing built or run for it): tier ladder, structures, exits, invalidation, exactness layers L0 to L6, cost model (ASSUMED), work packages, gates G0 to G5, risks R1 to R19.
- Skeleton (S0, commit 1b06822a): `crates/nord-arm/src/machine/sb/` with the frozen `Uop` IR (16 bytes, 141 allocated kind ids), `Sb`/`Exit`, config and counters, decode registry, bus and backend traits, a correct but unoptimised cache, the tier driver and executor stubs (`GENERIC`, `GENERIC_END`, `EXIT` only), differential test kit, `arm-sb-bench` and `arm-sb-ab` skeletons in `nord-tools`, and the optional Cranelift tier under `native/` behind feature `sb-native` (default off). Everything is off by default and no existing code path calls into it; with every lowerer a stub the stack equals the interpreter by construction.
- W1-DOC (this bean): `docs/specs/arm0-superblocks.md` (structures, exits, invalidation, exactness contract L0 to L6, gates, ownership) and the `docs/specs/README.md` row. W4-DOC ticks the criteria above and adds the DECISIONS, ROADMAP, LEGAL and TECH_DEBT hunks.
- Open: lowering and executor families, driver, cache revalidation, idle v2 stages, verification layers and the native tier are separate work packages (spec section 11); no gate has run. S0 allocated no kind id for `MUL`/`MLA`/long multiplies, so they stay `GENERIC` until a design review allocates ids (spec section 12).
