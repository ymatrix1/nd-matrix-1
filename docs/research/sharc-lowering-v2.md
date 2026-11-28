# SHARC lowering v2: design note (IR, native locals, superblocks)

Checked: 2026-11-24
Sources: [sharc-lle-realtime](sharc-lle-realtime.md) (measurements, profile, bounded experiment, D249), `crates/nord-sharc/src/core/aot_{whole,lower,emit,mem,pe,glue,hot}.rs`, `runner/pass_sched.rs`, `lle_sharc_bench`; bean `ns4-lle3`.
Question: what lowering design can bring ORG Core2 (6 280 steps per pass), then PNO Core2 and the other cores, to at most x2 of the 46.08 us budget in exact mode, using everything the measurements say?

Tags follow `.agents/rules/provenance-in-code.md`. Cycle figures use ~4.5 GHz (Ryzen 9 5900XT boost, ASSUMED; the 3.3 GHz base clock would make every budget 1.4x tighter).

## 1. What the measurements fix

| Fact | Source | Consequence for the design |
|---|---|---|
| Budget: 46.08 us = ~207 k cycles per pass = 33 cycles per step at 6 280 steps; x2 = 66 cycles per step | arithmetic | every stage gets a cycles-per-step line below |
| A pass runs every generated site once; module code fetched per pass matters (2.5 MB inlined lost to 0.4 MB shared: x7.2 to x5.5) | experiment, step 1 | native locals must not be paid for with big inlined code; code bytes per step is a first-class budget (target <= 24 bytes per step average) |
| Copying element state per compute op was half the op cost (123 to 74 cycles) | step 2 | state must stay in place, never be copied per op |
| Exact 40-bit float op: ~13 ns per element (~58 cycles) in the micro-benchmark; stage B measured 59 cycles per both-element op, 2 831 ops per pass = about 37 us (x0.8) | micro-benchmark, stage B | compute is about 25% of the pass; the rest of the pass sets the limit (see 6); Fast mode is the same IR with an f32 kernel |
| Bail-outs came from interrupt-controller and mode writes; after cutting there are none | step 0 | superblocks must handle these as explicit exits, not guards |
| 842 compute ops per pass still run on `exec_compute_op` (146 cycles) | profile | the IR must cover every compute form, or the pass stays interpreter-bound |
| Interpreter step ~ AOT step (~100 cycles) before step 1 | profile | an interpreter fallback is never a cheap escape; coverage has to be ~100% of executed steps |

## 2. Pipeline

```
executed pcs (census run) -> decode -> IR (per superblock) -> analyses -> schedule -> emit Rust -> rustc -O
```

The old path (`aot_*.rs`) stays unchanged and is the oracle: the new module is accepted only when `sharc_jit_diff --aot2` shows the same state and SRAM as the interpreter at 20 000-step chunks over boot + 300 passes, like the current diff.

New tree `crates/nord-sharc/src/aot2/` (every file under ~400 lines, own unit tests):

| File | Content |
|---|---|
| `ir.rs` | `Op` (SSA-ish three-address ops on typed values), `Block`, `Superblock`, `Exit` |
| `decode.rs` | VISA instruction to IR ops (all forms the passes execute; unconditional and conditional) |
| `cfg.rs` | superblock formation from the executed-pc set: traces across delayed branches, zero-overhead `DO` loops, direct calls |
| `consts.rs` | constant propagation of immediates, DAG modifiers, loop counters; static circular-buffer specialization |
| `flags.rs` | lazy flag materialization and dead-flag elimination |
| `emit.rs` | Rust text for one superblock; Exact/Fast switch |
| `rt.rs` | runtime: `Regs` (native locals struct), cold-path helpers, entry/exit glue |
| `verify.rs` | 1:1 differential harness against the old path and the interpreter |

## 3. IR

Values: `r[0..16]` 40-bit float or 32-bit int views of the data registers (PEx and PEy are two instances, SIMD is "run the same ops on both"), DAG registers `i, m, l, b`, flags, MRF/MRB. Ops are small and total: `Ld/St(addr)`, `AddrPost/AddrPre(i, m|imm)`, `FAdd/FSub/FMul/FSel/...` (one per hardware op, 40-bit semantic), `IAdd/...`, `Shift...`, `Mov`, `Swap`, `Select(cond)`, `Br/Call/Ret/Do`. Each op carries its decode site (pc) for faults and traces. The IR is built once per pass image by tools (`sharc-aot`), never at run time.

Why an IR and not text templates: constant propagation, flag liveness and trace scheduling all need the ops as data; the current emitter prints the interpreter's semantics through `aot_xfer` and cannot see across instructions.

