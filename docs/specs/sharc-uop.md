# SHARC uop engine (D260)

Status: **skeleton (W0)**. The module tree, types, driver, invalidation, verification harness and gate tooling exist and are tested; every lowering family is a stub that returns "not handled", so each instruction becomes an `Interp` uop and the engine is bit-exact by construction. Design, scores and work packages: [sharc-compiler-design](../research/sharc-compiler-design.md). Decision: [D260](../DECISIONS.md) (proposed).

Code: `crates/nord-sharc/src/core/uop/`, `runner/uop_pass.rs`, `runner/uop_report.rs`, `crates/nord-tools/src/bin/uop_report.rs`, `tools/uop_gates.py`.

## 1. Scope and invariants

| Rule | Where pinned |
|---|---|
| `Core::step` is the only oracle. The engine reproduces its state at a uop boundary on every exit. | `verify/compare.rs` (`exact_diff`, 51 named fields) |
| Off by default. A core that never calls `enable_uop` behaves as before; `uop_try` returns false. | `hook_is_noop_when_disabled`, `engine_default_is_interp` |
| No instruction is guessed. Anything the lowering does not model is an `Interp` uop (runs the interpreter body verbatim). | `lower/mod.rs` `lower_insn` |
| No allocation, lock or I/O in `uop_try`, `run_block`, handlers, `lean_pass_uop`, revalidation and `drop_block` (`install` reserves `dropped`). `uop_maintain` may allocate and runs only after a program change or overlay load. | gate G2 (`maintain_calls == 0`, `smc == 0`), `count_alloc` tests |
| `INTERP` runs `exec_cached` verbatim: it materializes before the call, signals `Jump` when `core.pc != u.b`, checks `RF_SMC` after a store, re-checks `due`, counts `steps_interp_uop`. | header of `exec/h_core.rs`, design 4.5 rule 5 |
| `stop_pc` (the pass idle pc) is a block start, never an interior pc; `uop_begin_pass` repairs a late one. A cut never separates a delayed branch from its owed slots. | header of `lower/cut.rs`, design P7 |
| No firmware bytes or derived tables in git (D20, D128). Census replay reads the committed hardware-measured cases only. | `tools/check_fw_derived.py` |
| Feature `uop-native = ["jit"]` (Tier 2, Cranelift) is not in `default`. | `crates/nord-sharc/Cargo.toml` |

## 2. Types (`types.rs`)

| Item | Definition |
|---|---|
| `Uop` | 16 bytes, `repr(C, align(16))`: `h:u16` handler index, `steps:u8` (0 for the second uop of one instruction, 2 or 3 fused), `flags:u8` (`UF_FIN`, `UF_CB`, `UF_LAST`), `a:u32`, `b:u32`, `pc:u32` (short words, first covered instruction). Test `uop_is_16_bytes`. |
| `Handler` | `unsafe extern "C" fn(*mut UopCtx, *const Uop) -> u32`, returns an `Exit` code. |
| `Exit` | `Ok, Jump, Fault, Guard, Smc, Slow`. `Slow` = a uop does not fit the event room; the driver finishes with `Core::step`. |
| `UopKind` ranges | `0x000` core (`NOP`, `INTERP`, `FIN`, `BAIL`), `0x100` float, `0x200` fixed, `0x300` move, `0x400` flow, `MAX = 0x500`. A family adds constants in its own file only. |
| `Block` | `pc`, `last_pc` (last instruction), `end_pc` (pc after it: the fall-through successor), uop range, static step budget, cumulative steps per uop boundary, `simd`, `isa`, `code_hash` (`lower::decode::code_hash(core, pc, end_pc)`), `poisoned`, `native` slot, `Link`s. |
| `GuardClass`, `Footprint` | entry-guard classes and the state a block reads or writes (parallel-probe seed, not used yet). |

## 3. State and API

