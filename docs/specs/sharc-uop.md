# SHARC uop engine (D260)

Status: **Tier 1 integrated (wave 2)**. The module tree, types, driver, invalidation, verification harness, gate tooling and the float, fixed, move and flow families exist and are tested against `Core::step`; an instruction no family takes is an `Interp` uop, which is bit-exact by construction. Tier 2 (native) is still a stub. Design, scores and work packages: [sharc-compiler-design](../research/sharc-compiler-design.md). Decision: [D260](../DECISIONS.md) (proposed).

Code: `crates/nord-sharc/src/core/uop/`, `runner/uop_pass/`, `runner/pass_sched/`, `runner/uop_report/`, `crates/nord-tools/src/bin/uop_report.rs` (+ `uop_report/`), `crates/nord-tools/src/bin/lle_sharc_bench.rs`, `tools/uop_gates.py`.

## 1. Scope and invariants

| Rule | Where pinned |
|---|---|
| `Core::step` is the only oracle. The engine reproduces its state at a uop boundary on every exit. | `verify/compare.rs` (`exact_diff`, 51 named fields) |
| Off by default. A core that never calls `enable_uop` behaves as before; `uop_try` returns false. | `hook_is_noop_when_disabled`, `engine_default_is_interp` |
| No instruction is guessed. Anything the lowering does not model is an `Interp` uop (runs the interpreter body verbatim). | `lower/mod.rs` `lower_insn` |
| No allocation, lock or I/O in `uop_try`, `run_block`, handlers, `lean_pass_uop`, revalidation and `drop_block` (`install` reserves `dropped`). `uop_maintain` may allocate and runs only after a program change or overlay load. | gate G2 (`maintain_calls == 0`, `smc == 0`), `count_alloc` tests |
| `INTERP` runs `exec_cached` verbatim: it materializes before the call, signals `Jump` when `core.pc != u.b`, checks `RF_SMC` after a store, re-checks `due`, counts `steps_interp_uop`. | header of `exec/h_core.rs`, design 4.5 rule 5 |
| `stop_pc` (the pass idle pc) is a block start, never an interior pc; `uop_begin_pass` repairs a late one. A cut never separates a delayed branch from its owed slots. `Core::enable_uop` saves the stop pc across `disable_uop` and restores it before it translates, so the first translation honours it (a stop pc set before arming gives the same build as arming alone); `uop_maintain` keeps it for every retranslation. | header of `lower/cut.rs`, design P7; `arming_cuts_the_first_translation_at_the_idle_pc_which_a_bare_enable_does_not`, `the_stop_pc_set_at_arming_cuts_the_retranslation_too` |
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
| `lean_pass_uop`, `lean_pass_uop_checked` | pass loops (`runner/uop_pass/`: `mod.rs` the engine loop, `checked.rs` the interpreter replay with `shadowed_with`, `diff_of`, `checked_with`; `*_entries` forms report each block entry to a callback for the histogram; `engines.rs` the engine names and `arm_for`). The checked tests get a wrong engine from a test-only hook (`support::mark_wrong_engine`): the engine equals the interpreter, so without a wrong one no test could show that a check sees a difference. |
| `run_ticks`, `run_ticks_with` | threaded pass scheduler (`runner/pass_sched/`). A panic in a core's pass stops every core at once (shared abort flag) and returns `Err("core NAME panicked in pass N: ...")`; the bench exits non-zero. `CoreSeries::uop_at_warmup` is the counter snapshot taken when the first measured pass starts. |
| `Engine::{Uop, UopChecked, UopNative}` | census and pass-scheduler selection; `#[default] Interp`. |

## 4. Entry guard and event split

A block runs only when all of these hold; else `Exit::Guard` and one `Core::step`. Rows map to `UopStats.guard_fail[row]`, names in `lower/guard.rs::name`.

