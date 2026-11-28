# SHARC uop engine (D260)

Status: **skeleton (W0)**. The module tree, types, driver, invalidation, verification harness and gate tooling exist and are tested; every lowering family is a stub that returns "not handled", so each instruction becomes an `Interp` uop and the engine is bit-exact by construction. Design, scores and work packages: [sharc-compiler-design](../research/sharc-compiler-design.md). Decision: [D260](../DECISIONS.md) (proposed).

Code: `crates/nord-sharc/src/core/uop/`, `runner/uop_pass/`, `runner/pass_sched/`, `runner/uop_report/`, `crates/nord-tools/src/bin/uop_report.rs` (+ `uop_report/`), `crates/nord-tools/src/bin/lle_sharc_bench.rs`, `tools/uop_gates.py`.

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
| `uop_try(limit) -> bool` | one block from `pc`: entry guard, event room, run, counters. False means the caller runs `Core::step`. |
| `run_uop(n)` | `Core::run` with the hook; fault returned like `run`. |
| `uop_begin_pass(idle_pc)`, `uop_take_fault`, `uop_set_stop_pc` | pass runner support. |
| `uop_note_write(first_short, count)`, `uop_note_all()` | called from `invalidate_sram_cache` and `invalidate_code_cache`; early return while `uop_run.active` is false. |
| `uop_revalidate_dirty`, `uop_maintain`, `uop_needs_maintenance`, `uop_dirty` | invalidation (section 5). |
| `lean_pass_uop`, `lean_pass_uop_checked` | pass loops (`runner/uop_pass/`: `mod.rs` the engine loop, `checked.rs` the interpreter replay with `shadowed_with`, `diff_of`, `checked_with`; `*_entries` forms report each block entry to a callback for the histogram). |
| `run_ticks`, `run_ticks_with` | threaded pass scheduler (`runner/pass_sched/`). A panic in a core's pass stops every core at once (shared abort flag) and returns `Err("core NAME panicked in pass N: ...")`; the bench exits non-zero. `CoreSeries::uop_at_warmup` is the counter snapshot taken when the first measured pass starts. |
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

Events raised inside a block follow design section 7.3: a system-register write is a block terminator; a peripheral access re-checks `irq.due` after the access; the I7/I15 wrap latch is processed by `UF_CB` uops; `IDLE` runs through the interpreter body.

## 5. Invalidation

1. `uop_note_write` ORs the bitmap words a write touches into `uop_run.dirty` and, when the write hits a code page, sets `RF_SMC` and counts `smc`; `uop_note_all` marks everything. Both hooks use `Core::uop_run` only, never `UopHot`.
2. A store handler checks `uop_run.flags` once after the write; `RF_SMC` returns `Exit::Smc` after the store completed.
3. At the next `uop_try`, `uop_revalidate_dirty` handles the blocks on dirty pages. Skeleton behaviour: drop every block on a dirty page (tombstone, clear the `PcMap` entries of the block's own mode, push the range to `dropped`, `epoch += 1`). W2-DRIVE adds the `code_hash` compare (`lower::decode::code_hash` over `block.pc..block.end_pc`, the function the translator stored) that keeps an overlay that reloads identical code.
4. Dropped ranges run in the interpreter; `uop_maintain` retranslates them after the pass.

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

## 10. Provenance and open items

- Constants in the cost model, the page size (256 short words), the 8 translation-validation states and every gate threshold are `ASSUMED`; nothing here is `VERIFIED` against hardware beyond the layout tests (`uop_is_16_bytes`, `code_pages_cover_short_window`, handler-table order, `CTX_*` offsets).
- Tier 2 sources under `core/uop/native/` are stubs (`enter` returns `Slow`, emitters emit nothing). `UopNative` degrades to Tier 1.
- Real-time on ORG Core2 stays unlikely (design 9.2: x1.14 raw); the hybrid verdict in `sharc-lle-realtime.md` stands until K3 passes.