| Item | Role |
|---|---|
| `UopRun` (in `Core`, by value) | `active`, `flags` (`RF_SMC`, shadow, trace), `dirty` (u64, bit k = word k of the `CodePages` bitmap), `CodePages` (256 short words per page, 4096 pages from short word `0x10_0000`), `UopStats`, stop pc, pending fault. Allocation-free to construct. |
| `UopHot` (boxed, taken out during `uop_try`) | blocks, uop arena, `pcmaps` (one `PcMap` per PEy mode; `pcmap(simd)`), `page_blocks`, `dropped`, `epoch`, handler table. |
| `Core::enable_uop(cfg, ranges)` | validates the request (native needs the feature; ranges inside the code window), translates, installs, marks pages, arms the hooks. |
| `disable_uop`, `uop_enabled`, `uop_stats`, `uop_reset_stats` | lifecycle and counters. |
| `uop_try(limit) -> bool` | a chain of blocks from `pc` (`drive/chain.rs`): revalidate dirty pages, find the block (cached `Link` while its epoch is current, else the pc map of `core.simd()`), entry guard (a failing row is counted for the first block of the call only), run, count; continues after `Ok`, `Jump` and `Smc` unless `pc == stop_pc`, no step ran or a fault is pending. False means no step ran and the caller runs one `Core::step` (`steps_core_step += 1`); a `Slow` or `Guard` exit with no progress in a later block of the chain is replayed by the driver, so each exit is counted once. A `Fault` from a fast handler is replayed through `Core::step` (the interpreter's fault and partial effects); `Interp` stores its own. |
| `run_uop(n)` | `Core::run` with the hook; fault returned like `run`. |
| `uop_begin_pass(idle_pc)`, `uop_take_fault`, `uop_set_stop_pc` | pass runner support. |
| `uop_note_write(first_short, count)`, `uop_note_all()` | called from `invalidate_sram_cache` and `invalidate_code_cache`; early return while `uop_run.active` is false. |
| `uop_revalidate_dirty`, `uop_maintain`, `uop_needs_maintenance`, `uop_dirty` | invalidation (section 5). |
| `uop_maintain_async(&mut Service)`, `uop_submit_dropped`, `uop_collect`, `uop_install_done` | the same through the translation service; the caller owns the `Service` (the frozen state types have no slot for one). |
| `lean_pass_uop`, `lean_pass_uop_checked` | pass loops (`runner/uop_pass.rs`). |
| `Engine::{Uop, UopChecked, UopNative}` | census and pass-scheduler selection; `#[default] Interp`. |

## 4. Entry guard and event split

A block runs only when all of these hold; else `Exit::Guard` and one `Core::step`. Rows map to `UopStats.guard_fail[row]`, names in `lower/guard.rs::name`.

| Row | Condition |
|---|---|
| Steps | `room = min(limit, irq.due) - steps` is not 0 (a block longer than `room` runs the checked loop, which exits `Slow` at the first uop that does not fit) |
| Window | `irq.window_len == 0`, `!irq.idle`, `irq.stall == 0` |
| Pending | `pending.is_none()` |
| Loop | no active `LoopFrame` ends inside the block |
| Simd | `core.simd() == block.simd`; the driver already looks the pc up in the map of `core.simd()` (both PEy variants of every range are translated: ORG Core2 runs 5,584 of 5,953 steps with PEy on), so this row is a safety net |
| Hooks | `trace.is_none() && site_hits.is_empty()` |
| Cb | `cb_wrap == 0` unless the block has `UF_CB` uops |
| Arith | no arithmetic-interrupt enable unless the block cannot raise one |
| Valid | `!block.poisoned`, ISA at pc equals `block.isa` |

Event split (`exec/event.rs`, `exec/dispatch.rs`): `Core::step` looks at the controller only when `steps >= irq.due`, so a block entered at `steps` may run `due - steps` steps unchecked. A handler that may poke the controller (`INTERP`, the wrap latch, `finish_step`) lowers `ctx.room` to `event::due_room`; the dispatch loop re-checks the room after every uop and runs the rest of the block checked, so the first uop that no longer fits exits `Slow` in front of itself and the interpreter prologue sees the event at the step where it would have seen it. This replaces "the uop ends with `return Slow`" of design 4.5 rule 4: `Slow` means the uop did not run, so a uop that ran cannot return it. Every exit materializes `pc` and `steps` at the uop boundary (`exec/exit.rs`).

Events raised inside a block follow design section 7.3: a system-register write is a block terminator; a peripheral access re-checks `irq.due` after the access; the I7/I15 wrap latch is processed by `UF_CB` uops; `IDLE` runs through the interpreter body.

## 5. Invalidation

1. `uop_note_write` ORs the bitmap words a write touches into `uop_run.dirty` and, when the write hits a code page, sets `RF_SMC` and counts `smc`; `uop_note_all` marks everything. Both hooks use `Core::uop_run` only, never `UopHot`.
2. A store handler checks `uop_run.flags` once after the write; `RF_SMC` returns `Exit::Smc` after the store completed.
3. At the next block entry (`uop_chain`), `uop_revalidate_dirty` walks the blocks registered on the pages of the dirty bitmap words and recomputes `lower::decode::code_hash` over `block.pc..block.end_pc` (the function the translator stored). A block whose hash still matches is kept, so an overlay that reloads identical code and a data store into the page of a block cost a compare (P6). The others are dropped: tombstone, clear the `PcMap` entries, push `(pc, last_pc + 3)` to `dropped`, `epoch += 1`. The walk is in place and allocates nothing (P14); `drop_block` pushes into capacity `UopHot::install` reserved.
4. Dropped ranges run in the interpreter; `uop_maintain` (pass boundary, only when `uop_needs_maintenance()`) merges them, retranslates and installs them, and compacts the tombstones once there are at least 8 (ASSUMED) and more than live blocks: the arena, block list, pc maps, page lists and links are rebuilt and `epoch` moves. Retranslation keeps the stop pc.
5. Async path: `uop_submit_dropped` sends the merged ranges to the service (ISA-mode code and a full request ring stay with the caller: translated synchronously, respectively queued again), `uop_collect` installs the answers through `uop_install_done`, which recomputes `code_hash` on the live core and does not install a block whose code changed since the request (its range goes back to `dropped`). An empty answer means "not translatable": nothing is installed and nothing is retried. A block whose pc, `end_pc` and hash match a live block is a twin and is skipped.
6. Known limitation: a range that retranslates to zero blocks leaves no block to drop later, so it stays interpreted until the next `enable_uop`. A dropped range is `last_pc + 3`, up to two parcels past the block, so a block retranslated again and again widens by up to two parcels per round (bounded by `max_block_steps`).

## 6. Cost model and report

`runner/uop_report.rs` turns counters into modeled host cycles. All costs are ASSUMED until gate K0: dispatch 6, transition 38, `Interp` step 100, Tier 1 work 42 per step, native 38 per step all-in. Reference: ORG Core2, 6,280 steps, 8,478 uops, 520 blocks, 125 `Interp` steps gives about 340k cycles (Tier 1) and about 236k to 246k (Tier 2). Tests pin both totals.

`uop-report` (nord-tools) prints one JSON object, schema 1, hand-written, counts only:

`{"schema":1,"label":s,"stats":{passes, blocks_run, uops, steps_uop, steps_interp_uop, steps_core_step, steps_native, exits[6], guard_fail[9], smc, translations, poisoned, maintain_calls, shadow_checks, shadow_mismatch, shadow_skipped},"model":{tier1_cycles, tier2_cycles}}`

The skeleton binary arms the engine on a synthetic NOP program (no firmware) and is the schema and wiring check; with firmware the same report comes from the runner that owns the core.

## 7. Gates (`tools/uop_gates.py`)

Reads the report JSON only; exit 0 pass, 1 fail, 2 bad input. All thresholds are ASSUMED decision thresholds.

| Gate | Condition |
|---|---|
| G2 | uops per step <= 1.35; `Interp` plus `Core::step` steps <= 6% of steps (fast share >= 90%); `Slow` exits <= 20 per pass; `maintain_calls == 0` and `smc == 0` in steady passes |
| K3 | modeled Tier 2 cycles per pass <= 290,000 (1.4 x 207,360) |

The other gates of design section 12 need wall-clock or firmware runs and are evaluated by their owners (G1 by `verify/`, G3 and K4 by the A/B driver).

## 8. Verification (`verify/`)

| Module | What it checks |
|---|---|
| `compare` | `exact_diff(a, b) -> Option<ExactDiff>` over `FIELDS`; a perturbation test changes each field and demands a diff |
| `snap` | `CoreSnap::capture/restore` (registers and sequencer, not SRAM or peripherals); round trip identity |
| `gen` | deterministic xorshift64* `Rng`, edge words, `fill_state`, `random_program_from(rng, pool, n)` (each family brings its own table; `PROGRAM_POOL` and `random_program` stay the shared default), `core_with_code` |
| `mutate` | 10 mutant classes wrapped around handlers through a thread-local inner table; each must change the diff |
| `fuzz` | `diff_program_run` returns a `DiffRun` (diff, build report, stats) whose `exercised()` is true only when something lowered to a fast uop and ran; `diff_program`, `fuzz` wrap it; iterations from `NORD_UOP_FUZZ_N` (default 100, release gate 200,000 blocks per family). A test that never reaches `exercised()` is vacuous |
| `count_alloc` | test-only: the one `#[global_allocator]` of the lib test binary and `allocs_in(f)` (per-thread count) for the no-allocation assertions |
| `census` | replays the committed hardware-measured single-instruction cases under `Engine::Uop` and `UopChecked` |
| `shadow` | `shadow_run_with`: engine result and interpreter result compared, interpreter result wins |
| `tv` | translation validation on random states through the shadow check |
| `sweep` | exit sweep: every uop boundary of a block is a valid exit |
| `lockstep` | firmware lockstep, skips cleanly without `NORD_UOP_FW` |

## 9. Work packages and ownership

Each stub file starts with `OWNER: <package>`; after W0 a file belongs to one package. `types.rs`, `config.rs`, `run.rs`, `hot.rs`, the top `mod.rs` and existing files change only by amendment; a directory `mod.rs` belongs to the package its header names. The full table (files, interfaces, acceptance) and the protocols P1 to P14 are in design section 11. Waves are merge waves.

| Wave | Packages |
|---|---|
| 1 (independent, parallel worktrees) | W1-VERIFY, W1-KERN, W1-LOWER, W1-SVC, W1-RUN (also owns the gate evaluator and the `lle_sharc_bench` wiring), W2-DRIVE, W2-FLOAT, W2-FIXED, W2-MOVE, W2-FLOW |
| 2 | W3-FUZZ, W4-NCORE (feature `uop-native`) |
| 3 | W4-NSVC, W5-NFLOAT, W5-NMOVE |
| 4 | W6-NREG (only if K6), W3-DOC |

## 10. Provenance and open items

- Constants in the cost model, the page size (256 short words), the 8 translation-validation states and every gate threshold are `ASSUMED`; nothing here is `VERIFIED` against hardware beyond the layout tests (`uop_is_16_bytes`, `code_pages_cover_short_window`, handler-table order, `CTX_*` offsets).
- Open, W1-LOWER (`lower/guard.rs`): the `Window` row tests `irq.stall != 0`, but `irq.stall` is the cumulative cycle offset (`cycles() = steps + stall`; interrupt entry and IDLE wake-up add to it and it is never cleared), so after the first interrupt no block ever passes the guard. The row should test `window_len != 0 || idle` as `aot_try` does, or the transient condition it means. The W2-DRIVE event tests hold with either guard (equality with the interpreter; exact counters only up to the interrupt).
- Tier 2 sources under `core/uop/native/` are stubs (`enter` returns `Slow`, emitters emit nothing). `UopNative` degrades to Tier 1.
- Real-time on ORG Core2 stays unlikely (design 9.2: x1.14 raw); the hybrid verdict in `sharc-lle-realtime.md` stands until K3 passes.