## 4. Stages

Each stage has a cycles-per-step budget (whole pass average, ORG Core2) and must report, on the 300-pass run, the measured value with the 1:1 diff passing.

| Stage | Content | Budget to claim success | Must measure |
|---|---|---|---|
| A. Decode to IR, emit unfused, still per-op calls into existing `Pe`/handlers; every executed form covered (no `aot_step`, no interpreter item) | coverage 100% of the 6 280 steps; the 842 `exec_compute_op` ops and 480 generic steps become IR ops | <= 120 cycles per step (x3.7 of budget; current best is x4.6) | coverage per op kind, steps/s, x budget, code bytes per step |
| B. Registers and flags in native locals per superblock (`Regs` struct on the stack, spilled only at cold calls and exits), dead-flag elimination, lazy flags (only ASTAT bits some later op or exit reads are computed) | no per-op state copy, flags mostly not materialized | <= 70 cycles per step (x2.1) | cycles per step per op class, flags materialized per pass, spill count per superblock, code bytes per step |
| C. Static analysis: constant propagation of loop counters and DAG modifiers, circular-buffer addressing specialized per site (modulo wrap folded when `L`, `B` are constants), address range proofs to drop SRAM bounds checks | addressing ops collapse to `base + const` where provable | <= 45 cycles per step (x1.4) | share of memory ops with proven addresses, bounds checks left |
| D. Superblocks across delayed branches, direct calls and `DO` loops (unrolled when the count is a known small constant, else a native loop); exits only for computed jumps, interrupt-controller or mode writes | block entries per pass from ~430 to <= 40 | <= 35 cycles per step | entries per pass, loop trip counts, dispatch share |
| E. Fast tier: same IR, f32 kernels, chosen at generation time | | measure only (compute share) | Fast vs Exact SNR in dB on a real pass (still open) |

Checkpoint rule from the brief: after A and after B report steps/s and x over budget per core; stop and write the honest result if the curve flattens above x2.

## 5. Per-topic design decisions

**Native locals.** A superblock function owns `Regs { r: [u32;16], s: [u32;16], tag/lo: ..., i,m,l,b, flags, mr }` as a stack struct; LLVM keeps the hot ones in machine registers inside straight-line code. The risk the experiment found: big inlined code loses to instruction fetch. Mitigations: (1) dead-flag elimination and constants shrink each site; (2) cold work (slow memory, faults, interrupts) is one shared out-of-line function per kind, not inlined; (3) repeated op shapes (same op, different registers) are generated as small `#[inline(never)]` functions taking register indices only if the code-bytes-per-step budget is exceeded; the per-stage measurement decides, not a rule.

**Lazy flags.** AZ/AN/AV/AC/AS/AI/SZ/SV/SS and MN/MV/MU/MI are computed from the op result only if a later instruction in the superblock, or the exit, reads them before they are overwritten; sticky bits (AUS, AOS, ...) accumulate in one local word. At exits the live flags are materialized in the same order the interpreter would have left them (state diff must match bit for bit).

**Static analysis.** Pass-level facts are not valid across arbitrary entry: constants are propagated only inside a superblock from its entry state (unknown) plus immediates; facts that need the whole pass (a DAG modifier written once at init) are accepted only as guarded assumptions: entry checks the assumption (one compare) and falls to the old path or interpreter otherwise.

**Superblocks.** Trace formation follows the observed pcs; side exits are explicit `Exit` ops with the exact interpreter state (pc, steps, pending delay slots) rebuilt by a per-exit stub. Delayed branches are lowered as: evaluate condition, run the two delay-slot ops in both arms (duplicated code is bounded by the 2 slots). `DO` loops: the loop-end check disappears inside the trace; arithmetic-condition loops (`UNTIL`) keep a native compare.

**SIMD and dual moves.** SIMD is a superblock-level mode guard (SIMD bit read once at entry; the mode write is an exit, as today). Both elements share one set of address computations. Dual moves (`DM` and `PM` in one word) lower to two memory ops with the interpreter's read-before-write order.

**Memory model.** Internal SRAM: `sram[(addr - 0x90000) * 4]` through a `&mut [u8; N]` of known length (no bounds checks when the range is proven or masked). Everything else: one cold shared function, which also raises the "slow access" exit check (interrupt-controller and device writes). No-peripherals is a precondition of the module (guard at entry as today).