| Row | Condition |
|---|---|
| Steps | `room = min(limit, irq.due) - steps` is not 0 (a block longer than `room` runs the checked loop, which exits `Slow` at the first uop that does not fit) |
| Window | `irq.window_len == 0`, `!irq.idle`. `irq.stall` is not tested: it is the cumulative cycle offset (`cycles() = steps + stall`; interrupt entry and IDLE wake-up add to it, nothing clears it), so testing it would send every block after the first interrupt to `Core::step`. Same choice as `aot_try`; pinned by `blocks_keep_running_after_the_first_interrupt_and_stay_equal_to_the_interpreter` |
| Pending | `pending.is_none()` |
| Loop | no active `LoopFrame` ends inside the block |
| Simd | `core.simd() == block.simd`; the driver already looks the pc up in the map of `core.simd()` (both PEy variants of every range are translated: ORG Core2 runs 5,584 of 5,953 steps with PEy on), so this row is a safety net |
| Hooks | `trace.is_none() && site_hits.is_empty()` |
| Cb | `cb_wrap == 0` unless the block has `UF_CB` uops |
| Arith | no arithmetic-interrupt enable unless the block cannot raise one |
| Valid | `!block.poisoned`, ISA at pc equals `block.isa` |

Event split (`exec/event.rs`, `exec/dispatch.rs`): `Core::step` looks at the controller only when `steps >= irq.due`, so a block entered at `steps` may run `due - steps` steps unchecked. A handler that may poke the controller (`INTERP`, the wrap latch, `finish_step`) lowers `ctx.room` to `event::due_room`; the dispatch loop re-checks the room after every uop and runs the rest of the block checked, so the first uop that no longer fits exits `Slow` in front of itself and the interpreter prologue sees the event at the step where it would have seen it. This replaces "the uop ends with `return Slow`" of design 4.5 rule 4: `Slow` means the uop did not run, so a uop that ran cannot return it. Every exit materializes `pc` and `steps` at the uop boundary (`exec/exit.rs`).

For the families (W2-MOVE generic memory outside the SRAM windows and peripheral access, W2-FLOW `Sys` terminators, everything that reaches `poke_irq`): a handler that mutated state returns `Ok` (or `Jump`/`Smc`) and lowers `ctx.room` through `event::due_room`; it never returns `Slow` after a write. A `Slow` is handed to `Core::step` as "the uop did not run" (`drive/chain.rs`), so a handler that returned it after mutating would run that instruction twice. The handler-level recipe of P8 catches it: a bailing handler must leave the core equal to the interpreter state before the instruction.

Events raised inside a block follow design section 7.3: a system-register write is a block terminator; a peripheral access re-checks `irq.due` after the access; the I7/I15 wrap latch is processed by `UF_CB` uops; `IDLE` runs through the interpreter body.

## 5. Invalidation

