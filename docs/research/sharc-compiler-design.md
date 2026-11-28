# SHARC compiler design: exact two-tier uop engine (Phase 8 LLE)

Checked: 2026-11-28
Sources: `origin/main` at `80748ba5` (the worktree `HEAD` equals it; #685 is merged, so the exact kernels `aot_fast`, `aot_fadd`, `aot_round`, `aot_simd`, `aot_simd_lanes` and `aot_simd_core` exist): `crates/nord-sharc/src/core/{mod,new,exec,decode_cache,aot_hot,aot_pe,aot_whole,aot_glue,aot_state,branch,interrupts,memory,census,jit,sysregs,types}.rs`, `core/periph/glue.rs`, `runner/{pass_sched,jit_diff,mod}.rs`, `crates/nord-sharc/{Cargo.toml,build.rs}`, `tools/check_tests.py`, `tools/check_fw_derived.py`, `.github/workflows/ci.yml`; `docs/research/{sharc-lle-realtime,sharc-lowering-v2,sharc-kernel-speed,sharc-jit-ceiling-study,arm0-rust-runtime}.md`; `docs/DECISIONS.md` (D20, D128, D129, D246, D249; highest id D259, D257 and D258 absent). Four candidate designs (S-A Cranelift, S-B rustc AOT, S-C uop interpreter, S-D parallel executor) scored by three judges. No new measurement was taken for this document.
Question: which architecture gives the best exact (bit-exact against `Core::step`) SHARC execution for ORG, PNO, FX, SY1SY2 and SMP (8 cores), how close to real time can it get on the worst core (ORG Core2), what must be built, in which disjoint work packages, and which measured result stops the exact real-time attempt and locks the hybrid.

Tags: every number below is `VERIFIED` (matches a source by a named check), `EMPIRICAL` (measured in an existing research doc, cited), or `ASSUMED` (estimate or threshold chosen here, with what would confirm it). Cost figures are model outputs, not measurements.

## 1. Result

Build a two-tier engine over one shared block format.

| Tier | What it is | Where | Expected ORG Core2 pass (ASSUMED) |
|---|---|---|---|
| Tier 1 | Predecoded `uop` block interpreter (S-C). In-place PEx/PEy state, handler table, static step budgets, exact bail-out to `Core::step`. Always correct, no unsafe codegen. | `crates/nord-sharc/src/core/uop/` | about 340k cycles, x1.65 of the 46.08 us budget (v1 AOT is x3.5) |
| Tier 2 | Cranelift native emission from the same uop blocks (S-A backend), feature `uop-native`. Starts call-threaded, inline families added one at a time. | `core/uop/native/` | about 236k cycles, x1.14 raw, x1.3 likely |

Tier 1 is the exactness baseline, the fallback, the oracle for Tier 2 and a 2x faster replacement for v1 whatever Tier 2 does. Tier 2 is the only route to real time. The cost model says exact real time on ORG Core2 is not expected (x1.14 raw against a budget of x1.0 and a claim threshold of x0.85). The hybrid verdict recorded in `docs/research/sharc-lle-realtime.md` stands unless the Tier 2 gates in section 12 pass. This plan finds out cheaply: Tier 1 ships first, K0 and K3 gate the expensive part.

The v1 `aot_*.rs` path is untouched and stays the regression oracle for output. The oracle for exactness of state is `Core::step`, never v1 (v1 `Core::state_mismatch` compares only the effective f40 value, section 6).

Answer to the request "implement everything in one go, benchmark later" (faz sentido): yes, with one condition. Exactness checks are not the thing to postpone, because they are what makes fixing later cheap. Wall-clock benchmarks and per-commit perf runs are postponed. The wave plan in section 11 runs unit tests per package (cheap, deterministic), runs deterministic counters at wave ends, and takes wall-clock numbers once per tier at a named gate.

## 2. Budget arithmetic

| Quantity | Value | Tag | Derivation |
|---|---|---|---|
| Pass budget | 46.08 us | VERIFIED | 2 frames / 43,402.34 Hz = 4.608e-5 s, matches `sharc-lle-realtime.md` |
| Host clock for cycle math | 4.5 GHz | ASSUMED | dev machine class; confirm with the owner's CPU before reading any cycle number as time |
| Cycles per budget | about 207k | ASSUMED | 46.08 us x 4.5 GHz = 207,360 |
| ORG Core2 steps per pass | 6,280 | ASSUMED | brief; `sharc-jit-ceiling-study.md` records 5,953 for an 8-voice ORG pass |
| Cycles per step to hit budget | 33 | ASSUMED | 207,360 / 6,280 |
| v1 AOT ORG Core2 | x3.5 = 161 us | EMPIRICAL | `sharc-lle-realtime.md` stage B table |
| v1 PNO Core2 | x2.93 | EMPIRICAL | same doc |
| Spike floor, inline kernels | 675k x86 insns, x0.96 | EMPIRICAL | `sharc-jit-ceiling-study.md` as summarized in the brief; IPC 3.37 calibrated |
| Spike, shared kernels | 853k insns, x1.2 | EMPIRICAL | same |
| Compute share of spike insns | 599k of 675k | EMPIRICAL | same |
| Average kernel | 684-785 insns, 243.6 ops per pass, 27% of the ideal pass | EMPIRICAL | same |
| f32 add / mul kernel | 3.3 ns / 2.7 ns per element | EMPIRICAL | `sharc-kernel-speed.md` (f64 add trick plus `round_norm`) |

Reading: even the ideal inline-kernel floor is at x0.96, so there is no slack for any overhead. Every design decision below is measured against that floor.

## 3. Architecture and why

### 3.1 Scores and decision

| Design | Idea | Judge score | Verdict |
|---|---|---|---|
| S-C | predecoded uop interpreter, handler table, exact bail-out | 88 | base (Tier 1) |
| S-A | Cranelift native emission, exact kernel diet | 82 | Tier 2 backend, grafted onto S-C blocks |
| S-B | rustc AOT of the firmware, whole-program | 62 | oracle and ceiling probe only |
| S-D | parallel executor, P <= 2 | 41.5 | rejected, optional read-only probe |

All three judges ranked S-A first for the sharc track, the script score ranked S-C first. The two are reconciled instead of picked between: S-C and S-A are the same pipeline at two depths. Both need predecoded blocks, step budgets, exact exits, an event contract, an invalidation scheme and the exact kernels. Building S-C first produces all of that. S-A then only adds a second consumer of the same `Block` array. This avoids a second IR, a second lifter and a second evaluator, and gives S-A an always-correct baseline to validate against.

### 3.2 Strongest objections of the losing designs, answered

| Objection | Source | Answer |
|---|---|---|
| "An interpreter cannot meet a 33 cycles/step budget; dispatch alone eats it." | S-A | Agreed for ORG Core2, and the plan does not claim it. Dispatch is about 15% of Tier 1 cost (section 9). Compute kernels are about 60% and are shared with Tier 2. Tier 1 still beats v1 by about 2x and is the baseline Tier 2 reuses. |
| "A second tier doubles the work." | S-A | Tier 2 consumes the Tier 1 `Block` array, handlers, exit protocol, event contract and verification harness. New work is the emitter, the code cache and the translation service. |
| "Cranelift compile time and code size on the audio path." | S-A | Translation runs off the audio path on a worker, installed at a pass boundary. Gate K5: <= 5 s single-thread for ORG Core2. Blocks run Tier 1 until their native version is installed. |
| "rustc AOT gives the best code and zero dispatch." | S-B | It cannot ship: the generated Rust is firmware-derived, D20 and D128 forbid tracking it, and `check_fw_derived.py` only lets the baseline shrink. v1 already is this path (x3.5; rustc peaks near 19 GB). It also is a pre-decoded interpreter, not a compiler. Kept as an oracle and as a ceiling probe, not rebuilt. |
| "Run the two elements and independent cores on threads." | S-D | Dependencies inside a pass limit P to <= 2 and the sync cost eats the gain; the repo's own threading attempts were dropped. Rejected. Only the per-block read/write footprint analysis is kept (section 4.6), usable for a future read-only probe. |
| "S-C state in memory loses to register-resident state." | S-A | True. Tier 2 stage K2 promotes block-local registers with write-back at every exit; the exit sweep (section 6.5) is what makes that safe. |
| "uop handlers cannot be exact for hidden 40-bit state." | judges | The comparator compares hidden tag/lo, not only the effective f40, and handlers reuse v1 `Pe<'a>` semantics (verified by census) until a kernel passes the diet tests. |

### 3.3 Grafts absorbed

| Graft | From | Where it lands |
|---|---|---|
| Cranelift emission over uop blocks | S-A | `core/uop/native/`, feature `uop-native` |
| Exact fast kernels: avg, select, compare, convert diet | S-A | `core/uop/kernels/`, shared by both tiers |
| Exit sweep (run every block to every uop boundary and compare) | S-A | `verify/sweep.rs` |
| Translation validation on the worker thread | S-A | `native/tv.rs`, `verify/tv.rs` |
| Mutation testing of the verifiers (output-perturbation wrappers) | S-A | `verify/mutate.rs` |
| Dev-only `Engine::UopChecked` | S-A | `runner/uop_pass.rs` |
| Exact comparator with hidden state, census replay, lockstep on real traces | judges | `verify/{compare,census,lockstep}.rs` |
| Deterministic counters instead of wall-clock | S-B | `runner/uop_report.rs`, `tools/uop_gates.py` |
| rustc AOT as oracle and K0 ceiling probe | S-B | v1 untouched, `kernel-bench` style probes |
| Per-block register footprint (reads, writes) | S-D | `lower/builder.rs` `Footprint`, used by regpromo and sweep |
| Optional read-only parallel probe | S-D | not built; footprint is the only hook |

Out of scope: Fast f32 mode (non-exact), any change to v1, any shipped firmware-derived code.

## 4. Data structures and state layout

All new code lives in `crates/nord-sharc/src/core/uop/`, a child module of `core`, so it sees the private `Core` fields and every `pub(super)` item of `core` (`exec_cached`, `finish_step`, `aot_pe::Pe`, `CachedInsn`, `invalidate_sram_cache`). Where an item is private to its file (visibility narrower than `pub(super)`), W0 widens it to `pub(super)` in a one-line edit.

### 4.1 `Uop` (16 bytes)

| Field | Type | Meaning |
|---|---|---|
| `h` | `u16` | handler index = `UopKind.0` |
| `steps` | `u8` | steps this uop accounts for: 1 for a normal instruction, 0 for the second uop of one instruction, 2 or 3 for a fused group |
| `flags` | `u8` | `UF_FIN` (call `finish_step` after), `UF_CB` (call `latch_cb_overflow` after), `UF_LAST` (block terminator), `UF_SIMD` |
| `a`, `b` | `u32` | operands: register indexes, immediate, packed compute word, constant-pool index |
| `pc` | `u32` | pc (short-word units) of the first instruction the uop covers, used by every cold exit |

`Uop` is `#[repr(C, align(16))]`; a `const` assert pins `size_of::<Uop>() == 16` (VERIFIED by the skeleton's `uop_is_16_bytes` test).

### 4.2 `Block`

Blocks are non-overlapping basic blocks stored in an arena. Fields: `pc`, `last_pc` (pc of the last uop's instruction), `end_pc` (pc after the last instruction: where `Exit::Ok` at the end of the block continues), `steps` (static budget `S`), `first`/`len` (range into `UopHot::arena`), `cum: first..` (cumulative steps before each uop, `u16`, used for mid-block re-entry), `simd: bool` (translated for PEy on or off), `isa` (VISA flag from `isa_at`), `guard: GuardClass`, `links: [Link; 2]` (`Link {id, epoch}`), `code_hash: u64`, `native: u32` (index into the native table, `NONE` when absent, present in all builds to avoid cfg in the struct), `poisoned: bool`, `footprint: Footprint`.

Cut rules (`lower/cut.rs`), a block ends after: any flow instruction (jump, call, return, DO setup when the loop end is outside, IDLE), any system-register write or read with side effects (`Sys`), the last instruction of a statically known loop body (`UF_FIN` terminator), an instruction lowered as `Interp` that can reach `poke_irq`, `max_block_steps`. A cut never separates a delayed branch from its owed delay slots (`BlockBuilder::owe_slots`, P1). `stop_pc` (the pass idle pc) is a block START and never an interior pc: the pass ends when a step lands on it, so the translator cuts before it, and a `stop_pc` learnt after translation is repaired by `uop_begin_pass` (P7). Every range is translated twice, once per PEy mode (7.1, P4). A block starts at: a pass entry, `stop_pc`, a static jump or call target, the instruction after a terminator, the first instruction of a loop body. Dynamic targets (computed jumps, returns) land on a block start or on a uop boundary found through `PcMap`.

### 4.3 State: `UopRun` (in `Core`, by value) and `UopHot` (boxed)

`Core` gains two fields: `uop: Option<Box<UopHot>>` and `uop_run: UopRun`.

| Struct | Lives | Holds | Why separate |
|---|---|---|---|
| `UopRun` | `Core.uop_run`, always present, no heap | `cfg: UopConfig`, `stats: UopStats`, `code_pages: CodePages` (bitmap), `flags: u32` (bit0 `smc_dirty`, bit1 `shadow`, bit2 `trace`), `fault: Option<Fault>`, `stop_pc: u32`, `dirty: u64`, `maintain_calls: u64` | the invalidation hook runs while `UopHot` is taken out; it must reach its state through `Core` alone |
| `UopHot` | `Core.uop`, `Option<Box<_>>` | `arena: Vec<Uop>`, `blocks: Vec<Block>`, `pcmaps: [PcMap; 2]` (one per PEy mode, indexed by `usize::from(block.simd)`, read through `pcmap(simd)`), `dropped: Vec<(u32, u32)>` (ranges waiting for `uop_maintain`; `install` reserves room for every block so `drop_block` never allocates), `handlers: Box<[Handler]>`, `page_blocks: Vec<Vec<u32>>`, `epoch: u32`, `native: Option<Box<NativeTable>>` | `uop_try` does `self.uop.take()`, runs, puts it back; the borrow of `&mut Core` and `&UopHot` never overlap |

`CodePages`: one bit per 256 short words over the short-word window, `[u64; 64]` = 512 bytes (page shift 8 is ASSUMED; the window size comes from `core::memory`, 1 Mi short words; test `code_pages_cover_short_window` pins it). `PcMap`: two-level, 256-entry leaf pages allocated lazily, entry `(block_id, uop_idx)` or `NONE`.

### 4.4 Handler ABI

```rust
pub type Handler = unsafe extern "C" fn(ctx: *mut UopCtx, u: *const Uop) -> u32; // returns Exit as u32
```

`UopCtx` is `#[repr(C)]` and holds only integers and the raw `*mut Core`: `core`, `base_steps` (steps at block entry), `done` (steps accounted so far in the block), `room` (steps left before `min(limit, irq.due)`), `next_pc`, `exit_idx`. Handlers get all state through `(*ctx).core`. One raw pointer is derived from the `&mut Core` at `run_block` entry and is the only access path inside the block (aliasing discipline, section 14 risk 10). Handlers never panic: no indexing without bounds proof, `wrapping_*` arithmetic, `debug_assert!` only. The same `Handler` type is what Tier 2 calls from native code (call-threaded) and what its inline families are validated against.

### 4.5 Exit protocol

```rust
#[repr(u32)] pub enum Exit { Ok = 0, Jump = 1, Fault = 2, Guard = 3, Smc = 4, Slow = 5 }
```

| Exit | Raised by | Driver action |
|---|---|---|
| `Ok` | uop finished | continue with the next uop |
| `Jump` | terminator uop wrote `ctx.next_pc` | materialize, follow `links` or look up `PcMap` |
| `Fault` | handler's precondition says this instruction would fault | materialize at `u.pc`, replay through `Core::step`; the real `Fault` comes from the interpreter, so partial effects are exact by construction; stored for `uop_take_fault` |
| `Guard` | block-entry guard failed (simd mismatch, pending branch, loop frame, window, trace) | no uop ran; one `Core::step`, retry |
| `Smc` | a store hit a code page (`uop_run.flags & smc_dirty`) after the store completed | materialize after the store uop, return to the outer loop, revalidate dirty pages |
| `Slow` | `irq.due` shrank, uop needs the interpreter, exit sweep stop | materialize at `u.pc`, one `Core::step`, retry |

Rules.
1. Precondition before mutate: a handler checks everything that can bail before it writes any state. A bailing handler leaves `Core` exactly as it was at the uop boundary.
2. Materialize at a uop boundary: `core.pc = u.pc`, `core.steps = base_steps + done`. The dispatch loop adds `u.steps` to `done` after each `Ok` uop (one add per uop, constant-folded at every exit in Tier 2).
3. Any instruction whose interpreter path can fault after mutating state is lowered as `Interp` (never as a fast uop), so a `Fault` exit never needs to undo anything.
4. Every uop that can reach `poke_irq` (`Interp`, `Sys`, generic memory outside the SRAM windows) ends with `if core.irq.due < base_steps + done + remaining { return Slow }` where `remaining` is the static step count of the rest of the block.
5. `INTERP` (kind 0) runs `exec_cached` verbatim, so an all-`Interp` build is exact by construction. It materializes before the call (timers read `steps`), signals `Jump` when `core.pc != u.b` (a branch or a loop back-edge moved the pc), checks `RF_SMC` after an interpreted store, re-checks `due`, and counts `steps_interp_uop`.
6. Dispatch (header of `exec/mod.rs`, P2): `UF_CB` calls `latch_cb_overflow` after the uop when `cb_wrap != 0`; `UF_FIN` calls `finish_step(u.pc, kind, next)` with kind and next from the decode-cache entry at `u.pc`; `Smc` is raised after the store completed and continues at the next uop's pc or `Block::end_pc`; `Fault`, `Guard` and `Slow` did not run the uop and materialize `u.pc`.

### 4.6 `Footprint`

Per block: `reads: u64`, `writes: u64` bitmasks over the PE register file and system state classes, `touches_mem: bool`, `touches_flags: bool`. Used by the exit sweep (what to compare after a partial run), by register promotion (Tier 2) and as the only hook for a later read-only parallel probe.

### 4.7 Reuse map (what is taken from v1, what is new)

| v1 item | Use in uop |
|---|---|
| `core::decode_cache::{CachedInsn, DecodedOp, ComputeOp, FlowOp, MoveOp, DualMoveOp}` | lowering input, read through the same accessor `Core::step` uses (`isa_at`, `decode_cache.get`, `fetch_and_cache`) |
| `Core::exec_cached(&CachedInsn)` | body of the `Interp` uop: one instruction, exact, including `finish_step`, `latch_cb_overflow`, window close and `steps += 1` |
| `aot_pe::{Pe, pe_supports, Core::aot_pe_end}` | in-place PEx/PEy element access and the reference semantics for the generic float and fixed handlers (`FGen`, `XGen`); the diet kernels are verified against it |
| `aot_whole::{terminator, cuts_block, guard_class, loop_ends, whole_starts}` | read as the spec of v1 cut and guard decisions; reimplemented in `lower/{cut,guard,scan}.rs` with the extra rules of section 7 |
| `aot_hot::{aot_try, aot_enter, run_aot}`, `runner::pass_sched::lean_pass_aot` | the model for `uop_try`, `run_uop`, `lean_pass_uop` (same contracts: `uop_try(limit) -> bool`, fault via `uop_take_fault`) |
| `core::jit.rs` | Cranelift precedent (`JITModule`, nested `offset_of!`, `opt_level=speed`, `is_pic=false`); copied in style, not reused (own module, own cache) |
| `runner::jit_diff.rs` | precedent for the lockstep runner |
| `Core::state_mismatch` | not reused: it compares only the effective f40 |

## 5. Constants and provenance

| Constant | Value | Tag | Source or what would confirm |
|---|---|---|---|
| `Uop` size | 16 bytes | VERIFIED (once W0 lands) | const assert plus test `uop_is_16_bytes` in `types.rs` |
| Entry guard step rule | `steps + n <= irq.due` | ASSUMED until pinned | derived from the `Core::step` prologue `if self.steps >= self.irq.due && self.sequencer_event()` (read in `exec.rs`); becomes VERIFIED when `event_split_matches_interpreter` (W2-DRIVE) passes |
| `poke_irq` sets `due = 0` | yes | ASSUMED until pinned | read in `interrupts.rs`; becomes VERIFIED when `poke_forces_checked_loop` (W2-DRIVE) passes |
| Page shift for `CodePages` | 8 (256 short words) | ASSUMED | tradeoff of invalidation granularity against bitmap size; confirm with overlay traces |
| `max_block_steps` | 48 | ASSUMED | bounds the checked-loop replay near events; recalibrate with `uop_report` block histogram |
| `min_block_steps` | 1 | ASSUMED | blocks of one step are legal; the report counts them |
| Interp uop cost | 100 cycles | ASSUMED | `exec_cached` through a decode lookup; confirm at K0 |
| Dispatch cost per uop | 6 cycles | ASSUMED | indirect call, return, `done += steps`, loop; confirm at K0 |
| Block transition cost | 38 cycles | ASSUMED | exit, link or PcMap probe, entry guard; confirm at K0 |
| uops per step | 1.35 | ASSUMED (also a gate) | fusion target of `lower/*` |
| Core `periph_range` | 0x2000_0000..0x3200_0000, disjoint from the SRAM windows | ASSUMED until pinned | read in `periph/glue.rs`; becomes VERIFIED when `periph_claims_disjoint_from_sram_windows` (`exec/mem.rs`, W2-MOVE) passes |
| SRAM word window; short-word window | 0x9_0000..0x10_0000; 0x10_0000..0x20_0000 | ASSUMED until pinned | read in `memory.rs`; the skeleton imports the constants (no copies) and `code_pages_cover_short_window` pins the derived sizes |

## 6. Exactness and verification plan

Principle: the interpreter `Core::step` is the only oracle. Every layer below compares against it, never against v1 and never against a second uop path. Hot-path code gets no per-op check; checks live in tests, in `Engine::UopChecked` and in the optional shadow mode.

### 6.1 Exact comparator (`verify/compare.rs`, `verify/snap.rs`)

`exact_diff(a: &Core, b: &Core) -> Option<ExactDiff>` compares: `pc`, `steps`, the r, s, i, m, l, b register files, the f40 registers including hidden tag and lo (the 40-bit extension model: `lo` counts only while `tag == r`; direct r writes keep a stale lo), `mr`, flags and `astat` for both PEs (PEy owns flags, `astat_extra`, `stky`, `mr` in SIMD), `mode1`/`mode2`, the bit FIFO, `cb_wrap`, call stack, loop stack, `pending`, `irq` (`due`, `window_len`, `idle`, `stall`), and all SRAM words (`SRAM_WORDS`) plus the short-word alias window. `ExactDiff` names the first differing field and its two values. `CoreSnap` captures and restores every compared field except SRAM (SRAM uses an undo log, 4.5 rule 4 and 6.6).

Test obligation: a field-coverage test lists every hidden field and perturbs each one on a clone; the comparator must flag all of them (mutation of the comparator itself).

### 6.2 Block-level fuzz (`verify/gen.rs`, `verify/fuzz.rs`)

Deterministic xorshift PRNG (seeded, no `rand` dependency). For each supported instruction family, generate random blocks and random states biased to edge cases: zero, denormal, NaN, infinity, max exponent, tag/lo corner cases, saturation boundaries, empty and full stacks, circular buffers at wrap, SIMD on and off, conditions true and false. Run Tier 1 against `Core::step` for `block.steps`, call `exact_diff`. Iteration counts come from `NORD_UOP_FUZZ_N` with a small default so the debug-profile CI job stays inside its 20-minute limit (default 3,000 blocks per family, ASSUMED; release gate G1 uses 200,000).

### 6.3 Census replay (`verify/census.rs`)

`census::Engine::Uop` replays all 1,580 census cases (EMPIRICAL count from the brief) through `translate_range` plus `run_uop` and requires the same output lines as `Engine::Interp`. Runs in CI; no firmware needed.

### 6.4 Real traces, lockstep (`verify/lockstep.rs`)

Modeled on `runner/jit_diff.rs`: run `Core::run` and `run_uop` from the same snapshot on real firmware passes of ORG, PNO, FX, SY1SY2 and SMP cores (steady state and a playing state), compare every N steps (N = block boundary, then every step in failing windows) with `exact_diff`. Needs local firmware, so tests check an environment variable and return early when absent (never `#[ignore]`, never failing CI). Because CI cannot run them, section 14 risk 7 applies: synthetic fuzz and census carry CI.

### 6.5 Exit sweep (`verify/sweep.rs`)

For each translated block and each uop index `i`, run the block with a budget that ends exactly at `i` (using the checked loop) and compare against the interpreter run for `cum[i]` steps. This verifies exit materialization at every possible exit, which is what makes Tier 2 register promotion safe.

### 6.6 Shadow mode (`verify/shadow.rs`, dev only, `shadow_every > 0`)

At block entry capture `CoreSnap` (a few KB, no SRAM) and enable an undo log for SRAM stores (the store handlers already branch on `uop_run.flags`, so the log costs nothing when off). After the block, capture the result, restore, run the interpreter for `done` steps, compare. The interpreter result wins (self-healing), mismatches are counted. A block that touched a peripheral (`periph_touch`) cannot be replayed and is skipped and counted (`shadow_skipped`). Off by default, never in the audio path.

### 6.7 Translation validation (`verify/tv.rs`, `native/tv.rs`)

On the translation worker: every new native block runs on K random states (K = 64, ASSUMED) against the Tier 1 result and `exact_diff` must be empty before install. A failing block is marked `poisoned` and runs Tier 1. Tier 1 itself is validated at `enable_uop` by running each new block on 8 random states against `Core::step` (ASSUMED), off the audio path.

### 6.8 Mutation testing of the verifiers (`verify/mutate.rs`)

Output-perturbation wrappers: `perturb(handler, Mutant)` returns a handler that calls the real one and then flips one result bit, hidden lo, a flag, `stky`, `astat_extra`, a FIFO bit, `cb_wrap`, a stack slot, a SRAM word, or the step count. Test obligation: for every mutant class, fuzz plus sweep plus comparator detect the mutant within the default iteration count; a surviving mutant fails the suite. This tests the tests.

### 6.9 `Engine::UopChecked`

`lean_pass_uop_checked` runs the pass under Tier 1, then rewinds with a pass-start snapshot and runs the interpreter pass, comparing with `exact_diff`. Allocates; dev and tooling only; never selected by default. Restricted to cores without peripherals that cannot be snapshotted (open risk 6).

### 6.10 Verification matrix

| Bug class | Caught by |
|---|---|
| Wrong result bits, wrong hidden tag/lo, wrong flags | fuzz, census, lockstep, comparator |
| Wrong state at an early exit | exit sweep, shadow |
| Step-count or pc drift (budget, delayed branch, loop end) | fuzz with control flow, lockstep, `event_split_matches_interpreter` |
| Event timing (`due`, window, `poke_irq`) | event tests, lockstep with timer cores |
| Stale code after a store or overlay | `smc_*` tests, shadow, lockstep on program-change traces |
| A weak verifier | mutation tests |
| Bad native code | translation validation, `UopChecked`, lockstep with `uop-native` |

## 7. Events, interrupts, overlays, invalidation

### 7.1 Block entry guard (`lower/guard.rs`, checked in `exec/mod.rs`)

All of these must hold, else `Exit::Guard` and one `Core::step`:

| Condition | Why |
|---|---|
| `steps + block.steps <= min(limit, irq.due)` (or the checked loop is used, 7.2) | event exactness |
| `irq.window_len == 0`, `!irq.idle`, `irq.stall == 0` | window and idle semantics live in the prologue and epilogue of `Core::step` |
| `pending.is_none()` unless the block begins with a delay window it models | delayed-branch state |
| no active `LoopFrame` has its end in `[block.pc, block.last_pc)` | back-edge happens in `finish_step`; loop ends are block-final |
| `core.simd() == block.simd` | PEy participation is baked into the block. The driver looks the pc up in the map of `core.simd()` (P4), so this row is a safety net that should not fire; ORG Core2 runs 5,584 of 5,953 steps with PEy on, which is why both variants are translated |
| `trace.is_none() && site_hits.is_empty()` | those prologue hooks must run per step |
| `cb_wrap.get() == 0` unless the block has `UF_CB` uops | the latch is processed after each step |
| `!arith_irq_enabled()` unless the block has no uop that can raise an arithmetic exception | `note_arith_exception_in` can raise an interrupt mid-block |
| `block.poisoned == false`, `isa_at(pc) == block.isa` | validity |

The `periph.is_none()` restriction of v1 `aot_fast_mem` is dropped: SRAM accesses use a direct fast path, every other address goes through `dm_read`/`dm_write` (peripheral first, same order as the interpreter). This is sound only because `periph_claims` does not overlap the SRAM windows; `periph_claims_disjoint_from_sram_windows` pins that.

### 7.2 Event split

`room = min(limit, irq.due) - steps`. If `room >= block.steps` the block runs in the hot loop with no per-uop check. If not, `run_block_checked` runs uops while `room >= u.steps` and exits `Slow` at the first uop that does not fit; the driver then calls `Core::step` for the remaining 1 to 3 steps (the step that crosses `due` services the event in its prologue and returns), then re-enters through `PcMap` at the next uop boundary. Steps inside fused uops that are not boundaries are finished by `Core::step`.

### 7.3 Events raised inside a block

| Source | Handling |
|---|---|
| System-register write (`IMASK`, `MODE1`, timers, `poke_irq`) | the writing uop is a block terminator (`Sys`), followed by re-dispatch; `poke_irq` sets `due = 0`, so the next entry is in the checked loop and the prologue runs |
| Peripheral access through `dm_read`/`dm_write` | rule 4 of 4.5: `due` re-check after the access |
| Arithmetic exception sticky | guarded at entry (7.1) |
| Circular-buffer wrap latch (I7, I15) | `UF_CB` uops call `latch_cb_overflow` after, exactly like `exec_cached` |
| IDLE | `Idle` is a terminator and runs `enter_idle` through the interpreter body |
| Interrupt window close | entry guard keeps blocks out of windows; `Interp` uops run `close_window_if_left` via `exec_cached` |

### 7.4 Self-modifying code, overlays, DMA

`Core::invalidate_sram_cache(first_short, count)` and `invalidate_code_cache()` get one line each in `decode_cache.rs`: `self.uop_note_write(first_short, count)` and `self.uop_note_all()` (both `#[inline]`, early-return when `uop_run.code_pages` is empty). The hook sets `uop_run.dirty` bits and `smc_dirty` using only `Core` state.

1. A store handler checks `uop_run.flags` once after the write (the same check covers shadow log and trace). If `smc_dirty` is set it returns `Smc` after the store completed and accounted. The rest of the block, which may be stale, is not run. This closes the "SMC hole inside a running block" recorded in `sharc-lle-realtime.md`.
2. At the next `uop_try`, `revalidate_dirty` walks only the blocks in dirty pages (`page_blocks`), recomputes `lower::decode::code_hash(core, block.pc, block.end_pc)`, the function the translator stored the hash with (precedent: v1 `aot_code_matches`), keeps matching blocks (an overlay that reloads identical code costs a compare, not a retranslation) and drops the others: tombstone in `blocks`, the `PcMap` entries of the block's own mode cleared, the range pushed to `dropped` (capacity reserved at `install`, no allocation on the audio path), `epoch += 1` so cached `Link`s revalidate on use.
3. Dropped ranges run in the interpreter (`PcMap` miss means `Core::step`). They are retranslated by `uop_maintain()`, called by `lean_pass_uop` at the end of a pass only when `uop_run.dirty != 0` (program change, overlay load). `maintain_calls` must stay 0 in steady passes (gate G2). `uop_maintain` may allocate; steady passes never reach it.
4. Tier 2: the native table slot becomes a tombstone that routes to Tier 1 or the interpreter; code memory is freed at pass boundaries only, never while a block runs (single thread, the hook only sets flags). New native code arrives from the worker through the lock-free ring and is installed at a pass boundary into a pre-sized table (pointer write, no allocation; a full table drops the install and returns the block to the worker for free).
5. SRAM short-word alias window writes reach SRAM through `dm_write`, which already calls the hook; handlers never bypass it. Test `alias_write_invalidates` pins that.

## 8. Always-correct fallback

| Layer | Behaviour |
|---|---|
| `Interp` uop | any instruction the lowering does not model runs `exec_cached` verbatim |
| `Exit::Fault/Guard/Slow` | materialize at a uop boundary, continue with `Core::step` |
| `PcMap` miss | `Core::step` |
| `uop_try` returns false | caller runs `Core::step`; engine off means v1 behaviour byte for byte |
| Poisoned block | repeated mismatch or failed translation validation sets `poisoned`; block runs Tier 1 or interpreter |
| Config | `Engine::Interp` stays the default; `enable_uop` is never called by default; `uop-native` is off by default |
| Never | no handler panics; no guess where state is unknown (guard instead) |

A Tier 1 build in which every lowered instruction is an `Interp` uop is bit-exact by construction. This is milestone M1 (section 11): driver, events, exits and invalidation are validated before any fast handler exists.

## 9. Cost model

All figures are ASSUMED model outputs built from the EMPIRICAL inputs in section 2 and the per-class costs in section 5. Wall-clock is not measured here. ORG Core2, 6,280 steps per pass (ASSUMED), host cycles at 4.5 GHz (ASSUMED).

### 9.1 Tier 1

| Class | Count per pass | Cycles each | Cycles | Note |
|---|---|---|---|---|
| Compute uops (both elements, in-place handlers) | 2,300 | 89 | 205k | shared in-place handlers about 57-59 cycles per add or mul on both elements (matches v1), plus flags, sticky, avg/select/compare which are heavier |
| Move and memory uops | 2,900 | 18 | 52k | SRAM fast path, dual moves, DAG post-modify |
| Dispatch | 8,478 uops (1.35 per step) | 6 | 51k | indirect call, `done += steps` |
| Block transitions | 520 blocks (about 12 steps each) | 38 | 20k | exit, link or PcMap, entry guard |
| `Interp` uops | 125 steps (2%) | 100 | 12k | |
| **Total** | | | **about 340k** | **x1.64 of 207k, 76 us** |

Range: best x1.15 to x1.25 (53-58 us), worst x2.6 (120 us). Without the avg diet (W1-KERN) the model adds about 37k cycles: about x1.84, reported as x1.9. Against v1 (161 us) the likely gain is x2.1. Tier 1 does not reach real time on ORG Core2 and is not claimed to.

### 9.2 Tier 2 (feature `uop-native`)

| Class | Cycles | Note |
|---|---|---|
| Compute, inline add/sub/mul/avg/select/compare/convert | 140k | spike floor is 675k insns / IPC 3.37 = 200k cycles in total, this row is the compute portion after the diet |
| Moves and memory, inline SRAM path | 38k | |
| Flow, block transitions, links | 16k | |
| Residual call-threaded handlers and `Interp` | 24k | families not yet inline |
| Entry guard, exit materialization, `due` checks | 18k | |
| **Total** | **about 236k** | **x1.14 raw, 52 us** |

Range: x0.85 best (39 us), x1.3 likely (60 us), x1.9 worst (88 us). The likely figure adds cache and branch effects that the raw model omits. The real-time claim needs median <= 0.85 x 46.08 = 39.2 us and p99 <= 1.3 x 46.08 = 59.9 us (gate K4), which only the best case reaches. Hence the stated expectation: exact real time on ORG Core2 is unlikely, lighter cores (PNO, FX) are the realistic targets, and the hybrid holds for ORG unless K3 passes.

### 9.3 Translation cost

| Item | Value | Tag |
|---|---|---|
| Tier 1 translate | about 1 us per instruction, once per `enable_uop` and per overlay | ASSUMED |
| Tier 2 translate | <= 5 s single-thread for ORG Core2 (gate K5) | ASSUMED threshold |
| Audio path allocation | none in `uop_try`, `run_block`, handlers, `lean_pass_uop`; `uop_maintain` allocates and runs only on overlay events | design rule |

## 10. Skeleton contract (W0)

The skeleton is one commit-sized change by one agent, made on a fresh `origin/main`. It creates every new file listed in section 11 (stubs with a real unit test each, so `check_tests.py` stays at 0 uncovered), makes the edits to existing files listed below exactly once, and leaves all new behaviour off: `enable_uop` is never called by default, `uop_try` on a core without `enable_uop` returns false, `Engine::Interp` stays the default, `uop-native` is not in `default`. Later packages do not edit `mod.rs`, `types.rs`, `config.rs`, `run.rs`, `hot.rs` or any existing file. Ranges for `UopKind` are partitioned so each family adds constants in its own file.

### 10.1 Cargo and features

`crates/nord-sharc/Cargo.toml`, under `[features]`:

```toml
uop-native = ["jit"]
```

No new dependencies. `jit` already pulls `cranelift-codegen`, `cranelift-frontend`, `cranelift-jit`, `cranelift-module`, `cranelift-native` 0.116. CI already uses `--all-features` on ubuntu, windows and macos (arm64), so `uop-native` is compiled and its tests run on all three.

`crates/nord-tools/Cargo.toml`:

```toml
[[bin]]
name = "uop-report"
path = "src/bin/uop_report.rs"
```

(Evaluation harnesses are bins, never examples: D57.)

### 10.2 Module tree (all paths under `crates/nord-sharc/src/`)

```
core/mod.rs                      EDIT: `mod uop;` + `pub use uop::{UopConfig, UopStats, UopBuildReport, UopError, Exit, ExactDiff};` + 2 fields on Core
core/new.rs                      EDIT: init `uop: None, uop_run: uop::UopRun::new()`
core/decode_cache.rs             EDIT: hook calls in invalidate_sram_cache and invalidate_code_cache
core/census.rs                   EDIT: Engine::Uop, Engine::UopChecked + arms in case_run/run_cases_on
core/aot_pe.rs, aot_state.rs,
  branch.rs, interrupts.rs, ...  EDIT only if needed: widen a private item to `pub(super)` (no logic change)
runner/mod.rs                    EDIT: `pub mod uop_pass; pub mod uop_report;`
runner/pass_sched.rs             EDIT: Engine::{Uop, UopChecked, UopNative} + match arms in run_pass and every exhaustive match

core/uop/mod.rs                  declarations, cfg, re-exports, one wiring test
core/uop/types.rs                Uop, UopKind, Block, Link, Exit, Handler, UopCtx, GuardClass, Footprint, consts
core/uop/config.rs               UopConfig, UopStats, UopBuildReport, UopError
core/uop/run.rs                  UopRun, CodePages
core/uop/hot.rs                  UopHot, PcMap
core/uop/drive.rs                impl Core { enable_uop, disable_uop, ... uop_try, run_uop }
core/uop/invalidate.rs           impl Core { uop_note_write, uop_note_all, uop_revalidate_dirty, uop_maintain }
core/uop/lower/{mod,scan,cut,budget,guard,pcmap,decode,builder,float,fixed,moves,flow}.rs
core/uop/exec/{mod,exit,event,pe,mem,h_core,h_float,h_fixed,h_move,h_flow}.rs
core/uop/kernels/{mod,add,mul,avg,select,compare,convert}.rs
core/uop/verify/{mod,snap,compare,gen,mutate,fuzz,lockstep,census,shadow,tv,sweep,count_alloc}.rs
core/uop/service/{mod,ring,worker}.rs
core/uop/native/{mod,ctx,emit,call,enter,cache,inline_float,inline_move,inline_flow,regpromo,service,tv}.rs   #[cfg(feature = "uop-native")]
runner/uop_pass.rs               lean_pass_uop, lean_pass_uop_checked
runner/uop_report.rs             deterministic counter report (JSON) + cost model evaluator
crates/nord-tools/src/bin/{uop_report,lle_sharc_bench}.rs   (the second is an existing file: W1-RUN adds the uop engines)
tools/uop_gates.py, tools/tests/test_uop_gates.py, tools/uop_insn_count.py (+ test)
docs/specs/sharc-uop.md, docs/DECISIONS.md (D260, status proposed, Decided by left empty), docs/ROADMAP.md rows
```

Every `mod.rs` carries at least one `#[test]` and no `#[allow(...)]` attribute line (an `allow` attribute voids the declaration-only exemption in `check_tests.py`). `uop/mod.rs`:

```rust
pub mod config;
pub mod drive;
pub mod exec;
pub mod invalidate;
pub mod kernels;
pub mod lower;
pub mod run;
pub mod hot;
pub mod service;
pub mod types;
pub mod verify;
#[cfg(feature = "uop-native")]
pub mod native;

pub use config::{UopBuildReport, UopConfig, UopError, UopStats};
pub use types::{Block, Exit, Footprint, GuardClass, Handler, Link, Uop, UopCtx, UopKind, NONE};
pub use verify::compare::ExactDiff;
pub(crate) use hot::UopHot;
pub(crate) use run::UopRun;
```

### 10.3 Types (exact)

```rust
// core/uop/types.rs
pub const NONE: u32 = u32::MAX;
pub const PAGE_SHIFT: u32 = 8;                      // ASSUMED, section 5

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Uop { pub h: u16, pub steps: u8, pub flags: u8, pub a: u32, pub b: u32, pub pc: u32 }
const _: () = assert!(core::mem::size_of::<Uop>() == 16);

pub const UF_FIN: u8 = 1; pub const UF_CB: u8 = 2; pub const UF_LAST: u8 = 4; pub const UF_SIMD: u8 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UopKind(pub u16);
impl UopKind {                      // ranges: core 0x000.., float 0x100.., fixed 0x200.., moves 0x300.., flow 0x400..
    pub const NOP: UopKind = UopKind(0x000);
    pub const INTERP: UopKind = UopKind(0x001);   // a = index into block constant pool of a CachedInsn
    pub const FIN: UopKind = UopKind(0x002);      // terminator that runs finish_step
    pub const BAIL: UopKind = UopKind(0x003);     // always Exit::Slow (test and unimplemented-kind default)
    pub const MAX: u16 = 0x500;                   // size of the handler table
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit { Ok = 0, Jump = 1, Fault = 2, Guard = 3, Smc = 4, Slow = 5 }

#[repr(C)]
pub struct UopCtx { pub core: *mut crate::core::Core, pub base_steps: u64, pub done: u32,
                    pub room: u32, pub next_pc: u32, pub exit_idx: u32 }

pub type Handler = unsafe extern "C" fn(ctx: *mut UopCtx, u: *const Uop) -> u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardClass { Plain, Loop, Delayed, Cb, Arith }          // entry-guard strength, section 7.1

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Footprint { pub reads: u64, pub writes: u64, pub touches_mem: bool, pub touches_flags: bool }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link { pub id: u32, pub epoch: u32 }

#[derive(Clone, Debug)]
pub struct Block {
    pub pc: u32, pub last_pc: u32, pub end_pc: u32, pub steps: u32, pub first: u32, pub len: u32,
    pub cum: Box<[u16]>, pub simd: bool, pub isa: bool, pub guard: GuardClass,
    pub links: [Link; 2], pub code_hash: u64, pub native: u32, pub poisoned: bool,
    pub footprint: Footprint,
}
```

```rust
// core/uop/config.rs
#[derive(Clone, Debug)]
pub struct UopConfig {
    pub min_block_steps: u32,   // default 1
    pub max_block_steps: u32,   // default 48
    pub shadow_every: u32,      // default 0 = off (dev only)
    pub native: bool,           // default false; needs feature uop-native
    pub families: u32,          // NativeFamilies bitset, default 0
    pub async_translate: bool,  // default false
    pub poison_after: u32,      // default 3
}
impl Default for UopConfig { fn default() -> Self { /* values above */ } }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UopStats {
    pub passes: u64, pub blocks_run: u64, pub uops: u64, pub steps_uop: u64,
    pub steps_interp_uop: u64, pub steps_core_step: u64, pub steps_native: u64,
    pub exits: [u64; 6],            // indexed by Exit as usize
    pub guard_fail: [u64; 9],       // one per row of section 7.1
    pub smc: u64, pub translations: u64, pub poisoned: u64,
    pub maintain_calls: u64, pub shadow_checks: u64, pub shadow_mismatch: u64, pub shadow_skipped: u64,
}
impl UopStats { pub fn since(&self, earlier: &UopStats) -> UopStats; }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UopBuildReport { pub blocks: u32, pub uops: u32, pub insns: u32, pub interp_insns: u32, pub pages: u32 }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UopError { OutOfRange { lo: u32, hi: u32 }, NativeUnavailable, Translate(String) }
```

```rust
// core/uop/run.rs
pub const CODE_PAGE_WORDS: usize = 64;                     // 64 x u64 = 4096 pages of 256 short words
#[derive(Clone, Debug)] pub struct CodePages { pub bits: [u64; CODE_PAGE_WORDS] }
impl CodePages { pub const fn new() -> Self; pub fn set_range(&mut self, lo: u32, hi: u32);
                 pub fn hit(&self, first_short: u32, count: u32) -> bool; }
pub const RF_SMC: u32 = 1; pub const RF_SHADOW: u32 = 2; pub const RF_TRACE: u32 = 4;
pub struct UopRun { pub cfg: UopConfig, pub stats: UopStats, pub code_pages: CodePages, pub flags: u32,
                    pub fault: Option<crate::core::Fault>, pub stop_pc: u32, pub dirty: u64, pub active: bool }
impl UopRun { pub fn new() -> Self; }
```

```rust
// core/uop/hot.rs
pub(crate) struct PcMap { /* two-level: Vec<Option<Box<[(u32, u32); 256]>>> */ }
impl PcMap { pub fn new() -> Self; pub fn get(&self, pc: u32) -> Option<(u32, u32)>; pub fn set(&mut self, pc: u32, v: (u32, u32)); pub fn clear_range(&mut self, lo: u32, hi: u32); }
pub(crate) struct UopHot {
    pub arena: Vec<Uop>, pub blocks: Vec<Block>, pub pcmaps: [PcMap; 2], pub handlers: Box<[Handler]>,
    pub page_blocks: Vec<Vec<u32>>, pub epoch: u32, pub dropped: Vec<(u32, u32)>,
    #[cfg(feature = "uop-native")] pub native: Option<Box<crate::core::uop::native::NativeTable>>,
}
```

### 10.4 Core API (exact signatures)

```rust
// core/uop/drive.rs
impl Core {
    /// Translates [lo, hi) (short words) and arms the engine. Never called by default.
    pub fn enable_uop(&mut self, cfg: UopConfig, ranges: &[(u32, u32)]) -> Result<UopBuildReport, UopError>;
    pub fn disable_uop(&mut self);
    pub fn uop_enabled(&self) -> bool;
    pub fn uop_stats(&self) -> UopStats;
    pub fn uop_reset_stats(&mut self);
    /// pc at which block chaining must return to the caller (the pass idle pc). `None` clears it.
    pub fn uop_set_stop_pc(&mut self, pc: Option<u32>);
    /// Same contract as `aot_try`: runs one or more chained blocks when `limit` leaves room and
    /// the guard holds; returns whether any block ran. A fault is left for `uop_take_fault`.
    pub fn uop_try(&mut self, limit: u64) -> bool;
    pub fn uop_take_fault(&mut self) -> Option<Fault>;
    /// `Core::run` with the block hook, same contract as `run_aot`.
    pub fn run_uop(&mut self, max_steps: u64) -> Option<Fault>;
}
// core/uop/invalidate.rs
impl Core {
    #[inline] pub(in crate::core) fn uop_note_write(&mut self, first_short: u32, count: u32);
    #[inline] pub(in crate::core) fn uop_note_all(&mut self);
    pub fn uop_maintain(&mut self) -> usize;                 // retranslate dropped ranges; returns blocks added
}
// core/uop/exec/mod.rs
pub(crate) unsafe fn run_block(core: &mut Core, hot: &UopHot, id: u32, from_idx: u32, room: u32) -> (Exit, u32 /*uop idx*/, u32 /*done*/);
pub(crate) unsafe fn run_block_checked(core: &mut Core, hot: &UopHot, id: u32, from_idx: u32, room: u32) -> (Exit, u32, u32);
// core/uop/exec/h_*.rs: one per family, called from UopHot construction
pub(crate) fn register(t: &mut [Handler]);                   // h_core, h_float, h_fixed, h_move, h_flow (each its own range)
// core/uop/lower/mod.rs
pub(crate) fn translate_range(core: &mut Core, cfg: &UopConfig, lo: u32, hi: u32) -> Result<(Vec<Block>, Vec<Uop>), UopError>;
pub(crate) struct LowerCtx<'a> { pub cfg: &'a UopConfig, pub stop_pc: Option<u32>, pub simd: bool, pub isa: bool }
pub(crate) trait Family {                                     // implemented by lower/{float,fixed,moves,flow}.rs
    fn lower(cx: &LowerCtx<'_>, c: &CachedInsn, out: &mut builder::BlockBuilder) -> bool;  // false => caller emits Interp
}
// core/uop/lower/builder.rs
pub(crate) struct BlockBuilder { /* start pc, uops, steps, footprint, flags */ }
impl BlockBuilder { pub fn push(&mut self, u: Uop); pub fn interp(&mut self, c: &CachedInsn); pub fn finish(self, guard: GuardClass) -> (Block, Vec<Uop>); }
// core/uop/kernels/mod.rs: pure functions on u32 f32 bit patterns and hidden state, exact vs `float::*`
pub fn fadd(a: F40, b: F40, st: &mut FlagsOut) -> F40;       // and fsub, fmul, favg, fcmp, fselect, fconv: same shape, each in its own file
// core/uop/verify
pub fn exact_diff(a: &Core, b: &Core) -> Option<ExactDiff>;   // compare.rs
pub struct CoreSnap; impl CoreSnap { pub fn capture(c: &Core) -> Self; pub fn restore(&self, c: &mut Core); }   // snap.rs
pub fn perturb(h: Handler, m: Mutant) -> Handler;            // mutate.rs (built with a macro table of wrappers)
// core/uop/service
pub struct Ring<T, const N: usize>;                          // SPSC lock-free, fixed capacity, no allocation after new
pub struct TransReq { pub lo: u32, pub hi: u32, pub words: Box<[u16]> }
pub struct TransDone { pub blocks: Vec<Block>, pub uops: Vec<Uop> }
// runner/uop_pass.rs
pub fn lean_pass_uop(core: &mut Core, cfg: &PassConfig) -> (u64, bool);
pub fn lean_pass_uop_checked(core: &mut Core, cfg: &PassConfig) -> (u64, bool, Option<ExactDiff>);
// runner/pass_sched.rs
pub enum Engine { Interp, Aot, Uop, UopChecked, UopNative }  // run_pass: Uop => lean_pass_uop, UopChecked => checked (diff panics), UopNative => lean_pass_uop with cfg.native
// core/census.rs
pub enum Engine { Interp, #[cfg(feature = "jit")] Jit, Uop, UopChecked }
// runner/uop_report.rs
pub struct UopReport { pub stats: UopStats, pub cost: CostModel }   // serde_json via nord-tools, field names stable: gates read them
pub fn model_cycles(s: &UopStats, tier: Tier) -> u64;       // deterministic counters x section 5 costs (ASSUMED)
```

Native tier skeleton (feature `uop-native`):

```rust
// core/uop/native/mod.rs
pub struct NativeTable;                                       // fixed-capacity slots, install at pass boundary
pub const FAM_CALL: u32 = 1; pub const FAM_FLOAT: u32 = 2; pub const FAM_MOVE: u32 = 4; pub const FAM_FLOW: u32 = 8; pub const FAM_REGPROMO: u32 = 16;
pub(crate) trait Emitter { fn try_emit(&mut self, u: &Uop, fb: &mut cranelift_frontend::FunctionBuilder<'_>) -> bool; } // false => call-threaded
pub(crate) fn enter(core: &mut Core, hot: &UopHot, id: u32, room: u32) -> (Exit, u32, u32);   // called from drive.rs when Block.native != NONE
```

`drive.rs` already contains the one `#[cfg(feature = "uop-native")] if b.native != NONE { return native::enter(...) }` call; W4 packages fill it in.

### 10.5 Skeleton acceptance

- `cargo fmt`, `clippy -D warnings`, `cargo test --workspace --all-targets --all-features`, `ruff`, `pytest`, `check_tests.py` (0 uncovered), `check_fw_derived.py` (baseline unchanged) all pass with every stub.
- With `enable_uop` never called, every existing test, census output and golden hash is unchanged (the hook functions early-return).
- Stub behaviour: `enable_uop` builds an empty `UopHot` and returns `Ok(UopBuildReport::default())`; `uop_try` returns false; `lean_pass_uop` therefore behaves like the interpreter pass and its test asserts equality with `lean_pass` on a synthetic core.
- New tests in the skeleton: `uop_is_16_bytes`, `code_pages_cover_short_window`, `hook_is_noop_when_disabled`, `engine_default_is_interp`, `stub_pass_equals_interpreter_pass`.

### 10.6 Interfaces added by the readiness review (all in the skeleton)

| Item | Where | Purpose |
|---|---|---|
| `UopHot::pcmaps: [PcMap; 2]`, `UopHot::pcmap(simd)` | `hot.rs` | one pc map per PEy mode (P4) |
| `Block::end_pc` | `types.rs`, `lower/builder.rs` | fall-through successor (P5) |
| `BlockBuilder::{end_block, ends, owe_slots, owed_slots, slot_done, set_end}` | `lower/builder.rs` | block-end and delay-slot protocol (P1) |
| `lower::decode::code_hash(core, lo, hi)` | `lower/decode.rs` | one hash for translator and revalidation (P6) |
| `verify::count_alloc::allocs_in(f)` | `verify/count_alloc.rs` (`#[cfg(test)]`) | the single counting `#[global_allocator]` (P9) |
| `verify::gen::random_program_from(rng, pool, n)` | `verify/gen.rs` | per-family program tables, `PROGRAM_POOL` stays closed (P13) |
| `verify::fuzz::{DiffRun, diff_program_run}`, `DiffRun::exercised()` | `verify/fuzz.rs` | non-vacuous differential runs (P10) |
| G2 `smc == 0` | `tools/uop_gates.py` | a data store sharing a code page is visible |

## 11. Work packages

Status after the readiness review of 2026-11-28 (11.3): W0 is done in the working tree and still to be committed, path-limited (11.4). Waves are MERGE waves. A package starts when the interfaces it consumes exist in the skeleton, not when another package has finished its body: a package is tested at handler or unit level in its own worktree and meets the others only at the integration points of 11.2. After W0, a stub file belongs to exactly one package. The five W0 files (`uop/mod.rs`, `types.rs`, `config.rs`, `run.rs`, `hot.rs`) and every pre-existing file change only by an amendment decided by the orchestrator (each one ripples through every family), except the one-line visibility widening of rule P12. Directory `mod.rs` files belong to the package named in their header (`lower/mod.rs` W1-LOWER, `exec/mod.rs` W2-DRIVE, `kernels/mod.rs` W1-KERN); `verify/`, `service/` and `native/` `mod.rs` are declaration-only and W0-only. Every package ships unit tests in the same change (tests per source file, at most about 400 lines per file, provenance tags on constants, no firmware bytes, no `#[ignore]`). Per package the author runs its own unit tests only; no benchmark, no whole-suite run per commit.

| Id | Wave | Files owned (disjoint) | Consumes (frozen by W0) | Acceptance in its own worktree |
|---|---|---|---|---|
| W1-VERIFY | 1 | `verify/{snap,compare,gen,mutate}.rs` | `Core` fields | field-coverage perturbation of every compared field; every mutant class changes `exact_diff`; snap/restore identity; generator determinism. Adds `gen::census_encodings(filter)` (valid encodings per instruction kind from the committed census `docs/research/sharc-seq/census_cases.txt`, so the families do not hand-write opcodes) and fillers for sticky, `astat_extra`, FIFO and stack state. May split `compare.rs` into `compare/` (P11) |
| W1-KERN | 1 | `kernels/{mod,add,mul,avg,select,compare,convert}.rs`, `exec/pe.rs`, `tools/uop_insn_count.py`, `tools/tests/test_uop_insn_count.py` | merged `aot_fast`, `aot_fadd`, `aot_round`, `aot_simd`, `aot_simd_lanes`, `aot_simd_core` (#685 on `main`) | each kernel equals `float::*` / `Pe::exec` bit for bit (value, hidden tag and lo, flags) on class grids plus seeded random, including the paired `*2` forms. Instruction budgets are measured by `tools/uop_insn_count.py` (disassembles `#[inline(never)]` probe symbols with `objdump -d`; owner-run for K1 and K2, not a unit test, because Rust has no in-process instruction counter) |
| W1-LOWER | 1 | `lower/{mod,scan,cut,budget,guard,pcmap,decode,builder}.rs` | `BlockBuilder` hooks, `decode_at`, `code_hash` | synthetic images: leaders, cut rules, `stop_pc` is a leader, delay-slot grouping, loop ends, both SIMD variants (P4), static budget equals interpreter steps for every block, `code_hash` and `end_pc` set, an all-`Interp` lowering stays exact |
| W1-SVC | 1 | `service/{ring,worker}.rs` | `translate_range` signature | ring order, wrap, full, empty, two-thread stress with a fixed seed; no allocation after `new` (`count_alloc::allocs_in`); worker uses a scratch `Core` and returns what `translate_range` returns for the same words |
| W1-RUN | 1 | `runner/{uop_pass,uop_report}.rs`, `crates/nord-tools/src/bin/{uop_report,lle_sharc_bench}.rs`, `tools/uop_gates.py`, `tools/tests/test_uop_gates.py` | `UopStats`, `PassConfig`, `enable_uop` | stub pass equals interpreter pass; report schema pinned; `model_cycles` pins; adds `uop_pass::code_ranges(core)` (extent of non-zero code in the VISA window) and `--engine uop`, `uop-checked`, `uop-native` in `lle_sharc_bench`; the gate evaluator (written in W0) gains no threshold without a design row |
| W2-DRIVE | 1 | `drive.rs`, `invalidate.rs`, `exec/{mod,exit,event,h_core}.rs` | `BlockBuilder`, `hot.install`, `guard::check` | hand-built all-`Interp` blocks (no translator needed) equal the interpreter; event split; poke forces the checked loop; `smc_*` (own block, other block, page of another block); alias write; revalidation keeps an identical overlay (P6); INTERP rules (P3); `uop_begin_pass` repairs a late `stop_pc` (P7); no allocation in a steady `uop_try` and none in `drop_block` (P14, `count_alloc`) |
| W2-FLOAT | 1 | `lower/float.rs`, `exec/h_float.rs` | kernels API, recipe P8 | per-kind handler-level differential against `Core::step`, PEx and PEy, `ComputeCond` true and false, sticky and exception guard class, paired kernels when `UF_SIMD`; a bailing handler leaves the core untouched |
| W2-FIXED | 1 | `lower/fixed.rs`, `exec/h_fixed.rs` | same | same, plus MR spill, saturation modes, shifter edges |
| W2-MOVE | 1 | `lower/moves.rs`, `exec/h_move.rs`, `exec/mem.rs` | same | same, plus `periph_claims_disjoint_from_sram_windows`, order equals `dm_read` and `dm_write`, post-modify and circular wrap including the I7/I15 latch, short-word alias, local I/M/L/B and SRAM data fillers (P13); records the static instruction count of the hot load and store handlers (design 9.1 prices a Tier 1 move at 18 cycles against about 83 measured in v1) |
| W2-FLOW | 1 | `lower/flow.rs`, `exec/h_flow.rs` | builder hooks (P1), dispatch (P2) | jump, call, return, conditional, delayed branch with 2 slots (`owe_slots`, `UF_FIN` on slots), nested DO UNTIL, loop end as block-final `Fin`, `Sys` terminator then the `due` re-check, IDLE |
| W3-FUZZ | 2 | `verify/{fuzz,lockstep,census,shadow,tv,sweep}.rs` | all of wave 1 merged | non-vacuous end to end: every test asserts `DiffRun::exercised()` (P10); per-family program shapes through `random_program_from`; every `Mutant` class is caught by fuzz, sweep and comparator; census 1,580 cases equal; `firmware_lockstep` uses the same `runner::` functions as `lle_sharc_bench` and skips cleanly without firmware; shadow self-heals a perturbed handler |
| W4-NCORE | 2 | `native/{ctx,emit,call,enter,cache}.rs` | handlers, exit protocol, `UopCtx` layout | call-threaded block equals Tier 1 on fuzz and sweep; `offset_of!` layout test; install, tombstone and free at the pass boundary; x86-64 and arm64 |
| W4-NSVC | 3 | `native/{service,tv}.rs` | `Service`, `native::emit` compile entry | only validated blocks install; a perturbed emitter is rejected and the block poisoned; audio-side install never allocates (`count_alloc`); translation time exposed for K5 |
| W5-NFLOAT | 3 | `native/inline_float.rs` | W4-NCORE | each inline body equals its handler on fuzz and sweep; the family bit toggles it; static counts for K1 and K2 |
| W5-NMOVE | 3 | `native/{inline_move,inline_flow}.rs` | W4-NCORE | same for moves and flow; SRAM window inline; `Smc` after a code-page store |
| W6-NREG | 4 | `native/regpromo.rs` | W5-* and gate K6 | write-back at every exit verified by the sweep; built only if K6 passes |
| W3-DOC | 4 | `docs/specs/sharc-uop.md`, `docs/research/sharc-uop-results.md`, `docs/OPEN-QUESTIONS.md`, `docs/ROADMAP.md`, `docs/DECISIONS.md` (D260 status text only; "Decided by" stays the owner's), the `#[allow(dead_code, unused_imports)]` line on `mod uop` in `core/mod.rs` | everything | docs match code; the results note has Checked, Sources, Question, Implications and Follow-ups |

W3-GATES from the first table is folded into W1-RUN (the evaluator exists since W0). A missing package, real-firmware wiring, is also in W1-RUN. About 70 new files, none over about 400 lines.

### 11.1 Cross-package protocols (what lets the packages run in parallel)

| Id | Rule | Written by | Used by |
|---|---|---|---|
| P1 | Block-end protocol. `BlockBuilder::end_block()` marks the instruction just lowered as the terminator (jump, call, return, `Sys`, IDLE). `owe_slots(n)` records the delay slots a delayed branch still owes and the translator must not cut until `owed_slots() == 0` (it calls `slot_done()` per lowered instruction). `lower_insn` calls `set_end(c.next_pc)` for every instruction. The translator sets `UF_FIN` on the uop of every static loop-end instruction (found by `scan` from `DoUntil`) and on every uop in a delay slot. | W1-LOWER (driver); hooks in W0 | W2-FLOW |
| P2 | Dispatch protocol (header of `exec/mod.rs`). `UF_CB` calls `latch_cb_overflow` when `cb_wrap != 0`. `UF_FIN` calls `finish_step(u.pc, kind, next)` with kind and next from the decode-cache entry at `u.pc`. `Jump` carries the terminator's steps in `done` and the pc in `ctx.next_pc`. `Smc` carries the store's steps and continues at the next uop's pc or `Block::end_pc`. `Fault`, `Guard` and `Slow` did not run the uop and materialize `u.pc`. A uop that needs `finish_step` and also needs both operands for itself is lowered `Interp`. The native tier mirrors all of it. | W2-DRIVE | every family, W4 |
| P3 | `INTERP` rules (header of `exec/h_core.rs`): materialize before `exec_cached` (timers read `steps`), signal `Jump` when `core.pc != u.b`, check `RF_SMC` after an interpreted store, re-check `due`, count `steps_interp_uop`. | W2-DRIVE | everyone |
| P4 | SIMD variants. The mode of PEy is baked into a block and the ORG Core2 pass runs 5,584 of 5,953 steps with PEy on (`sharc-jit-ceiling-study.md`). `translate_range` therefore translates every range twice (`LowerCtx.simd` false and true), `UopHot` holds one `PcMap` per mode (`pcmap(simd)`), the driver looks the pc up in the map of `core.simd()`, and the `Simd` guard row is a safety net that should never fire. Compute and DAG uops of the true variant carry `UF_SIMD`. A block with no mode-dependent uop may be shared (one `Block`, both maps) as an optimization. `UopBuildReport.blocks` counts both variants. | W1-LOWER; `hot.rs`, `drive.rs` in W0 | W2-DRIVE, W2-* |
| P5 | Fall-through. `Block::end_pc` is the pc after the last instruction, where `Exit::Ok` at the end of a block continues. | builder (W0) | W2-DRIVE, W4 |
| P6 | `code_hash`: `lower::decode::code_hash(core, block.pc, block.end_pc)` (FNV-1a 64 over `parcel_at`), stored by the translator and recomputed by revalidation after a write. One function serves both sides. | W1-LOWER (store), W2-DRIVE (compare) | |
| P7 | `stop_pc`. Known at `enable_uop` (set it with `uop_set_stop_pc` first; `enable_uop` keeps it): the translator makes it a block start. Learnt or changed later: `uop_begin_pass(idle_pc)` finds a block that holds `idle_pc` as an interior uop through the pc map and drops it (into `dropped`, retranslated by `uop_maintain`). Without this a block can run past the idle pc and the pass no longer ends where the interpreter's does. | W2-DRIVE, W1-LOWER | W1-RUN |
| P8 | Handler-level test recipe (families, wave 1): build a core, `fill_state`, lower the instruction with `lower_insn` into a `BlockBuilder`, take the handler from `UopHot::handler_table()`, call it with `UopCtx::new(core_ptr, steps, room)`, then `exec::exit::materialize` at `u.pc` plus the done steps and compare with `Core::step` through `exact_diff`. A handler that bails (`Fault`, `Slow`) must leave the core equal to the interpreter state before that instruction; the test finishes the instruction with `Core::step` and compares again. The recipe needs no driver and no translator. | | W2-* |
| P9 | One counting allocator. `verify/count_alloc.rs` defines the only `#[global_allocator]` of the lib test binary; packages call `allocs_in(f)` (counted per thread). A second `#[global_allocator]` anywhere in the crate is a compile error at merge. | W0 | W1-SVC, W2-DRIVE, W4-NSVC |
| P10 | No vacuous tests. With the skeleton nothing is lowered and the executor declines every block, so a differential test passes trivially. A family or driver test asserts `DiffRun::exercised()` (some instruction lowered to a fast uop and `steps_uop > steps_interp_uop`) or, at handler level, that the handler ran. | W0 (`DiffRun`) | everyone |
| P11 | Splitting a file over about 400 lines: replace `x.rs` by `x/mod.rs` plus `x/*.rs` inside the package's own path (same module path, no parent `mod.rs` edit), every new file with its own test. | each package | |
| P12 | A private v1 item the package needs may be widened to `pub(super)` or `pub(in crate::core)` in one line without logic change; the package lists it in its final message. Two packages widening the same item make identical edits and merge cleanly. | | W2-*, W4-* |
| P13 | State fillers for a family's domain (I/M/L/B and the SRAM data window, loop stack, `pending`, stack depth) live in that family's test module; `fill_state` stays registers, flags and float MODE1 bits. Programs come from `random_program_from(rng, pool, n)` with the family's own table; no package edits `PROGRAM_POOL`. | each package | |
| P14 | Revalidation and `drop_block` run on the audio path: no allocation (`install` reserves `dropped` for every block). The freeing of a dropped `page_blocks` list is the one allowed exception and only after a write into translated code. | W2-DRIVE | |

### 11.2 Wave plan

| Wave | Packages (parallel worktrees) | Merge order and integration |
|---|---|---|
| 0 | W0 (done in the working tree) | commit path-limited (11.4), fan out from that commit |
| 1 | W1-VERIFY, W1-KERN, W1-LOWER, W1-SVC, W1-RUN, W2-DRIVE, W2-FLOAT, W2-FIXED, W2-MOVE, W2-FLOW (10 agents, disjoint files) | one integration branch, one PR. Merge VERIFY, KERN, LOWER, RUN, SVC, DRIVE, FLOAT, FIXED, MOVE, FLOW (any order compiles); then `cargo check`, then the check list of `AGENTS.md`, then the fix-up loop below |
| 2 | W3-FUZZ, W4-NCORE | start after the wave-1 integration is green; FUZZ turns the vacuous tests into M1 (all-`Interp`) and M2 (families) evidence |
| 3 | W4-NSVC, W5-NFLOAT, W5-NMOVE | start after W4-NCORE merges; K0 (wall-clock, owner) decides whether W5 merges enabled, never whether it starts |
| 4 | W6-NREG (only if K6 passes), W3-DOC | results note; gates G1 to G3 and K0 to K7 evaluated |

Fix-up loop after wave 1 (the "ultracode fix-up pass" the owner asked for): run `cargo test -p nord-sharc --lib uop`, then fuzz with `NORD_UOP_FUZZ_N=3000`, census and sweep. Each failure names a field and a seed; the owning package (by file) fixes it; re-run the deterministic gates (`uop-report`, `uop_gates.py`). No wall-clock number is taken until G1 and G2 pass.

Milestones: M1 end of W2-DRIVE with all-`Interp` lowering (exactness machinery proven before any fast handler); M2 end of wave 1 plus W3-FUZZ (Tier 1 complete); M3 gates G1 to G3 evaluated (Tier 1 shippable); M4 call-threaded native equals Tier 1, K0 evaluated; M5 inline families, K1 to K5 evaluated; M6 K3 verdict and the hybrid decision.

### 11.3 Readiness review of 2026-11-28 (adversarial, before fan-out)

Verdict: READY after the amendments below, which are applied to the skeleton and to this document. Verified on the working tree: `cargo check` and `clippy -D warnings --all-targets --all-features` clean; the uop unit tests pass. Found and fixed:

| Severity | Finding | Fix |
|---|---|---|
| blocker | Blocks were translated for one SIMD mode (the mode of the core at `enable_uop`, PEy off at reset) while the ORG Core2 pass runs about 94% of its steps with PEy on: the `Simd` guard row would have sent almost every step to `Core::step` and gate G2 could never pass. | P4: two pc maps, both variants translated (`hot.rs`, `drive.rs`) |
| blocker | `Block` had no fall-through pc, so the driver could not continue after `Exit::Ok` at the end of a block cut by size or by a leader. | P5: `Block::end_pc`, builder `set_end` |
| blocker | W1-SVC, W2-DRIVE and W4-NSVC would each define a counting `#[global_allocator]`; only one is allowed per binary. | P9: `verify/count_alloc.rs` |
| high | `Family::lower` sees one instruction, so delayed branches, loop ends and terminators had no protocol between W1-LOWER and W2-FLOW. | P1: builder hooks `end_block`, `owe_slots`, `slot_done` |
| high | `INTERP` semantics were underspecified: it must materialize before `exec_cached`, signal `Jump` when a loop back-edge or branch moved the pc, check `RF_SMC` after an interpreted store, and count `steps_interp_uop` (nobody did, so G2 would read 0). | P3, header of `exec/h_core.rs` |
| high | The skeleton tests pass vacuously: with no translator and a declining executor every differential test equals the interpreter by construction. | P10: `DiffRun::exercised()`; W3-FUZZ owns the end-to-end evidence |
| high | `stop_pc` was written as "a block ends after stop_pc"; the pass ends when a step LANDS on it, so it must be a block start, and a pc learnt after translation must repair existing blocks. | P7, header of `lower/cut.rs` |
| high | `PROGRAM_POOL` was one shared table every package would append to (merge conflict, wrong owner). | `random_program_from`, P13 |
| high | `code_hash` had no shared function, and `Block` cannot hold the words for the "full compare up to 64 words" of risk 15. | P6: `lower::decode::code_hash`; risk 15 restated |
| high | `drop_block` pushed to `dropped` on the audio path (allocation). | P14: `install` reserves; test in `hot.rs` |
| high | No package owned real-firmware wiring: nothing connected an `Engine::Uop` run to `lle_sharc_bench`, so G1 lockstep, the G2 counters on ORG Core2 and the G3 wall-clock A/B had no entry point. | W1-RUN owns `lle_sharc_bench --engine uop` and `code_ranges` |
| medium | `kernels/mod.rs` (holds `FlagsOut` and `F40`) had no owner, and W1-KERN would have conflicted with "mod.rs is W0-only". | W1-KERN owns it |
| medium | W1-KERN ignored that #685 is merged: exact scalar kernels (`aot_fast`) and exact paired SSE2 kernels (`aot_simd`) exist. Rewriting them wastes the wave, and the paired kernels halve SIMD-mode add, sub and mul. | W1-KERN reuses them; the Tier 2 floor of 9.2 must be re-estimated with the paired kernels (the 675k-instruction spike predates them) |
| medium | "instruction-count assertion in a unit test" is not implementable in Rust. | `tools/uop_insn_count.py` over `objdump -d` of probe symbols, owner-run |
| medium | Two packages adding sibling files need a parent `mod.rs` edit they do not own. | P11 |
| medium | `smc` was not a gate: a data store that shares a 256-word page with code costs a revalidation per store and shows in no other counter. | G2 row and the `tools/uop_gates.py` check |
| low | W3-GATES was already implemented in W0; W3-DOC could not tick ROADMAP (W0-owned). | folded into W1-RUN; W3-DOC owns the doc files |
| low | The Sources line said the worktree was 106 commits behind; `HEAD` equals `origin/main` now. | fixed |

Open (not fixable in the skeleton; owned by a package or by the owner):

| Item | Owner | Note |
|---|---|---|
| Tier 1 moves are priced at 18 cycles in 9.1 against about 83 cycles per lowered transfer measured in v1 (`sharc-lle-realtime.md`, stage B); nothing supports the 4.6x. | W2-MOVE records static instruction counts; G3 decides | the Tier 1 A/B gate (x1.4 over v1) is where it shows |
| 520 blocks per pass at 12 steps each is ASSUMED. The JIT classification run counted 1,971 distinct blocks and 4,208 distinct pcs in a 5,953-step pass; basic blocks cut at every flow, `Sys` and loop end may be shorter than 12 steps, and the transition cost (38 cycles) scales with the count. | W1-RUN histogram | recompute the 9.1 total from the histogram at M3 |
| `Slow` exits per pass <= 20 (G2) against about 14 blocks per pass refused by the v1 entry guard because an interrupt-controller event falls inside them (`sharc-lle-realtime.md`, 255 steps, 9.4% of the time). The margin is small. | G2 owner | recalibrate at M3 |
| The orchestrator's estimate of 100 to 140k cycles (x0.5 to x0.7) for a "real compiler" contradicts the repo's spike: 675k x86 instructions at IPC 3.37 is about 200k cycles with inline kernels, and the compute part alone is about 140k after the avg diet. It can only hold if the paired kernels and a better IPC cut compute well below the spike. K1, K2 and K3 measure it; the design does not assume it. | K gates | keep the hybrid verdict until K3 |
| Control flow of the ORG pass is not measured as data independent (it is for PNO Core1: 2,233 pcs with 0, 1 and 8 keys). If it is, a per-pass trace or superblock specialization could remove most transition cost. Not designed here. | follow-up | count distinct pc sequences over several passes in `uop-report` |
| `uop_maintain` retranslates synchronously at the end of a pass after an overlay: about 1 us per instruction of the dropped range, a pass-time spike. `service/` can carry it off-thread (scratch core), but nothing wires `async_translate` for Tier 1 yet. | W1-SVC, W2-DRIVE | acceptable for a program change, not for steady passes (gate `maintain_calls == 0`) |
| Decision numbering: D260 is taken by this track; the ARM0 JIT design (`docs/research/arm0-jit-design.md`) says "D260 and up" and must take D261 or the next free number after fetching. | orchestrator | tell the arm track |
| Merge conflicts with open PRs are mechanical: #683, #688 and #689 edit `docs/DECISIONS.md`, `docs/ROADMAP.md`, `docs/specs/README.md`, `crates/nord-tools/Cargo.toml` (and #688 also `core/mod.rs` and `core/new.rs`) at the same anchors as W0. | integrator | take both sides; no logic overlaps |

### 11.4 Fan-out hygiene (commit of W0)

The worktree holds two tracks. The W0 commit must be path-limited: stage only the sharc paths, `tools/uop_gates.py`, `tools/tests/test_uop_gates.py` and the three doc files of this track (`docs/DECISIONS.md`, `docs/ROADMAP.md`, `docs/specs/README.md` hunks for D260 and the uop rows, plus `docs/specs/sharc-uop.md` and `docs/research/sharc-compiler-design.md`), and stage `crates/nord-tools/Cargo.toml` with `git add -p`, taking only the `uop-report` `[[bin]]` hunk (the `sb-native` feature and the `arm-sb-bench` and `arm-sb-ab` bins belong to the arm track and name files that are not in the commit). `Cargo.lock` is not needed (no new dependency): leave it to the arm track. Use `git commit -s`, a Conventional Commits subject, no attribution lines, no push. Every wave-1 worktree is created from that commit.

## 12. Gates

Counters are deterministic and come from `uop-report`. Wall-clock numbers are taken once per tier by the owner, never by implementers per commit. All thresholds are ASSUMED decision thresholds, not measurements.

| Gate | When | Condition to pass | Result when it fails |
|---|---|---|---|
| G0 | W0 | skeleton acceptance of 10.5; with the engine off nothing changes | fix the skeleton before any package starts |
| G1 exactness | M3 | `exact_diff` empty on 1,580 census cases; fuzz 200,000 blocks per family release profile; real-trace lockstep 1:1 on ORG, PNO, FX, SY1SY2, SMP steady and playing; every mutant class caught | no performance work until fixed; a surviving mutant is a verifier bug |
| G2 structure (Tier 1, ORG Core2 and PNO Core2) | M3 | uops per step <= 1.35; native step share >= 90% (`Interp` uops <= 6% of steps); `Slow` exits <= 20 per pass; `maintain_calls == 0` and `smc == 0` in steady passes; no allocation counter increments in `uop_try` | rework lowering (fusion, cut rules, guard classes) before measuring time |
| G3 performance (Tier 1) | M3, wall-clock once | A/B over v1 AOT >= x1.4 (ORG Core2 pass <= 115 us, v1 is 161 us); target >= x2.0 (<= 80 us) | A/B < x1.4: Tier 1 loses its performance role, keep it only as the exactness fallback and stop Tier 2 |
| G3-RT per core | M3, wall-clock once | steady pass <= 1.3 x 46.08 = 59.9 us | the real-time claim is killed for that core on Tier 1; ORG Core2 is expected to fail here (model 76 us) and relies on Tier 2 |
| K0 IPC probe | after W4-NCORE merges; gates the MERGE of W5, not its start | call-threaded block 6 (numbering as in `sharc-jit-ceiling-study.md`) <= 70 cycles per step | > 70: stop the native tier, lock the hybrid for ORG Core2, keep Tier 1 |
| K1 static size | M5 | block 6 <= 310 static x86 instructions per step with inline families (<= 420 with call-threaded only) | above: do not enable that family; re-plan emission before register promotion |
| K2 kernels | M5 | add, sub, mul <= 1.5x the spike kernels (3.3 and 2.7 ns per element, so <= 5.0 and <= 4.1 ns); avg <= 260 x86 instructions per both-element op, target 150-200 | above: that family stays call-threaded |
| K3 model | M5 | modeled cycles per ORG Core2 pass (counters x section 5 costs) <= 1.4 x budget = 290k cycles | above: **lock the hybrid for ORG Core2; stop the exact real-time attempt there**; PNO and FX may continue on their own K3 |
| K4 real-time claim | M5, wall-clock once | median <= 0.85 x 46.08 = 39.2 us and p99 <= 1.3 x 46.08 = 59.9 us, on the playing state | above: no real-time claim; native tier stays an accelerator |
| K5 translation | M5 | ORG Core2 translation <= 5 s single-thread, worker only | above: lazy translation by hotness; if still above, drop async install |
| K6 specialization | before W6 | tag/type specialization applicable to >= 60% of float ops | below: W6-NREG is not built |
| K7 playing state | M5 | playing-state cost <= 1.3 x idle-state cost | above: gates K3 and K4 use the playing numbers |

The brief's gate wording is kept: stop the exact real-time attempt when K0 fails or when K3 models above 1.4 x budget (about 290k cycles); G3 below x1.4 over v1 removes Tier 1's performance role; steady pass above 1.3 x budget kills the real-time claim.

## 13. Skeleton and package risks addressed by structure

| Concern | Structural answer |
|---|---|
| Parallel packages colliding on shared files | all shared files are W0-only; families register handlers through their own `register` fn; `UopKind` ranges are per family; `Family` trait per lowering file |
| Existing-file conflicts with open PRs (lle5-engine #683, aot2-cd #688, kernel-simd #685) | W0 edits are small, mechanical and made once on a fresh `origin/main`; no logic in v1 files |
| Hot path regressions in v1 | hook functions early-return on `uop_run.code_pages` empty; engine off by default; golden hashes unchanged is a W0 acceptance item |

## 14. Open risks

1. Cost model is unmeasured. Every cycle figure derives from the v1 stage B number and the spike; per-class costs (dispatch 6, transition 38, `Interp` 100) are ASSUMED. K0 and G3 replace them with data. Confirm: one wall-clock run per tier.
2. IPC 3.37 was calibrated on the spike, which keeps state in registers. Call-threaded handlers with state in memory may run at lower IPC. Confirm: K0.
3. The hidden-state comparator may expose divergences that v1 `state_mismatch` hid (it compares effective f40 only). The interpreter is the oracle, so any such divergence is a bug in v1 or the interpreter to be recorded, not a reason to weaken the comparator.
4. Parallel sessions touch the same area (`aot2-cd` #688, `kernel-simd` #685, `lle5-engine` #683). #685 (kernel-simd) is merged: W1-KERN reuses `aot_fast`, `aot_fadd`, `aot_round` and the paired `aot_simd` kernels instead of rewriting them.
5. Widening a private v1 item to `pub(super)` for `uop` access is a logic-free edit but still touches v1 files; W0 batches them in one change.
6. `UopChecked` and shadow mode need a snapshot of cores with peripherals; peripheral state may not be capturable. Fallback: restrict those modes to blocks and cores without peripheral access, count skips, rely on census and fuzz for the rest.
7. Real firmware lockstep cannot run in CI (D128, no firmware in git). CI relies on synthetic fuzz, census and exit sweep; real-trace confidence is local only.
8. Debug-profile CI time: fuzz iteration counts must stay small by default (`NORD_UOP_FUZZ_N`), and macOS arm64 runs the Cranelift tests too. Confirm with the first CI run of W1.
9. Self-modifying code and DMA into code pages mid-pass: covered by the `Smc` exit and revalidation, but overlay behaviour on real program changes is only proven by lockstep on a program-change trace. Page size 256 words is ASSUMED.
10. Aliasing and unsafe: `&mut Core` plus raw pointers in handlers is the main soundness risk. Discipline: one raw pointer derived at `run_block` entry, no `&mut Core` alive across handler calls, handlers never re-enter `run_block`, no panics across `extern "C"`. Miri cannot cover the native part.
11. Determinism across x86-64 and arm64: kernels use the f64 add trick; no fused multiply-add may be emitted by Rust or Cranelift. Test on all three CI targets with fixed vectors.
12. The real-time goal itself may not be reachable on ORG Core2: raw Tier 2 is x1.14 against a claim threshold of x0.85. The hybrid verdict then stays, with Tier 1 and Tier 2 as faster exact LLE for non-real-time use (offline render, verification, lighter cores).
13. Block-level specialization on `simd`: resolved by structure, not left open. Every range is translated for both PEy modes and the driver picks the variant by `core.simd()` through one pc map per mode (P4). The cost is double translation work and double arena size (K5 is checked on the sum); `guard_fail[simd]` should stay near 0.
14. K-gate block numbering ("block 6") and K1 instruction counts come from the brief and the spike; recalibrate them at K0 before using them to stop work.
15. Hash-based revalidation (`code_hash`, FNV-1a 64 over the words of `block.pc..block.end_pc`) trusts the hash; a collision keeps a stale block. `Block` has no field for the words, so the skeleton keeps the hash alone (collision odds about 2^-64 per compare, ASSUMED acceptable); shadow mode and `UopChecked` catch a stale block in development. A word-compare table for small blocks is a follow-up if a collision is ever observed.
16. Doubled translation (both PEy modes) and a synchronous `uop_maintain` after an overlay put a spike of about 1 us per instruction into one pass. Acceptable for a program change; steady passes never reach it (G2 `maintain_calls == 0`). Moving it off the audio thread through `service/` is a follow-up.
17. Wave 1 tests each package at handler or unit level only (recipe P8); integration defects show first at W3-FUZZ. The fix-up loop of 11.2 names the owning package from the failing field and file, so one integration PR with a loop is cheaper than per-package gates. Merge order and the loop are part of the plan, not an afterthought.
18. The skeleton worktree holds two tracks (SHARC uop and ARM0 JIT). The W0 commit must be path-limited (11.4) or the first fan-out inherits unrelated arm files and their `Cargo.toml` entries.

## 15. Implications

- Decision to take: record D260 "two-tier exact uop engine for SHARC LLE" (status `proposed`, W0 writes it, the user decides). Tier 1 is worth building regardless of Tier 2: about x2.1 over v1 on ORG Core2 and the foundation for everything else.
- The expensive part (Cranelift) is gated twice (K0 before W4, K3 before W5 finishes), so a no-go costs one wave, not the whole plan.
- The exact real-time claim on ORG Core2 is not expected to survive K4. The value of the plan is a verified, fast, exact engine and a measured basis for the hybrid decision, with real time achievable on lighter cores.
- The verification stack (comparator, fuzz, sweep, shadow, translation validation, mutation tests) is reusable for any later fast path, including the kernel SIMD work.
- No new firmware-derived file enters git: blocks are built at run time from the user's own firmware; `tools/fw-derived-baseline.txt` is unchanged; D20 and D128 hold.

## 16. Follow-ups

- Commit W0 path-limited (11.4), then fan out wave 1 (11.2). The existing-file edit list of 10.2 was confirmed against the live code by the readiness review (11.3).
- Tell the ARM0 JIT track that D260 is taken by this track and that its decisions start at D261 (or the next free id after a fetch).
- Re-estimate the Tier 2 floor of 9.2 with the merged paired kernels (the 675k-instruction spike predates them), before K1 and K2 are evaluated.
- Collect one wall-clock calibration of the section 5 per-class costs at K0 and replace the ASSUMED tags with EMPIRICAL ones.
- Capture a program-change (overlay) trace for lockstep, and a playing-state trace per core for K7.
- Decide `UopChecked` scope for cores with peripherals (open risk 6).
- After M3, record results in `docs/research/sharc-uop-results.md`, move answered items out of `docs/OPEN-QUESTIONS.md`, and tick `docs/ROADMAP.md`.
- Optional later: read-only parallel probe on the footprint (S-D), a Fast f32 mode (out of scope here), PNO and FX per-core K3 evaluation.