**Exact vs Fast.** One generic parameter `K: Kernel` on `emit`: `Exact` calls `fast_add`/`fast_mul` (existing verified kernels), `Fast` uses `f32` ops with flush-to-zero. The IR and all analyses are identical; only kernel calls differ. Flag semantics under Fast are approximate (documented per op in `aot_native.rs`); never used for the 1:1 oracle diff.

**Determinism and allocation.** No allocation in the pass loop: all buffers are in `Core` or the stack; the emitted functions take `&mut Core` only. Host-independent: no host FP flags beyond the verified kernels; same module, same inputs, same bytes.

## 6. Expected end state and the honest limit

The first version of this section estimated 3 000 ops x 2 elements x 58 cycles = 77 us (x1.7) for compute. That mixed units: the 58 cycles are per element, the op count is per both-element op. Stage B measured 2 831 ops x 59 cycles per both-element op = 167 k cycles = about 37 us (x0.8, 25% of the 161 us pass). The limit is therefore the rest of the pass: with compute free, the other 75% is about 121 us = x2.6, so in Exact mode x2 is not reachable unless transfers, glue and dispatch get cheaper by a further third, which the identified rows (about -15% at best) do not give. Candidate kernel work belongs in stage B/C as a measured sub-item: an exact fast path for operands in the normal range with equal exponents (add) and a table-free 24x24 multiply, both bit-exact by construction and verified against `float_ops` on the existing random-state tests. If after B the curve flattens above x2 (expected: compute-bound at about x2.5), the result is: Exact SHARC LLE does not fit one host core per DSP in real time; Fast tier (stage E) is the live tier and needs its null/listening test.

## 7. Risks

| Risk | Handling |
|---|---|
| Compile time and code size of inlined locals (the first prototype needed 12 min per image; 2.5 MB of code lost) | code-bytes-per-step and `rustc` seconds reported per stage; shared cold paths; chunking per superblock |
| Exactness of lazy flags | the interpreter-state diff at every 20 000 steps; per-op random-state tests like `xfer_equals_the_interpreter_on_random_states` over all flag-live and flag-dead orderings |
| Self-modifying SRAM code | unchanged: entry memcmp of the code snapshot. A store into the code of the block that is running is not an exit (stores re-check the signature only after a non-SRAM access): the change is seen at the next block entry. ASSUMED not to occur inside one block; the 1:1 diff on ORG Core2 holds |
| Host noise (+-15%, other sessions running) | minimum mean of 10 alternating runs per binary; same method as the experiment |
| Scope | stages A to B on ORG Core2 first; PNO Core2 and the missing cores only after the checkpoint |

## 8. Measurement protocol (every reported number)

Three sessions share one host and build concurrently, so:

1. Before each benchmark, check that no `cargo` or `rustc` process runs; note it, re-run if load appears. A run that overlapped load is marked unusable in the table, not averaged in.
2. At least 5 timed runs after a warm-up per binary; report median with min and max, never a single run. (The numbers in [sharc-lle-realtime](sharc-lle-realtime.md) "Bounded redesign experiment" are the minimum mean of 10 runs taken without a load check: treat their x-values as optimistic and re-measure them under this protocol at stage A.)
3. Before and after are built with the same flags and run back to back, interleaved (A B A B), in one session.
4. Pin the bench to one core where possible; record CPU model (Ryzen 9 5900XT), power plan, desktop (no battery), and any throttling seen.
5. A gain counts only if the medians differ by more than the larger of the two min-max bands; otherwise the table says "within noise".
6. Profile counters (`--profile`, `rdtsc` around fallbacks) add overhead: each profile share is validated against an unprofiled wall-clock A/B before it is used to justify a stage.
7. Each checkpoint row records its variance; the x2 stop rule is invoked only when the numbers are outside the noise band.

## 9. Out of scope

`crates/nord-arm`; peripherals, SDRAM, SPORT bus and FX presets (`lle4-bus`); changing the old `aot_*.rs` path (kept as oracle).

## Follow-ups

- Stage A measured (2026-11-24): ORG Core2 x4.57 to x4.13, 1:1 holds; see "Lowering v2 stages". Stage B next.
- Stage B measured (2026-11-24): ORG Core2 x4.10 to x3.5 by cold-path outlining; native locals and lazy flags not built, reasons in "Stage B" of the research note.
- Stage D1 built (2026-11-27, D252): event-aware blocks, 14 refusals and 255 interpreter steps per pass removed, 1:1 holds; D2 and C1 to C3 not built (below noise, see "Stages C and D" in the research note).
- Stage reports append to [sharc-lle-realtime](sharc-lle-realtime.md) section "Lowering v2 stages" with the tables above filled in.
- A decision entry once stage B is measured.