1. `uop_note_write` ORs the bitmap words a write touches into `uop_run.dirty` and, when the write hits a code page, sets `RF_SMC` and counts `smc`; `uop_note_all` marks everything. Both hooks use `Core::uop_run` only, never `UopHot`.
2. A store handler checks `uop_run.flags` once after the write; `RF_SMC` returns `Exit::Smc` after the store completed.
3. At the next block entry (`uop_chain`), `uop_revalidate_dirty` walks the blocks registered on the pages of the dirty bitmap words and recomputes `lower::decode::code_hash` over `block.pc..block.end_pc` (the function the translator stored). A block whose hash still matches is kept, so an overlay that reloads identical code and a data store into the page of a block cost a compare (P6). The others are dropped: tombstone, clear the `PcMap` entries, push `(pc, last_pc + 3)` to `dropped`, `epoch += 1`. The walk is in place and allocates nothing (P14); `drop_block` pushes into capacity `UopHot::install` reserved.
4. Dropped ranges run in the interpreter; `uop_maintain` (pass boundary, only when `uop_needs_maintenance()`) merges them, retranslates and installs them, and compacts the tombstones once there are at least 8 (ASSUMED) and more than live blocks: the arena, block list, pc maps, page lists and links are rebuilt and `epoch` moves. Retranslation keeps the stop pc. Every block installed after `enable_uop` (this path, the worker's answers and the ISA-mode branch of `uop_submit_dropped`) goes through `Core::uop_install_marked`, which marks the code pages of the parcels its `code_hash` covers (`pc..end_pc`). A dropped range is `(pc, last_pc + 3)` and may reach two parcels past the range `enable_uop` marked; without the marking a block there would sit on a page whose writes set no dirty bit and would never be revalidated.
5. Async path: `uop_submit_dropped` sends the merged ranges to the service (ISA-mode code and a full request ring stay with the caller: translated synchronously, respectively queued again), `uop_collect` installs the answers through `uop_install_done`, which recomputes `code_hash` on the live core and does not install a block whose code changed since the request (its range goes back to `dropped`). An empty answer means "not translatable": nothing is installed and nothing is retried. A block whose pc, `end_pc` and hash match a live block is a twin and is skipped. A block that holds the live stop pc as an interior uop (the worker's stop pc is fixed at spawn, and `uop_set_stop_pc` only repairs the blocks that exist when it runs) is not installed either (P7): its exact range `(pc, end_pc)` is translated on the live core with the live stop pc, once for both PEy modes, and nothing is queued, so the same block does not come back from the worker.
6. Known limitation: a range that retranslates to zero blocks leaves no block to drop later, so it stays interpreted until the next `enable_uop`. A dropped range is `last_pc + 3`, up to two parcels past the block, so a block retranslated again and again widens by up to two parcels per round (bounded by `max_block_steps`).

## 6. Cost model and report

`runner/uop_report/model.rs` turns counters into modeled host cycles. `steps_native` is a subset of `steps_uop` (native blocks are one kind of uop step; `UopStats` is frozen, so the contract is documented in `model.rs` and `tools/uop_gates.py`, which rejects a report with `steps_native > steps_uop`). All costs are ASSUMED until gate K0: dispatch 6, transition 38, `Interp` step 100, Tier 1 work 42 per step, native 38 per step all-in. Reference: ORG Core2, 6,280 steps, 8,478 uops, 520 blocks, 125 `Interp` steps gives about 340k cycles (Tier 1) and about 236k to 246k (Tier 2). Tests pin both totals.

`uop-report` (nord-tools) prints one JSON object, schema 1, hand-written, counts only:

`{"schema":1,"label":s,"window":"measured"|"all","replay":{checked, skipped},"stats":{passes, blocks_run, uops, steps_uop, steps_interp_uop, steps_core_step, steps_native, exits[6], guard_fail[9], smc, translations, poisoned, maintain_calls, shadow_checks, shadow_mismatch, shadow_skipped},"model":{tier1_cycles, tier2_cycles}[,"block_steps":{edges[11], counts[12], entries, steps, chained, empty}]}`

| Member | Meaning |
|---|---|
| `window` | `measured`: the counters are the measured passes only (`uop-report` resets after the warm-up; `lle_sharc_bench` subtracts the snapshot it took when the first measured pass started). `all`: every pass, warm-up included. |
| `replay` | passes of `uop-checked` replayed on the interpreter and found equal (`checked`) and passes it could not replay because the core has peripheral models (`skipped`). Separate from `stats.shadow_checks`, which stays the engine's own count of blocks checked. A run with `skipped > 0` has not shown exactness (`uop-report` exits 3). |
| `block_steps` | block-length histogram of design 11.3, present for `uop-report` only: steps run by each block entry, bucketed by `edges` (inclusive upper bounds; `counts` has one more, open bucket). `entries` are bucketed entries, `chained` entries that ran 2 or more linked blocks, `empty` entries that bailed before a step. `entries + empty + 2 * chained <= blocks_run`, equal when nothing chained. Edges are ASSUMED. A static table of the translated blocks needs a read-only accessor on `core::uop` (see the work package notes). |

`uop-report` runs `warmup + passes` passes on one thread (it can reset the counters between the phases); `lle_sharc_bench --uop-report FILE` runs the cores on their own threads and reports counters minus the warm-up snapshot. The report has no `block_steps` and no `replay` there.

The default binary arms the engine on a synthetic poll program (no firmware) and is the schema and wiring check; with firmware (`--image`, `--preset`) the same report comes from the real pass of that core.

## 7. Gates (`tools/uop_gates.py`)

Reads the report JSON only; exit 0 pass, 1 fail, 2 bad input. All thresholds are ASSUMED decision thresholds.

| Gate | Condition |
|---|---|
| G2 | uops per step <= 1.35; `Interp` plus `Core::step` steps <= 6% of steps (fast share >= 90%); `Slow` exits <= 20 per pass; `maintain_calls == 0` and `smc == 0` in steady passes (evaluated only on a report whose `window` is `measured` or absent; a `window: "all"` report fails both, as the warm-up is where translations and revalidations are expected) |
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

## 9a. Handler and service details (wave 2)

- Handler ABI: `handler(ctx, up)` gets `up = uops.as_ptr().add(i)`, a pointer into the block slice, never one made from a `&Uop` (that covers one element). The float family relies on it: after a raised arithmetic exception `run.rs::poked` returns `Jump` to the next uop's pc (`(*up.add(1)).pc`) unless `irq.due` is unchanged or the uop is `UF_LAST`/`UF_FIN`, so the interpreter prologue sees the interrupt at the step where it would have.
- Kind tables and operand layouts are in the module docs of `exec/h_float/mod.rs` (FGEN..FABSNEG), `exec/h_fixed/kinds.rs`, `exec/h_move/layout.rs` (register classes split at `SYS_UREG`, the same constant as `lower::cut::SYS_UREG`) and `exec/h_flow/mod.rs`. Each float kind carries a `VERIFIED` tag naming its differential test.
- DO_UNTIL (arithmetic loops): `Core::refresh_check` computes the check address with two walks (`walk_body`) and does not allocate; `the_check_address_equals_the_list_walk_on_random_bodies_and_does_not_allocate` compares it with the list walk on 400 random bodies, and `h_flow/allocs.rs` counts 0 allocations for a steady call of every flow handler, DO_UNTIL included.
- `TransReq::capture` copies `lo..hi` plus `TAIL_WORDS` (8) of the code window; it returns `None` for an empty range, a range over `CODE_SPAN`, or ISA-mode code. The service (`Service::new/spawn/spawn_with/submit/poll/pump`) has a request ring of `CAP = 16`.
- `exact_diff` hidden extension: the f40 registers compare `effective_f40` (`lo` counts only while `tag == r`) plus `raw_ext` (`lo`, and `tag` if `lo != 0`), so a stale `lo` under a different tag is seen when it can matter.
- Revalidation keeps a block whose `code_hash` is unchanged (P6); a dirty bit walks exactly the 64 pages of its bitmap word (`PAGES_PER_WORD == u64::BITS`, test `a_dirty_bit_walks_exactly_the_64_pages_of_its_bitmap_word`).

## 10. Provenance and open items

- Constants in the cost model, the page size (256 short words), the 8 translation-validation states and every gate threshold are `ASSUMED`; nothing here is `VERIFIED` against hardware beyond the layout tests (`uop_is_16_bytes`, `code_pages_cover_short_window`, handler-table order, `CTX_*` offsets).
- Sticky bail of the fixed uops (`exec/h_fixed/mod.rs`, measured by `h_fixed/diff_sticky.rs`): a fixed uop whose effect sets an arithmetic sticky bit answers `Slow` (the uop did not run) and the interpreter runs it. Latching is not a no-op even when STKY already holds the bit: `note_arith_exception_in` re-raises the vector and pokes `irq.due` to 0 on every event (PRM A-21), which `exact_diff` compares. Share of bails: 3 of 480 cases (0.6%) on the census pool (T2a, T2b, T6n; 2 seeds, both PEy modes), 24 of 840 (2.9%) on edge-biased random ALU programs (an upper bound: the rig draws operands from the edges). The handler is kept as is.
- `AF` (all-float flag) in `h_fixed/eff.rs`: the position and "float sets it" are `VERIFIED` (`simd_measured.rs`); "a fixed ALU op clears it" is `ASSUMED` (an ASTAT measurement after a fixed op that follows a float op would confirm it).
- Tier 2 sources under `core/uop/native/` are stubs (`enter` returns `Slow`, emitters emit nothing). `UopNative` degrades to Tier 1.
- Real-time on ORG Core2 stays unlikely (design 9.2: x1.14 raw); the hybrid verdict in `sharc-lle-realtime.md` stands until K3 passes.
