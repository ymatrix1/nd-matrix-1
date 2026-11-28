# ARM0 execution tiers: superblock traces in safe Rust, an optional native tier over the same IR, and exact idle skipping first

Checked: 2026-11-28
Sources: `origin/main` 80748ba5 (`crates/nord-arm/src/{lib,mem}.rs`, `cpu/{mod,predecode,arm_cached,arm_dp,arm_mem,alu,bus}.rs`, `machine/{mod,fastrun,idle,probe,sysbus,hooks}.rs`, `Cargo.toml`, `.github/workflows/ci.yml`, `crates/nord-sharc/Cargo.toml`, `Cargo.lock`); [arm0-rust-runtime](arm0-rust-runtime.md) on main and its revision in PR 689 (head d97b8207: block counters, tail at four budgets, producer thread); PR 683 `crates/nord-lle/src/arm.rs` (head 2e9f82d5); `gh pr list` (PRs 683, 687, 689 touch `nord-arm`); D20, D128, D129, D245, D246, D249, D250, D259 in `docs/DECISIONS.md`; [sharc-lle-realtime](sharc-lle-realtime.md); the two ARM proposals of this workflow (ARM-X native regions, ARM-SB superblocks), the three judge reports, and the SHARC ceiling spike (host-load note only). Nothing was built or run for this note: no cargo, no benchmark. Every speed figure below is a model.
Question: Which execution design takes ARM0 (ARM926EJ-S, OS v1.66) from the measured 17 thread cycles per guest instruction to a rate that keeps the ARM thread well under one host core in steady state and every heavy tick under 1 ms, stays bit-exact against `Cpu::step`, ships on Windows, macOS and Linux, and can be built by about 20 parallel agents without touching the files that `arm0-rt` is editing?

Tags: VERIFIED = read in source or CI config at the commit above, or arithmetic on such values. EMPIRICAL = measured in a repository note (cited). ASSUMED = estimate or chosen constant, with what would confirm it.

## Answer

* **Base design: ARM-SB** (judge score 91.5 against 85 for ARM-X). The spine is a tier of trace superblocks over the existing predecode cache: a flat micro-op IR (`Uop`), trace formation with chained exits, an exact budget rule, page-granular invalidation, and exact idle skipping driven by loop detection. It is safe Rust, adds no dependency, and every unlowered instruction runs through the interpreter's own handler.
* **Grafted from ARM-X:** a native tier (Cranelift, feature `sb-native`, default off) that compiles the same `Sb` IR instead of a separate region IR; word-snapshot revalidation of stale pages; counters-first and probe back-off as the first deliverable; lockstep with bisection to one instruction; mutation testing; `mem/raw.rs` as the only Mem change. The per-judge picks favoured ARM-X (all three judges picked A-A for `arm`); this note follows the score ranking as instructed and absorbs ARM-X's performance path so the disagreement costs nothing (risk R1).
* **The first lever is not the instruction rate.** Idle and key ticks execute only 3.3 to 3.4 k instructions (about 18 us at 5.3 ns) but cost 63 to 72 us (VERIFIED arithmetic on EMPIRICAL data: 45 to 48 us per tick unexplained). At the 24 kHz framing the engine uses (PR 683: `TICK_INSNS = 200_000` per 24 kHz tick), a per-tick cost of that size is 1.1 cores. Idle v2 (exact, trace-independent first stage) ships before any tier is switched on.
* **Modelled speed (ASSUMED, section 10), cycles per guest instruction against the measured 17:** tier 2 (safe Rust) best 9.0, likely 12.1, worst 15.2, that is 1.9x, 1.4x, 1.1x; tier 3 (native) best 3.6, likely 5.1, worst 9.2, that is 4.7x, 3.3x, 1.9x. The ARM-SB proposal said 2.3x, 1.6x, 1.3x; the difference is three rows both proposals omitted (plain `Cpu::step` instructions 1.9 %, indirect-dispatch mispredicts, register-in-memory dependency chains).
* **Consequence:** tier 2 alone does not meet "every heavy tick under 1 ms" at 300 k or 456 k instructions per ms and is marginal at 200 k; tier 3 does in the likely case. Steady state at 24 kHz: 0.43 core today, 0.31 tier 2, 0.13 tier 3 (model). ARM is small next to the SHARC side (8.1 budget periods measured for three cores, 17.3 projected for eight, against 16 cores), so ARM decides the hybrid verdict only through gates G4 and G5.
* **Build discipline (answers the "one swoop, benchmark later" question for ARM):** yes. Exactness tests are cheap and deterministic and run per work package; wall-clock runs happen only at gates G0, G2 and G3, after counters. No benchmark between commits. The risk of skipping per-commit exactness tests is not speed, it is silent divergence, so those stay.

## 1. Facts the design stands on

| Fact | Value | Tag | Source |
|---|---|---|---|
| `nord-arm` dependencies, unsafe | none, none | VERIFIED | `Cargo.toml` `[dependencies]` empty; `lib.rs` 12 lines |
| Decoded entry | `Entry { insn: u32, mask: u16, op: u8, run: u8 }`, 1,024 per 4 KiB page, `decode_arm`, `ends_block`, `COND_MASK`, `MASK_ALWAYS` are `pub` | VERIFIED | `cpu/predecode.rs` |
| Interpreter helpers reachable from a new module | `exec_decoded`, `exec_pure`, `dp_exec`, `ldst_exec`, `arm_ldm`, `arm_ldst_half`, `arm_mul`, `branch`, `set_nz` are `pub(crate)`; `alu::{shift, add_c, arm_imm}` are `pub` | VERIFIED | `cpu/*.rs` |
| `drop_stale_code`, `run_cached`, `step_cached` | `pub(super)` of `machine::fastrun`, so callable from any descendant of `machine` (not from the crate root) | VERIFIED | `machine/fastrun.rs` |
| Memory page entry | `table: Vec<u32>` of 1<<20; bits 31..28 = `HOOK_R, HOOK_W, HOOK_X, HOOK_C`, bits 27..0 = arena page index + 1 (0 = unmapped); `arena: Vec<u8>` grows by `resize` (pointer moves) | VERIFIED | `mem.rs` |
| Store to a `HOOK_C` page | slow path: `mark_stale` clears `HOOK_C` and pushes the page on `stale_code`; `entry`, `*_e`, `flags`, `is_mapped`, `mark_code` are `pub` | VERIFIED | `mem.rs` |
| Only production pop of `stale_code` | `Machine::drop_stale_code` (called by `run`, `run_cached`, `step_cached`) | VERIFIED by reading; a grep is obligation W4-SEAM | `machine/fastrun.rs` |
| `Machine::run` contract | `limit = start + max` absolute; hook check per page; `run_cached` only when predecode, unhooked, ARM state, `pc & 3 == 0` | VERIFIED | `machine/mod.rs` |
| Heavy stretch | about 6.5 M instructions, 75 % of them in a 2 KB soft-float-like routine; ticks at budget 150/200/300/456 k: 44/33/21/14 | EMPIRICAL | PR 689 doc, section "Tail follow-up 2" |
| Cached-loop counters (13.7 M executed) | blocks 1.71 M (7.8 insns/block), B/BL enders 1.32 M, other enders 0.38 M, page follows 304 k, stale exits 3.8 k, plain steps 257 k (1.9 %), hook runs 1,516 | EMPIRICAL | PR 689 doc |
| Per-instruction cost | 185 to 198 M insns/s = 5.3 ns = 16.6 to 17.8 thread cycles (least disturbed); 124 to 129 M/s = 25 to 26 cycles (loaded median); synthetic ceilings 375 / 230 / 205 M/s | EMPIRICAL | PR 689 doc, section 2 |
| Steady ticks | idle 3,384 insns/tick, 0.063 to 0.066 ms mean; key phases 3,288 to 3,448 insns, 0.063 to 0.072 ms | EMPIRICAL | arm0-rust-runtime (main) |
| Fixed cost of an idle tick | 63 to 66 us minus 3,384 x 5.3 ns = 45 to 48 us | VERIFIED arithmetic | previous two rows |
| Old interpreter rate | 93.5 M insns/s = 10.7 ns = about 36 cycles | EMPIRICAL | arm0-rust-runtime (main) |
| Instructions per ms | 200 k used; real value bounded by 300 k to 456 k (clock x IPC<=1), not measured | ASSUMED | arm0-rust-runtime, "Instruction budget per ms" |
| Engine framing | `nord-lle` runs `TICK_INSNS = 200_000` per 24 kHz tick: 4.8 G virtual insns/s against an AM1705 bound of 0.3 to 0.456 G | VERIFIED (constant), tick-scale conflict open | PR 683 `arm.rs` |
| CI | ubuntu-24.04, windows-latest, macos-latest; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo test --workspace --all-targets --all-features` | VERIFIED | `ci.yml` |
| Cranelift already in the workspace | `cranelift-{codegen,frontend,jit,module,native}` 0.116 (locked 0.116.1) behind `nord-sharc` feature `jit` | VERIFIED | `crates/nord-sharc/Cargo.toml`, `Cargo.lock` |
| Open PRs touching `nord-arm` | 689 (`machine/{consumer,fastrun,mod,producer,runstats}.rs`, adds `Machine.stats`), 687 (`machine/{boot,dsp_return,layout,mod,panel,system,uart}.rs`), 683 (`machine/dsp_return.rs`) | VERIFIED | `gh pr list` |
| Files over 400 lines (TD-22) | `cpu/predecode.rs` 440, `mem.rs` 453, `machine/fastrun.rs` 407 | VERIFIED | line counts |

## 2. Decision

### 2.1 Tier ladder

| Tier | What | Code | Default | Role |
|---|---|---|---|---|
| T0 | `Cpu::step` per instruction (`Machine.predecode = false`) | exists | off path kept | oracle |
| T1 | predecode cache, `run_cached` (blocks of 7.8 insns) | exists, unchanged | on | baseline and cold code |
| T2 | trace superblocks, flat `Uop` IR, in-loop executor, chained exits, idle v2 | new, safe Rust, `machine/sb/` | off (`SbConfig.enabled = false`) | portable fast tier, oracle for T3, fallback when T3 is unavailable |
| T3 | native code (Cranelift) compiled from the same `Sb` | new, feature `sb-native`, `machine/sb/native/` | off (`SbConfig.native = false`) | performance tier |

Every tier works in place on the same `Cpu` and `SysBus`: switching tier converts nothing, and at every exit the state equals the interpreter's state after the same number of instructions.

### 2.2 Why the ARM-SB spine, and the strongest objections answered

| Objection (from) | Answer |
|---|---|
| Safe Rust has a floor of about 5 cycles per instruction (register file in memory, one shared indirect jump), so superblocks cannot give "well under one core" at 300 k+ or on a loaded host (ARM-X, judges) | Accepted and priced: tier 2 is 1.4x likely, not 1.6x. It is not asked to meet that goal alone. T3 compiles the same IR to native code and does. Tier 2 is still required: it is the portable tier (macOS hardened runtime, DAW sandboxes, no-JIT builds), the executor that handles `Generic` uops, and the in-process oracle that makes T3 testable without the interpreter's speed. |
| ARM-SB's idle claim rests on an inference (the steady tick is the probe) (ARM-X) | Idle v2 stage 1 does not depend on it: comparing the state only when `pc` equals the probe start is exactly equivalent to comparing after every instruction (`same_state` includes `r[15]`), so it cannot change a result. Stage 2 (trace loop-watch) is gated by counters (G0, rule K5). |
| ARM-SB omits the plain `Cpu::step` instructions (1.9 %) and indirect-dispatch mispredicts (judges) | Added as cost rows 13 to 15 (section 10). They move the tier-2 likely figure from 10.5 to 12.1 cycles. |
| ARM-X puts unsafe code, a dependency and executable memory into a crate that has none (ARM-SB, judges) | Only in feature `sb-native`, default off, unsafe confined to `native/rt.rs`, runtime check `native::available()` with automatic fallback to T2, never persisted (D129). macOS hardened-runtime and sandbox failures degrade to T2, not to an error. |
| ARM-X's region former with a CFG, 24 basic blocks and `br_table` heads is complex (ARM-SB) | Dropped. A superblock is single-entry with side exits and at most one loop-back to its head, so the T3 lowering is one linear pass with branch-over blocks. |
| ARM-X tolerates 5 % of dynamic instructions leaving native code at 150+ cycles each (self-inconsistent: 7.5 cycles per instruction) | Gate G3 caps exits at 1 % of dynamic instructions (1.5 cycles). Page-granular device flags are the known cause; the helper-call lever is documented as follow-up F5, not built. |
| A page-granular store invalidation thrashes when data shares a code page (SRAM handlers) (both) | Revalidation by word snapshot (graft) keeps unchanged traces and re-marks `HOOK_C`; pages whose code words really changed more than `volatile_after` times become tier-1 only. |
| One shared dispatcher round trip per region (ARM-X rows 20 cycles x 83) | Traces average about 25 guest instructions per entry (ASSUMED from 7.8 per block and followed B/BL), so boundaries cost 40 per 1,000 instead of 83. |

### 2.3 Grafts absorbed

| # | From | Graft | Lands in |
|---|---|---|---|
| 1 | ARM-X | native tier over the trace IR (not a second IR) | `sb/native/*`, WPs W1-NCORE, W2-NDP, W2-NMEM, W3-NINT |
| 2 | ARM-X | page-table-direct memory fast path with side exits to the interpreter instead of helper calls (masks `0x8FFF_FFFF`, `0x5FFF_FFFF`, `0x0FFF_FFFF`; arena pointer refreshed before every entry) | `native/lower_mem.rs`, `native/abi.rs` |
| 3 | ARM-X | stale revalidation by word snapshot instead of dropping every trace of the page (plus ARM-SB's volatile-page rule) | `stale.rs` |
| 4 | ARM-X, judges | counters first (probe share, fixed tick cost, plain-step share, executed insns per tick at both framings), then probe back-off, before any tier | W1-CNT, W1-IDLE, gate G0 |
| 5 | ARM-X | lockstep over real firmware with automatic bisection to one instruction; mutation testing of the lowering; build-time validation | W3-VERIF (`shadow.rs`, `mutants.rs`, `validate.rs`) |
| 6 | ARM-X | raw table and arena pointers through a child module `mem/raw.rs`, so `mem.rs` gains one declaration | W1-NCORE |
| 7 | ARM-X | tail-call chaining (`return_call_indirect`, Tail convention) as stage 2 of T3, behind a three-OS spike with dispatcher-return as fallback | W1-NCORE spike, W3-NINT |
| 8 | ARM-X | `Machine: Send` compile-time assertion and a documented single-owner `unsafe impl Send` for the JIT module | `native/rt.rs` |
| 9 | judges | plain-step instructions, dispatch mispredicts and dependency chains as cost rows; native exit cap tightened to 1 % | section 10, G3 |
| 10 | judges | one DECISIONS and LEGAL wording shared with the SHARC track for in-memory runtime translation of the user's own image (nothing persisted, no firmware bytes, no hot addresses in the repo) | W4-DOC |
| 11 | ARM-X | stop rule K0: if counters show nothing to gain, tiers stay built but off | gate G0 |
| 12 | ARM-SB (kept) | loop-watch idle skipping, exact `until`/`stop` page-list test, build caps per `run()` call, empty `Clone`, 4-stale volatile rule, three-way fuzz with budgets expiring at every uop position | sections 4 to 8 |

Rejected: ARM-X's region former and per-region CFG; a separate `Insn`/`Op` IR; whole-module reset as the only reclaim policy (it leaks pages on Windows, risk R9); a software TLB in T2 (T2 reuses `SysBus` accessors; a TLB is follow-up F4).

## 3. Data structures and state layout

All indices, no raw pointers outside `native/`. Code lives under `crates/nord-arm/src/machine/sb/` (a child of `machine`, so it sees `pub(super)` items of `fastrun`; one declaration `pub mod sb;` is the only edit to `machine/mod.rs` until the seam; the skeleton also made `same_state` and `Machine::probe_loop` in `machine/idle.rs` `pub(super)`, a visibility change with no logic).

### 3.1 `Uop` (16 bytes, `repr(C)`)

`kind: u8, cond: u8, rd: u8, rn: u8, rm: u8, rs: u8, sh: u8, n: u8, imm: u32, pc: u32`. `n` = guest instructions the uop covers (1, or 2 for fused). `pc` = guest address of the first covered instruction (written to `bus.pc` before a memory access, and the fault pc). Lowerers emit unconditional uops; `trace.rs` wraps a conditional instruction in a `PRED` and `pred.rs` merges runs. Invariant I2: no uop other than `GENERIC*` and `EXIT*` reads or writes `r15`; constants that depend on the pc (`add rd,pc,#x`, `bl` link, literal loads) are folded at build time, and every other use of `r15` is `GENERIC`. The rule is enforced twice: each lowerer returns `Lowered::Generic` for it, and `decode::lower_or_generic` overrules any lowering whose uops still name `r15` (`decode::touches_r15`, which also catches `r15` in an `STM` list and `rd >= 14` of `ldrd`/`strd`), so a lowering bug cannot break I2. `n` is bookkeeping for validators and the fuser, never an input of an executor (counts come from `Exit::done`, I3): an instruction that lowers to several uops puts `n = 1` on the first and `n = 0` on the others, a `PRED` has `n = 0`, and summing `n` over the uops of a path in program order (a `PRED` body included, a failing condition still retires) gives the `done` of the exit that path leaves through. The early exit of a memory uop or `GENERIC` (I6) is stored in its `cond` field, that of every control uop in `rs` (`Uop::exit_idx`); the trace builder allocates it. Specialised non-S uops keep `sh = 0`.

| Kind ids | rd | rn | rm | rs | sh | imm |
|---|---|---|---|---|---|---|
| DP generic `0..=95`, id = form*32 + opcode*2 + s (form 0 immediate, 1 register shifted by immediate, 2 by register) | dest | first operand | Rm (forms 1, 2) | Rs (form 2) | form 1: `type<<5 \| amount (0..31)`, special encodings (LSR/ASR #0 = 32, ROR #0 = RRX) resolved by `alu::shift(.., imm = true)`; form 2: `type<<5`; form 0: bit0 = 1 when rot == 0 (shifter carry = C) | form 0: rotated immediate |
| Specialised DP `96..=107` (`MOV_IMM, MOV_REG, MVN_IMM, ADD_IMM, SUB_IMM, ADD_REG, SUB_REG, CMP_IMM, CMP_REG, TST_IMM, TST_REG, ORR_IMM`), same semantics as the generic id they replace, non-S unless the name says compare/test | dest | first | Rm | | | immediate |
| Memory word/byte `112..=119` (`LDR_IMM, LDRB_IMM, STR_IMM, STRB_IMM, LDR_REG, LDRB_REG, STR_REG, STRB_REG`) | Rd | Rn | Rm (reg offset) | flags: b0 U (add), b1 P (pre), b2 WB (`!P \|\| W`) | reg offset: `type<<5 \| amount` | 12-bit offset |
| Half and double `120..=125` (`LDRH, LDRSH, LDRSB, STRH, LDRD, STRD`) | Rd | Rn | Rm | flags + b3 REG | | 8-bit offset |
| `LDM`, `STM` `126, 127` (S bit and empty list stay Generic) | | Rn | | flags | popcount | register list |
| `LDR_ABS` 128 (literal on an unfolded page) | Rd | | | | | absolute address |
| `PRED` 144 (`cond` field = condition 0..=13; skips the next `imm` uops when it fails) | | | | | | skip count |
| `GUARD` 145 (leave through exit `rs` when condition `cond` holds; the condition is the `cond` field, `imm` is 0) | | | | exit index | | |
| `GUARD_REG_EQ` 146 (take exit when `r[rn] != imm`, used for inlined returns) | | Rn | | exit index | | expected value |
| `LOOP_BACK` 147 (re-enter at uop 0 while `left >= n_guest`, else take exit) | | | | exit index | | |
| `EXIT` 148, `EXIT_REG` 149 (BX: interwork), `EXIT_REG_ARM` 150 (`mov pc,rm`: no interwork) | | Rn | | exit index | | |
| `GENERIC` 151 (run the instruction in `imm` through `exec_decoded`), `GENERIC_END` 152 (same, then a `Dynamic` exit) | | | | exit index (END) | | instruction word |
| `MRS` 153, `MSR_C` 154 (`cpsr_c` immediate or register, T never written, `mode_changes` counted) | Rd | Rm | | b0 = SPSR (MRS) or immediate form | | rotated immediate |
| Fused `160..=164` (`CMP_GUARD_I, CMP_GUARD_R, TST_GUARD_I, MOV_PAIR, LDR_PAIR`), each defined as its unfused expansion; `n = 2`, the second instruction at `pc + 4` | layouts in `uop/fused.rs`: `fuse_pair(a, b)` is the only constructor (W2-PEEP decides when to call it), `expand(u)` the definition (executors and native lowerers call it or are tested against it); `bus.pc` is set per access as in the expansion | | | | | |

All kind ids, including fused ones, are fixed in S0 so no later work package edits `uop.rs`. The executor treats an unknown id as `unreachable!` and a test enumerates every id below `k::COUNT` against an executor arm.

### 3.2 `Exit`, `Sb`

```
Exit { kind: Direct | Indirect | IndirectArm | Dynamic | Fault | Stale | LoopBack | Watch,
       reg: u8, done: u16, target: u32, last_pc: u32,
       link: SbId, ic_pc: [u32; 2], ic_id: [SbId; 2], hits: u32 }
Sb   { entry: u32, pages: [u32; 4], n_pages: u8, n_guest: u16,
       uops: Box<[Uop]>, exits: Box<[Exit]>, words: Box<[(u32, u32)]>,
       live: bool, enters: u32, native: SbId }
```

`done` = guest instructions executed from the start of the current iteration through the last instruction retired on this path (static); `last_pc` = its address (becomes `bus.pc`); `Direct.target` is a constant; `Indirect` reads `r[reg]` (bit 0 sets `thumb`, target `& !1`, else `& !3`); `IndirectArm` reads `r[reg] & !3`; `Dynamic` takes `cpu.r[15]` after a Generic ender; `Fault` is reserved and never emitted: a faulting `GENERIC_END` (the only uop that can fault, since every block ender goes through it or `lower_flow`) leaves through its own `Dynamic` exit with the instruction counted, the fault stays in `cpu.fault`, and the driver takes it after every run whatever the exit kind (as `run_cached` does); `Stale` is the early exit after a store that left `stale_code` non-empty (memory uops and `GENERIC`); `LoopBack` is the budget-exhausted exit of a loop; `Watch` returns to the driver at a dispatch point for idle v2. `n_guest` = the maximum guest instructions on any path through the trace including one full loop iteration. `words` = `(address, word)` for every guest word the trace was built from, folded literals included (basis of revalidation).

### 3.3 Cache, hotness, state

* `SbCache` (`cache.rs`): arena `Vec<Sb>`; `at: per page Box<[u32; 1024]>` (word to `SbId + 1`); `by_page: Vec<Vec<SbId>>`; `modified: per page u8` (volatile rule); `uops_total` with cap `uop_cap` (4 M uops = 64 MB, flush all on overflow); `epoch`. `Clone` of the owning `SbState` yields an empty cache (so benches that clone a `Machine` stay valid).
* `HotTable` (`hot.rs`): per page `Box<[u16; 1024]>` of dispatch counts plus a failure counter; `bump(pc) -> u16`, `reset`, `mark_failed`, `failed`.
* `SbState` (`mod.rs`): `cfg, cache, hot, stats, watch, backend: Option<Box<dyn Backend + Send>>`, plus `placeholder: bool` and `pending: Vec<u32>`. Before the seam it lives next to the `Machine` (tests, tools); after the seam it is the field `Machine.sb`. The driver takes the real state out with `SbState::take_real(&mut m.sb)`, which leaves a placeholder; `on_stale` on a placeholder only records the page (deduplicated) in `pending`, `absorb_pending` forwards the records to the real state, and `put_back` ends the borrow. A stale page therefore reaches the real state even when a nested `Machine::run` (`call_preserving`, `inject_irq`, a hook) pops `stale_code` while the driver holds it.
* Constants: `hot = 24`, `slice = 32`, `max_uops = 128`, `max_guards = 6`, `max_pages = 4`, `inline_depth = 2`, `builds_per_run = 4`, `volatile_after = 4`: all ASSUMED (chosen from the 7.8-instruction block length and the 2 KB hot routine; the first counters run fixes them; `NS4_SB_*` env overrides).

### 3.4 Invariants (every work package obeys these)

* I1 A trace depends only on code words and folded literal words, never on register or flag values; there is no speculation. `call_preserving` restoring the `Cpu` therefore cannot invalidate a trace.
* I2 as above (r15).
* I3 `done`, `last_pc`, `n_guest` are static; `cpu.insns` changes only at exits.
* I4 The flag nibble `f = n<<3 | z<<2 | c<<1 | v` (the layout of `Cpu::nzcv`) is authoritative inside the executor; `cpu.n/z/c/v` are written at every exit and around `GENERIC`; `q`, `thumb`, `irq_off`, `fiq_off`, `mode`, `spsr`, `cp15_ctrl` change only inside `GENERIC` and `MSR_C`.
* I5 Memory effects occur in guest order through the `SysBus` accessors with `bus.pc` set to the accessing instruction, so device models, the poll breaker, probing and auto-mapping behave exactly as in T1.
* I6 A store that leaves `stale_code` non-empty ends the trace before the next uop.
* I7 A trace is entered only when `left >= n_guest` and none of its pages holds `until` or `stop`.
* I8 Anything not lowered is `GENERIC`, executed by the interpreter's `exec_decoded` (coverage grows per family; a missing lowering costs speed, never exactness).
* I9 The tier never maps a page: building reads only mapped, unhooked pages.
* I10 T3 native code calls no Rust function and exits to the driver for anything outside its fast path.
* I11 No stale page is lost: pages that go stale while the real `SbState` is taken out are recorded by the placeholder and forwarded before the next drain. The driver drains after every trace run whatever the exit kind, so a chain never continues into a trace that just died.
* Real-time rule (project `dsp-and-realtime.md`): trace building, compilation, revalidation and every `HashMap` or `Vec` growth of the cache happen in the dispatch path of `Machine::run`, which on the audio thread runs ahead of `process_block`; the engine's `process_block` and `render_frame` never call into this module. A page going stale is rare (program load, SRAM overlay); the only allocation it causes outside the cache is the first push to `SbState::pending`.

## 4. Execution

### 4.1 Driver (`run.rs`): `run_fast` replaces the `run_cached` call

```
run_fast(m, sb, limit, until, stop):                     // same contract as run_cached: runs >= 1 instruction or returns
  drain_stale(m, sb)                                     // forward stale_code to sb.on_stale, then m.drop_stale_code()
  builds = 0
  loop:
    pc = cpu.r[15]; left = limit - cpu.insns
    if left == 0 or pc&3 != 0 or cpu.thumb or pc == until or pc == stop or flags(pc) has HOOK_X: return None
    if id = cache.lookup(pc) and sb[id].n_guest <= left and no page of sb[id] holds until/stop:
        run the chain: out = backend.run(id) or exec::run_sb(...)      // T3 if compiled, else T2
          apply exit: cpu.insns += done_total; cpu.r[15] = target; bus.pc = last_pc; unpack flags
          after ANY exit kind: if cpu.fault.take() is Some(f) -> return f; drain_stale(m, sb)   // a dead successor is never chained into
          EXIT_SLOW (native only: nothing ran, nothing applied) -> run the same trace with exec::run_sb
          EXIT_STEPPED (native left to the interpreter and stepped one instruction) -> re-dispatch at the target, never link
          Stale -> break (re-dispatch);  Watch -> sb.watch.hit = true; return None
          Direct/Indirect/IndirectArm/Dynamic: patch exit.link or ic, bump exit.hits; next = linked/looked-up trace;
             continue the chain only if next is live, fits the budget, passes the guard test and the aligned/ARM/unhooked test
        continue
    if hot.bump(pc) >= cfg.hot and builds < cfg.builds_per_run and not hot.failed(pc):
        sb = trace::build(...); on Some: mark_code(pages), cache.insert, optional backend.compile, optional validate; builds += 1; continue
        on None: hot.mark_failed(pc)
    r = m.run_cached(min(limit, cpu.insns + cfg.slice), until, stop)      // tier-1 stretch, unchanged code
    drain_stale(m, sb); if r is Some(fault): return r
```

Hotness is sampled at dispatch points, which are tier-1 slice boundaries and trace exits. A hot loop of period P is sampled at pcs that are multiples of `slice` modulo P, so every loop is found; a trace may start mid-block, which is harmless because traces depend only on code words (I1). Taken-branch profiles are not collected: static layout (backward taken, forward not taken), and a guard exit whose `hits` exceed half the trace's `enters` after 64 entries triggers one rebuild with the prediction flipped; other hot side exits grow side traces through the same `hot.bump` on their target.

`run_sb_full(m, sb, max, until, stop_pc)` is a copy of `Machine::run`'s outer loop (hooks, `until`/`stop`, budget, plain step) that calls `run_fast` instead of `run_cached`. It is the entry for tests and tools before the seam and stays as the equivalence oracle afterwards.

### 4.2 Trace formation (`trace.rs`, `lower_*.rs`)

Start at a hot pc. Per instruction: `decode_arm`, then `lower_flow` for control flow, `lower_mem`, `lower_dp`, else `GENERIC`. Follow unconditional `B` and `BL` (`BL` emits `MOV_IMM r14, pc+4`; inline depth up to 2; the callee's `bx lr` at depth > 0 becomes `GUARD_REG_EQ r14 == return pc` and the trace continues), follow the predicted side of `Bcc` (the other side is a `GUARD` exit), end at an indirect branch (`EXIT_REG`, `EXIT_REG_ARM`), at a Generic ender (`GENERIC_END`), at an unmapped or `HOOK_X` page, or at a cap (128 uops, 6 guards, 4 pages). Returning to the start pc emits `LOOP_BACK`; returning to any other pc already in the trace ends with an `EXIT` (no inner loops). Conditional `BL` is Generic. Pc-relative loads fold to `MOV_IMM` when the literal's page is mapped, unhooked and added to the trace's page list. HALF-class forms the interpreter can fault on (odd `Rd` for `ldrd`/`strd`, `r14` pairs) stay Generic.

### 4.3 Executor (`exec.rs`, `exec_alu.rs`, `exec_mem.rs`, `exec_flow.rs`)

One `loop { match uop.kind }` with `regs = &mut cpu.r`, the flag nibble `f` and the index `i` in locals. Data-processing arms are monomorphised per `(OP, S, FORM)` const generics like `h_dp`; semantics are those of `Cpu::dp_exec` (logic ops keep V and take the shifter carry, arithmetic ops use `alu::add_c`, `sbc/rsc/adc` use the pre-instruction C). Memory arms are generic over `SbBus` and call the same `read32/write32/...` as the handlers (word loads rotate by `8*(a&3)`, stores use `a & !3`, halfword `a & !1`, write-back is skipped on a load with `rn == rd`). `PRED` skips the next `skip` uops when the condition fails. `GENERIC` (a non-ender: it cannot fault or jump) writes the nibble to `cpu`, calls `exec_pure`, reloads the nibble and leaves through its early exit when a store made code stale; `GENERIC_END` calls `exec_decoded`, reloads the nibble and leaves through its `Dynamic` exit, a fault staying in `cpu.fault`. After every store, `bus.stale()` ends the trace with a `Stale` exit. `LOOP_BACK` adds the iteration length to a local accumulator and re-enters while `left >= acc + n_guest`.

### 4.4 Budget

Entry requires `left >= n_guest` (I7), so no trace stops mid-way; the last stretch of a budget runs in tier 1 with the exact limit (`run_cached` already honours it). A conditional instruction whose condition fails still counts one instruction (static `done`).

## 5. Idle skipping and fixed per-tick cost

The existing rule (`machine/idle.rs`) is exact: from state `S0`, if after `L` instructions the registers, CPSR, SPSR and CP15 control equal `S0` and no store changed memory, no device page was touched, no hook ran and no unmapped page was mapped, `k*L` instructions are skipped. Idle v2 changes only the schedule and the comparison granularity.

| Stage | Change | Exactness argument | Cost effect | Files |
|---|---|---|---|---|
| 1 (tier 1 only, no trace needed) | `probe_idle_v2`: call `same_state` only when `r[15] == start.r[15]`; hoist the `HOOK_X` and `stop_pc` checks to page changes; after a failed probe grow the next slice geometrically past `MAX_SLICE` (up to 2^20, never past the tick end) | `same_state` compares `r`, which contains `r[15]`, so equal pc is a necessary condition and the first matching instant is unchanged; the skip itself is untouched; a larger slice only delays the next probe | removes about 20 words of compare per probed instruction; fewer probes | `idle.rs` |
| 2 (needs T2) | LoopWatch: arm when one trace self-links four times in a row or the same entry pc is dispatched twice; take `S0` at that dispatch point with `bus.probing = true`; compare at dispatch points and `LOOP_BACK` where `pc == S0.pc`; skip `k*L`; disarm with back-off on impurity or after `MAX_PERIOD` instructions | same skip rule; instants inspected are a subset of the instants the interpreter inspects, so a missed loop costs speed only; the probe runs on traces through the same instrumented `SysBus` accessors | probe instructions run at trace speed | `loopwatch.rs` |

Not in this work: zeroing 64 KB of low memory only when dirty (about 1.5 to 2 us per tick, ASSUMED) needs a `HOOK_W` dirty flag inside `SysBus::on_write` (existing file); listed as follow-up F6 for the seam owner.

Equivalence to the current path is proven per tick, not only per phase: the per-tick state hash of idle v2 equals the current `run_tick_idle` for all 1,900 ticks at five budgets (gate G1).

## 6. Events, interrupts, overlays, invalidation

| Event | Handling |
|---|---|
| Timer interrupt | Not asynchronous: `inject_irq` runs the AINTC handler as a nested `call_preserving` at tick boundaries, so no trace is ever interrupted. I1 makes the saved/restored `Cpu` irrelevant to traces. The handler (SRAM) is traced like any code. |
| Budget end, `until`, `stop_pc` | I7; the remainder runs in tier 1; the `stop`/`until` page test is an exact compare, never a bloom filter. |
| Code hooks (stubs, null-call guard) | Hooked pages are never traced and a trace never follows into one; `add_hook` sets `HOOK_X` through `add_flag`, which queues a `HOOK_C` page in `stale_code`, and `on_stale` drops every trace on a page that now has `HOOK_X`. The 1.9 % plain steps stay on `Cpu::step`. |
| Self-modifying code, SRAM overlays, program loads | Every page a trace reads is marked `HOOK_C` (`mark_code`). A store to it takes the `Mem` slow path, queues the page and clears the flag; the store uop ends the trace (I6); the driver drains the queue into `SbState::on_stale(page, mem)`, which revalidates (compares each trace's `words` with memory, keeps unchanged traces, re-marks `HOOK_C`, drops changed ones and resets links to them) or, for a `HOOK_X` page, drops all. A page whose words changed more than `volatile_after` times is flagged volatile: no traces, tier 1 only. Stores into the executing trace, the next word, or a folded literal's page are covered because the store uop exits before the next uop. |
| `drop_stale_code` is the single pop | Seam hunk 3 forwards each popped page to `self.sb.on_stale`. While `run_fast` holds the real `SbState` (`take_real`), `self.sb` is a placeholder that records the pages (`pending`); `drain_stale` forwards `m.sb.pending` to the real state before every drain, and `put_back` does it once more at the end, so no page is lost, also for pops made by a nested `Machine::run`. A nested run sees a placeholder with `cfg.enabled == false` and takes the tier-1 path, which is exact. |
| MMU, TLB, caches, permissions | None modelled (VA = PA, CP15 is a stub, all pages RWX, ASSUMED by `mem.rs`). Nothing to invalidate on `mcr c7/c8`. If an MMU model is added this table must grow. |
| Thumb | Not translated. `bx` and loads into `pc` with bit 0 set exit with `cpu.thumb = true`; tier 1 and `Cpu::step` run Thumb. Add a Thumb front end only if the dynamic Thumb share exceeds 3 % (counter in G0). |
| WFI | 0 encodings in the benchmark images; `mcr p15,0,rd,c7,c0,4` is Generic and raises `Fault::Wfi` exactly as before. |
| Faults, undefined, SWI | Generic; `Stop::Fault` reported as before; the faulting instruction is counted. |
| Device pages | T2 goes through `SysBus` (exact by construction). T3 exits to the interpreter for any flagged page (I10). |
| `Machine::clone`, threads | `SbState::clone` is empty (cold but correct). `SbState` is `Send`; the T3 JIT module is wrapped in a documented single-owner `unsafe impl Send` and a compile-time test asserts `Machine: Send`. |
| Arena reallocation (T3) | Only Rust code can map pages, never native code; `NativeCtl.arena` is refreshed before every native entry. |

## 7. Always-correct fallback

* Ladder: T3 exit or compile failure falls to T2; a missing trace falls to T1; `Machine.predecode = false` is T0. Each level is a config switch (`NS4_SB=0`, `NS4_SB_NATIVE=0`) and a runtime condition (executable memory unavailable, `bus.probing`, Thumb, hooked page, misaligned pc, budget below `n_guest`).
* `GENERIC` is the per-instruction fallback inside a trace (I8); native coverage grows per family without touching others.
* Quarantine: a trace that fails shadow or validation checks is dropped and its entry pc is added to a deny list (`hot.mark_failed`), with a loud counter (`shadow_mismatches`).
* Demotion: a T3 uop that exits more than 32 times is rebuilt as `GENERIC` and the trace recompiled; the exit histogram top 16 is printed by the bench.

## 8. Exactness and verification

Oracle: `Cpu::step` with `predecode = false` on a `SysBus`. The tiers are second and third implementations of the same semantics; they never replace the interpreter, and every doubt resolves to `GENERIC`.

Observable state compared everywhere: `r0..r15`, `n z c v q`, `thumb`, `irq_off`, `fiq_off`, `mode`, `mode_changes`, `spsr`, `cp15_ctrl`, `insns`, the fault, `Mem::state_hash`, `device_reads`, `poll_hits`, messages (path, comm, words, pc), `auto_mapped(_count)`, `tick_t`, I2C, panel and UART state, and the `Stop` reason.

| Layer | What | Where | Bound |
|---|---|---|---|
| L0 per uop | random state x random word per family (all 96 DP ids x conditions, shift amounts 0, 1, 2, 30, 31, 32, 33, 63, 64, 255, 256, every P/U/W combination, `rn == rd`, unaligned, page-end addresses, `ldm/stm` over a page end, all 16 NZCV), 1 to 3 instruction traces, against `Cpu::step` on a logging bus (registers, flags, bus access log) | `difftest.rs` | CI: `NS4_FUZZ_CASES` (default 60) scaling 20 k words per family; local gate: 1 M words per family |
| L1 programs, three-way | plain `Cpu::step`, T1, T1+T2 (and T3 when available) on the generator of `fastrun.rs`'s existing test extended with loops, `bl`/`bx lr` pairs, conditional runs, literal loads, stores into the own, next and literal page, hooks added between slices, Thumb entry, undefined words; budgets 1..7 and 1..400 so the budget expires at every uop position; `HOT` in {0, 1, 24}; trace caps {1, 2, 3, 128}; `until` and `stop` inside traces | `difftest.rs`, `testgen.rs` | CI 60 programs; local 2,000+ |
| L2 golden firmware | boot state hash `0960ef4eefb7fad5`; six phase message digests and state hashes at 200 k; whole-run digests `3efde38f525cfbd1` (150 k, 200 k) and `a3aab7d387c3f514` (300 k, 456 k) from PR 689; at 100 k, 150 k, 200 k, 300 k, 456 k with T2 on, off, `HOT = 0`, cap 3, idle v1 against v2 per tick. The goldens are re-derived with T1 on the current main first (OS 1.68 support in PR 687 may change them). Local only, skipped when `target/fw_unpacked` is absent, nothing printed (D20, D128) | `arm_sb_bench` | zero differences |
| L3 lockstep | chunks of 100 k instructions on boot and every tick: clone the `Machine`, run the reference on the clone, compare; on mismatch restore and halve the chunk to one instruction, print pc, word, register and flag deltas | `shadow.rs`, `arm_sb_bench` | clean over boot + 6 phases at 5 budgets |
| L4 translation validation | with `validate_cases = K` (default 0, CI real-firmware job 8), every newly built trace is run against the interpreter on K random states over a private mini machine holding copies of the trace's pages, biased registers (0, 1, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFF, addresses into mapped RAM, device pages, unmapped pages); catches lowering bugs on real firmware shapes | `validate.rs` | K random states per trace |
| L5 mutation | at least 40 mutants (flip a carry rule, drop a rotate, swap write-back order, wrong condition polarity, off-by-one `done`, wrong `last_pc`); each must be killed by L0 or L1 at the default case count; the comparator itself has mutants | `mutants.rs` | kill rate >= 95 % |
| L6 invariants | sum of `done` equals the `cpu.insns` delta; no trace entered with `left < n_guest`; `live`/`links` consistency after invalidation; every kind id has an arm; every `Exit.done` <= `n_guest` | tests in `exec.rs`, `cache.rs`, `run.rs` | always on |

T3 adds: its output is compared against the T2 executor on the same `Sb` (an in-process oracle at native speed) before it is compared with the interpreter; an exit sweep (for each trace and each k below its length, force a budget exit after k instructions and compare); the same L0 to L5 with the backend switched.

Translation validation, block-level fuzz with random states (circular state, boundary registers) and the real-trace lockstep are the answers to the orchestrator's critique points 3 and 5: a bug in a family costs a revert of that family to `GENERIC`, not a wrong result.

## 9. Tier 3 native tier (feature `sb-native`)

* **Input and output.** One `Sb` becomes one function `unsafe extern "C" fn(cpu: *mut Cpu, ctl: *mut NativeCtl) -> u32` returning the exit index. Registers `r0..r14` read by the trace and the four flags are Cranelift `Variable`s loaded at entry; the dirty subset and the flags that were defined are stored at every exit; `cpu.r[15]`, `cpu.insns` (`+= done`) are written by the exit epilogue. Field offsets come from `core::mem::offset_of!(Cpu, ...)` (stable, usable on `pub(crate)` fields); tests assert them against a live `Cpu`.
* **Memory.** `NativeCtl { table: *const u32, arena: *mut u8, limit: u64, exit: u32, target: u32, pad: [u32; 2] }`. Load fast path: `e = table[a >> 12]; t = e & 0x8FFF_FFFF; ok = (t - 1) <u 0x0FFF_FFFF` (mapped and no `HOOK_R`); store: mask `0x5FFF_FFFF` (no `HOOK_W`, no `HOOK_C`); host address `arena - 4096 + ((e & 0x0FFF_FFFF) << 12) + (a & 0xFFF)`; access wider than the page remainder, or any failed check, is an exit `Generic` at that pc with state exact before the instruction. The driver then runs one `Cpu::step` through `SysBus`, drains stale, and re-dispatches. Masks are constants in `abi.rs`, asserted equal to `mem::HOOK_*` by a test. Native code never runs while `bus.probing` is set.
* **Flags and conditions.** Computed from SSA operands; a compare followed by a guard fuses to one `icmp`; flag defs overwritten later in the trace are dead (the same rule as `flagopt.rs`).
* **Exits.** `Indirect` and `IndirectArm` exits are native (W1-NCORE, `emit.rs`: `ctl.target = r[reg] & !3` or `& !1` with the `OFF_THUMB` byte store for `bx`); `Dynamic` stays driver-side because the `GENERIC_END` before it is never native (the trace leaves with `exit_generic` at that pc). A lowerer returns `Unsupported` before it emits anything, a `PRED` whose body is not entirely lowerable is `Unsupported` itself (its pc is the first instruction of the body, so the state before it is exact), and `codegen::build_function` returns `None` for a uop list that does not end in a terminated block (the trace then stays on T2). `Generic` (`EXIT_GENERIC`, reported as `backend::EXIT_STEPPED` after the backend retired the counted instructions and stepped once), `EXIT_SLOW` (nothing ran, e.g. `bus.probing`) and `Stale` are driver exits; `LoopBack` is a native back-edge with a `cpu.insns + n_guest <= ctl.limit` check; chaining v1 returns to the driver (about 20 cycles); v2 (spike-gated) patches a link slot and uses `return_call_indirect` with the Tail convention.
* **Code memory.** One `JITModule` per backend, 32 MB cap; on overflow the module is dropped after the pass boundary and all traces are recompiled lazily (follow-up F7 replaces this with an arena if Windows page leaks show up, R9). Compile happens on the dispatch thread in v1 (ASSUMED 150 us per trace, about 4 ms for the heavy region); a bounded SPSC background mode is follow-up F8.
* **Unsafe.** `machine/sb/mod.rs` carries `#![deny(unsafe_code)]`; only `native/rt.rs` has `#![allow(unsafe_code)]` (code-pointer transmute, the C-ABI call, `unsafe impl Send`). `mem/raw.rs` returns raw pointers (safe to create).
* **Spike first (W1-NCORE).** Two-trace ping-pong with `return_call_indirect`, executable-memory allocation, and `Send` of the module on ubuntu, windows and macos CI (the `sb-native` tests run under `--all-features` on all three). The spike runs on CI only: the work package writes it and checks it with `cargo check`, it is not executed locally on the other two systems, and an unproven target keeps `available()` false. `rt.rs` also holds the module wrapper `Jit` (owns the `JITModule`, `define(func) -> Option<NativeFn>`), the single place with `unsafe`; W2-NDP and W2-NMEM run their native tests through it and `NativeBackend` wraps it. `native::available()` is a runtime probe; tests assert the fallback when it is false instead of skipping.

## 10. Cost model

Method: rows are cycles per 1,000 guest instructions of the firmware static mix (ASSUMED proxy for the dynamic mix; the heavy routine has more register arithmetic and conditionals and fewer memory operations, so its native share is higher). Conversion keeps the measured baseline as the unit: `ns = 5.3 x cycles / 17`, so no clock frequency enters (unit risk R3). Counts of instruction-bearing rows sum to 1,000.

### 10.1 Tier 2

| # | Row | per 1,000 | cycles each | cycles per 1,000 | Tag, basis |
|---|---|---|---|---|---|
| 1 | dp uops, flag nibble in a local | 417 | 4.0 | 1,668 | ASSUMED; today 8.8 cycles on the pure-dp synthetic loop (EMPIRICAL); mix from the static census (mov 15.9 %, add 10.6, cmp 6.2, sub 2.3, orr 1.5, shifts 1.5) |
| 2 | pc-literal loads folded to `MOV_IMM` | 44 | 3.0 | 132 | ASSUMED; 21,406 sites = 21 % of loads in the static census |
| 3 | loads (`SysBus` accessors) | 164 | 9.0 | 1,476 | ASSUMED, about 25 host instructions |
| 4 | stores plus stale check | 138 | 10.0 | 1,380 | ASSUMED |
| 5 | `ldm/stm/ldrd/strd` | 55 | 16.0 | 880 | ASSUMED, 5.2 registers average |
| 6 | `b/bl` followed inside a trace | 67 | 1.5 | 101 | ASSUMED; today about 24 cycles per ender (205 against 375 M/s) |
| 7 | conditional-branch guards | 60 | 4.0 | 240 | ASSUMED |
| 8 | guard mispredicts (10 % of guards, 18 cycles) | 6 | 18 | 108 | ASSUMED |
| 9 | indirect exits with a 2-way inline cache | 25 | 15 | 375 | ASSUMED |
| 10 | `mrs/msr` | 20 | 6 | 120 | ASSUMED |
| 11 | trace transitions (spill, budget test, link) | 40 | 10 | 400 | ASSUMED, about 25 guest insns per entry |
| 12 | `GENERIC` uops | 10 | 30 | 300 | ASSUMED |
| 13 | executor dispatch mispredicts (new) | | 0.4 per uop | 400 | ASSUMED, 2 % of about 950 uops at 20 cycles; one shared indirect jump |
| 14 | register-file-in-memory dependency chains (new) | | 0.5 | 500 | ASSUMED; store-to-load forwarding on serial soft-float chains |
| 15 | plain `Cpu::step` instructions (new) | 19 | 46 | 874 | count EMPIRICAL (1.9 %, PR 689); cost ASSUMED = 36 cycles (10.7 ns old interpreter) + 10 for the tier switch |
| | **Total (best)** | | | **8,954** | 8.95 cycles |

Likely = best x 1.35 = 12.1 cycles; worst = best x 1.7 = 15.2 (ASSUMED spreads; ARM-SB used 1.45 and 1.85 with rows 13 to 15 missing; the predecode step was predicted 5x and measured 2.0x, so a bottom-up sum is not trusted).

### 10.2 Tier 3

| # | Row | cycles per 1,000 | Tag, basis |
|---|---|---|---|
| A | ALU, flags, register shifts, long multiply, conditional-execution surcharge | 1,622 | ASSUMED; ARM-X rows 1 to 5 (0.9 x 430 + 2.6 x 190 + 3.8 x 70 + 3.5 x 50 + 1.0 x 300) |
| B | memory: word/byte/half 4.2 x 150, `ldm/stm` 11 x 40, folded literals 0.4 x 20 | 1,078 | ASSUMED; page-table fast path of about 12 host instructions |
| C | trace boundaries: 40 entries x 20 cycles (v1 dispatcher-return); 40 x 6 with tail-call chaining | 800 | ASSUMED |
| D | native exits to `Generic`: 5 per 1,000 (0.5 %) x 150 cycles | 750 | ASSUMED; gate G3 caps at 1 % |
| E | plain `Cpu::step` instructions (row 15) | 874 | as above |
| | **Total (likely)** | **5,124** | 5.1 cycles |

Best = 0.7 x likely = 3.6; worst = 1.8 x likely = 9.2 (ASSUMED, the spread ARM-X used).

### 10.3 Consequences (ASSUMED, from 10.1 and 10.2)

| Tier | cycles/insn best / likely / worst | ns/insn | speedup against 17 | M insns/s likely |
|---|---|---|---|---|
| T1 (measured best run) | 17 | 5.3 | 1.0x | 189 |
| T2 | 8.95 / 12.1 / 15.2 | 2.79 / 3.77 / 4.74 | 1.9x / 1.4x / 1.1x | 265 |
| T3 | 3.6 / 5.1 / 9.2 | 1.12 / 1.60 / 2.87 | 4.7x / 3.3x / 1.9x | 625 |

Heavy tick (a full budget of real work), load in host cores = budget x ns / 1e6; under 1.0 means the tick fits in 1 ms. A loaded host multiplies by about 1.5 (25 against 17 cycles measured).

| Budget (insns/ms) | T1 | T2 best / likely / worst | T3 best / likely / worst | T3 likely, loaded x1.5 |
|---|---|---|---|---|
| 200 k | 1.06 | 0.56 / 0.75 / 0.95 | 0.22 / 0.32 / 0.57 | 0.48 |
| 300 k | 1.59 | 0.84 / 1.13 / 1.42 | 0.34 / 0.48 / 0.86 | 0.72 |
| 456 k | 2.42 | 1.27 / 1.72 / 2.16 | 0.51 / 0.73 / 1.31 | 1.10 |

Backlog over the 6.5 M-instruction stretch = (load - 1) x virtual duration (32.5, 21.7, 14.3 ms at 200, 300, 456 k), derived and assuming a homogeneous stretch (the measured T1 backlogs are 0.5, 9.4, 17.0 ms, so this over-predicts by 15 to 300 %): T1 2.0 / 12.8 / 20.2 ms; T2 likely 0 / 2.8 / 10.3 ms; T3 likely 0 / 0 / 0. This is the lead the decoupled producer of PR 689 needs (25 to 69 ticks measured); T3 likely removes the need for a long queue at all three budgets, T2 only at 200 k.

Steady state, 24 kHz framing, 3,384 executed insns per tick (ASSUMED to hold at that framing; G0 measures it) = 81 M insns/s: T1 0.43 core, T2 likely 0.31, T3 likely 0.13, each plus the fixed per-tick cost (45 to 48 us per tick today, measured at the 1 ms framing). At the 1 ms framing the instruction part is negligible (0.02 core) and the fixed cost is the whole story (0.047 core).

Compile and build costs: T2 trace build ASSUMED 5 to 10 us per 100-instruction trace, capped at 4 traces and 512 lowered instructions per `run()` call; T3 ASSUMED 150 us per trace.

## 11. Skeleton contract

One agent (WP S0, wave 0) creates exactly the following. The result compiles and passes `cargo test -p nord-arm` with every tier off. With `SbConfig::on()` every instruction is lowered to `GENERIC`, so the whole stack is correct (just slow) before any family lands. Every `.rs` file carries at least one `#[test]` (`tools/check_tests.py`), stays under about 400 lines, and a work package may split its own file with `mod x;` declared inside that file (`exec.rs` plus `exec/x.rs`) without touching S0 files.

### 11.1 Edits to existing files (all one-liners, nothing else until W4-SEAM)

| File | Edit |
|---|---|
| `crates/nord-arm/src/machine/mod.rs` | `pub mod sb;` (alphabetical, between `probe` and `sysbus`) |
| `crates/nord-arm/src/mem.rs` | `#[cfg(feature = "sb-native")] mod raw;` |
| `crates/nord-arm/Cargo.toml` | `[features]` and optional dependencies below |
| `crates/nord-tools/Cargo.toml` | `[features] sb-native = ["nord-arm/sb-native"]` and two `[[bin]]` entries (`arm-sb-bench`, `arm-sb-ab`) appended at the end |

```toml
# crates/nord-arm/Cargo.toml
[features]
default = []
# Tier 3: native code from the superblock IR. Off by default; adds the only dependencies and the only unsafe code of the crate.
sb-native = ["dep:cranelift-codegen", "dep:cranelift-frontend", "dep:cranelift-jit", "dep:cranelift-module", "dep:cranelift-native"]

[dependencies]
cranelift-codegen = { version = "0.116", optional = true }
cranelift-frontend = { version = "0.116", optional = true }
cranelift-jit = { version = "0.116", optional = true }
cranelift-module = { version = "0.116", optional = true }
cranelift-native = { version = "0.116", optional = true }
```

Runtime switches (all default off): `NS4_SB=1`, `NS4_SB_IDLE2=1`, `NS4_SB_NATIVE=1`, `NS4_SB_HOT=n`, `NS4_SB_VALIDATE=n`. No other feature flag exists.

### 11.2 Module tree (all new; paths relative to `crates/nord-arm/src/`)

```
machine/sb/mod.rs        SbState, Watch, re-exports, #![deny(unsafe_code)]            [S0, frozen]
machine/sb/uop.rs        Uop, k::*, ExitKind, Exit, Sb, SbId, pack/unpack/holds        [S0, frozen]
machine/sb/uop/{k,fused}.rs  kind ids and registry; fused layouts fuse_pair/expand       [S0, frozen]
machine/sb/config.rs     SbConfig                                                      [S0, frozen]
machine/sb/stats.rs      SbStats                                                       [S0, frozen]
machine/sb/decode.rs     Insn view over predecode::Entry                               [S0, frozen]
machine/sb/bus.rs        trait SbBus + impl for SysBus                                 [S0, frozen]
machine/sb/backend.rs    trait Backend, RunOut                                         [S0, frozen]
machine/sb/lower_dp.rs   lower_dp                                                      [W1-DP]
machine/sb/lower_mem.rs  lower_mem                                                     [W1-MEM]
machine/sb/lower_flow.rs lower_flow, Flow, FlowCtx                                     [W1-FLOW]
machine/sb/trace.rs      BuildOpts, build                                              [W1-FLOW]
machine/sb/exec_alu.rs   DP arms, flag arithmetic                                      [W1-XALU]
machine/sb/exec_mem.rs   memory arms                                                   [W1-XMEM]
machine/sb/exec.rs       run_sb                                                        [W2-XCORE]
machine/sb/exec_flow.rs  PRED/GUARD/EXIT/LOOP/GENERIC/MRS/MSR/fused arms               [W2-XCORE]
machine/sb/cache.rs      SbCache                                                       [W1-CACHE]
machine/sb/hot.rs        HotTable                                                      [W1-CACHE]
machine/sb/stale.rs      SbState::on_stale, revalidation, volatile pages               [W1-CACHE]
machine/sb/run.rs        run_fast, run_sb_full, drain_stale                            [W2-DRV]
machine/sb/fuse.rs pred.rs flagopt.rs   peephole passes                                [W2-PEEP]
machine/sb/idle.rs       probe_idle_v2, run_idle_skip_v2 (tier 1 only)                 [W1-IDLE]
machine/sb/loopwatch.rs  run_tick_idle_v2, LoopWatch                                   [W3-LOOP]
machine/sb/shadow.rs     lockstep, bisect                                              [W3-VERIF]
machine/sb/validate.rs   validate_trace                                                [W3-VERIF]
machine/sb/testkit.rs testgen.rs   (cfg(test)) oracle runners, snapshots, generators   [W1-VGEN]
machine/sb/difftest.rs mutants.rs  (cfg(test)) L0, L1, L5                              [W3-VERIF]
machine/sb/native/{mod,abi,codegen,emit,rt}.rs                 (feature sb-native)     [W1-NCORE]
machine/sb/native/lower_dp.rs                                                          [W2-NDP]
machine/sb/native/lower_mem.rs, lower_flow.rs                                          [W2-NMEM]
machine/sb/native/backend.rs, link.rs, difftest.rs                                     [W3-NINT]
mem/raw.rs               Mem::raw_parts (feature sb-native)                            [W1-NCORE]
crates/nord-tools/src/bin/arm_sb_bench.rs                                              [W1-CNT]
crates/nord-tools/src/bin/arm_sb_ab.rs                                                 [W4-BENCH]
```

### 11.3 Frozen types and signatures

```rust
// machine/sb/uop.rs
pub const MAX_UOPS: usize = 128; pub const MAX_EXITS: usize = 64; pub const MAX_PAGES: usize = 4;
pub type SbId = u32; pub const NO_SB: SbId = u32::MAX;
#[repr(C)] #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Uop { pub kind: u8, pub cond: u8, pub rd: u8, pub rn: u8, pub rm: u8, pub rs: u8, pub sh: u8, pub n: u8, pub imm: u32, pub pc: u32 }
const _: () = assert!(core::mem::size_of::<Uop>() == 16);
pub mod k {                       // ids as in section 3.1
    pub const DP_BASE: u8 = 0;  pub const fn dp(form: u32, opcode: u32, s: bool) -> u8 { (form * 32 + opcode * 2 + s as u32) as u8 }
    pub const MOV_IMM: u8 = 96; pub const MOV_REG: u8 = 97; pub const MVN_IMM: u8 = 98; pub const ADD_IMM: u8 = 99; pub const SUB_IMM: u8 = 100;
    pub const ADD_REG: u8 = 101; pub const SUB_REG: u8 = 102; pub const CMP_IMM: u8 = 103; pub const CMP_REG: u8 = 104; pub const TST_IMM: u8 = 105;
    pub const TST_REG: u8 = 106; pub const ORR_IMM: u8 = 107;
    pub const LDR_IMM: u8 = 112; pub const LDRB_IMM: u8 = 113; pub const STR_IMM: u8 = 114; pub const STRB_IMM: u8 = 115;
    pub const LDR_REG: u8 = 116; pub const LDRB_REG: u8 = 117; pub const STR_REG: u8 = 118; pub const STRB_REG: u8 = 119;
    pub const LDRH: u8 = 120; pub const LDRSH: u8 = 121; pub const LDRSB: u8 = 122; pub const STRH: u8 = 123; pub const LDRD: u8 = 124; pub const STRD: u8 = 125;
    pub const LDM: u8 = 126; pub const STM: u8 = 127; pub const LDR_ABS: u8 = 128;
    pub const PRED: u8 = 144; pub const GUARD: u8 = 145; pub const GUARD_REG_EQ: u8 = 146; pub const LOOP_BACK: u8 = 147; pub const EXIT: u8 = 148;
    pub const EXIT_REG: u8 = 149; pub const EXIT_REG_ARM: u8 = 150; pub const GENERIC: u8 = 151; pub const GENERIC_END: u8 = 152; pub const MRS: u8 = 153; pub const MSR_C: u8 = 154;
    pub const CMP_GUARD_I: u8 = 160; pub const CMP_GUARD_R: u8 = 161; pub const TST_GUARD_I: u8 = 162; pub const MOV_PAIR: u8 = 163; pub const LDR_PAIR: u8 = 164;
    pub const COUNT: usize = 165;
}
#[repr(u8)] #[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitKind { Direct, Indirect, IndirectArm, Dynamic, Fault, Stale, LoopBack, Watch }
#[derive(Clone, Debug)] pub struct Exit { pub kind: ExitKind, pub reg: u8, pub done: u16, pub target: u32, pub last_pc: u32, pub link: SbId, pub ic_pc: [u32; 2], pub ic_id: [SbId; 2], pub hits: u32 }
#[derive(Clone, Debug)] pub struct Sb { pub entry: u32, pub pages: [u32; MAX_PAGES], pub n_pages: u8, pub n_guest: u16, pub uops: Box<[Uop]>, pub exits: Box<[Exit]>, pub words: Box<[(u32, u32)]>, pub live: bool, pub enters: u32, pub native: SbId }
pub fn pack_flags(cpu: &crate::cpu::Cpu) -> u8;                    // cpu.nzcv() as u8
pub fn unpack_flags(cpu: &mut crate::cpu::Cpu, f: u8);             // writes n z c v only
pub fn holds(cond: u8, f: u8) -> bool;                             // COND_MASK[cond] >> f & 1; cond 14 and 15 always

// machine/sb/config.rs
#[derive(Clone, Debug)] pub struct SbConfig { pub enabled: bool, pub idle_v2: bool, pub native: bool, pub hot: u16, pub slice: u32, pub max_uops: usize, pub max_guards: usize,
    pub max_pages: usize, pub inline_depth: u8, pub builds_per_run: u8, pub uop_cap: usize, pub volatile_after: u8, pub validate_cases: u32,
    pub fuse: bool, pub predicate: bool, pub dead_flags: bool, pub fold_literals: bool }
impl Default for SbConfig { /* all behaviour off; constants of section 3.3 */ }
impl SbConfig { pub fn from_env() -> Self; pub fn on() -> Self /* enabled, hot = 0, for tests */ }

// machine/sb/bus.rs
pub trait SbBus: crate::cpu::Bus + 'static { fn set_pc(&mut self, pc: u32); fn stale(&self) -> bool; }
impl SbBus for crate::machine::SysBus { /* pc field; !mem.stale_code.is_empty() */ }

// machine/sb/backend.rs
pub struct RunOut { pub exit: u16, pub target: u32 }
pub trait Backend { fn compile(&mut self, sb: &Sb) -> Option<u32>; fn release(&mut self, id: u32); fn name(&self) -> &'static str;
    fn run(&mut self, id: u32, cpu: &mut Cpu, bus: &mut SysBus, left: &mut u64) -> RunOut; }   // same contract as exec::run_sb

// machine/sb/mod.rs
#[derive(Default)] pub struct SbState { pub cfg: SbConfig, pub cache: cache::SbCache, pub hot: hot::HotTable, pub stats: SbStats, pub watch: Watch,
    pub backend: Option<Box<dyn backend::Backend + Send>> }
impl Clone for SbState { /* same cfg, everything else empty */ }
#[derive(Clone, Copy, Default, Debug)] pub struct Watch { pub armed: bool, pub hit: bool, pub pc: u32 }

// machine/sb/decode.rs
#[derive(Clone, Copy)] pub struct Insn { pub pc: u32, pub word: u32, pub entry: crate::cpu::predecode::Entry }
pub fn decode(pc: u32, word: u32) -> Insn;
impl Insn { pub fn cond(&self) -> u8; pub fn is_cond(&self) -> bool; pub fn rd(&self) -> u8; pub fn rn(&self) -> u8; pub fn rm(&self) -> u8; pub fn rs(&self) -> u8;
    pub fn form(&self) -> u32; pub fn is_ender(&self) -> bool; pub fn branch_target(&self) -> u32; }

// lowering (machine/sb/lower_*.rs, trace.rs)
pub enum Lowered { Done, Generic }
pub struct LowerCtx<'a> { pub insn: Insn, pub out: &'a mut Vec<Uop>, pub fold: &'a mut dyn FnMut(u32) -> Option<u32>, pub opts: &'a BuildOpts }
pub fn lower_dp(cx: &mut LowerCtx<'_>) -> Lowered;           // stub: Generic
pub fn lower_mem(cx: &mut LowerCtx<'_>) -> Lowered;          // stub: Generic
pub enum Flow { Follow(u32), End }
pub struct FlowCtx<'a> { pub insn: Insn, pub uops: &'a mut Vec<Uop>, pub exits: &'a mut Vec<Exit>, pub done: u16, pub taken_hint: Option<bool>,
    pub can_follow: &'a dyn Fn(u32) -> bool, pub depth: &'a mut u8, pub rets: &'a mut Vec<u32>, pub entry_pc: u32, pub opts: &'a BuildOpts }
pub fn lower_flow(cx: &mut FlowCtx<'_>) -> Flow;             // stub: ends the trace with GENERIC_END
#[derive(Clone, Debug)] pub struct BuildOpts { pub max_uops: usize, pub max_guards: usize, pub max_pages: usize, pub inline_depth: u8,
    pub fuse: bool, pub predicate: bool, pub dead_flags: bool, pub fold_literals: bool }
impl From<&SbConfig> for BuildOpts { /* field copy */ }
pub fn build(mem: &mut crate::mem::Mem, start: u32, opts: &BuildOpts) -> Option<Sb>;   // stub: one GENERIC_END trace

// executor (machine/sb/exec.rs)
pub fn run_sb<B: SbBus>(cpu: &mut Cpu, bus: &mut B, sb: &Sb, left: &mut u64, watch: bool) -> RunOut;   // stub: executes GENERIC and EXIT only

// cache (machine/sb/cache.rs, hot.rs, stale.rs)
impl SbCache { pub fn lookup(&self, pc: u32) -> Option<SbId>; pub fn get(&self, id: SbId) -> &Sb; pub fn get_mut(&mut self, id: SbId) -> &mut Sb;
    pub fn insert(&mut self, sb: Sb) -> SbId; pub fn is_live(&self, id: SbId) -> bool; pub fn live_count(&self) -> usize; pub fn uops_total(&self) -> usize;
    pub fn flush(&mut self); pub fn volatile(&self, page: u32) -> bool; }
impl HotTable { pub fn bump(&mut self, pc: u32) -> u16; pub fn reset(&mut self, pc: u32); pub fn mark_failed(&mut self, pc: u32); pub fn failed(&self, pc: u32) -> bool; }
impl SbState { pub fn on_stale(&mut self, page: u32, mem: &mut crate::mem::Mem);            // on a placeholder: records the page
    pub fn placeholder() -> Self; pub fn take_real(slot: &mut SbState) -> SbState;
    pub fn absorb_pending(&mut self, slot: &mut SbState, mem: &mut crate::mem::Mem); pub fn put_back(self, slot: &mut SbState, mem: &mut crate::mem::Mem); }
// decode.rs: pub fn touches_r15(u: &Uop) -> bool;  uop/fused.rs: pub fn fuse_pair(a: &Uop, b: &Uop) -> Option<Uop>; pub fn expand(u: &Uop) -> Option<[Uop; 2]>;
// backend.rs: pub const EXIT_SLOW: u16; pub const EXIT_STEPPED: u16;  mutants.rs: MUTANTS: &[(u32, &str)] with id blocks reserved per work package

// driver (machine/sb/run.rs)
pub fn run_fast(m: &mut Machine, sb: &mut SbState, limit: u64, until: u32, stop: u32) -> Option<Fault>;      // stub: delegates to m.run_cached
pub fn run_sb_full(m: &mut Machine, sb: &mut SbState, max: u64, until: u32, stop_pc: Option<u32>) -> (u64, Stop);
pub fn drain_stale(m: &mut Machine, sb: &mut SbState);

// idle (machine/sb/idle.rs, loopwatch.rs)
pub fn probe_idle_v2(m: &mut Machine, limit: u64) -> (Option<u64>, Option<Stop>);
pub fn run_idle_skip_v2(m: &mut Machine, budget: u64, stats: &mut IdleStats) -> Stop;
pub fn run_tick_idle_v2(m: &mut Machine, sb: &mut SbState, insns: u64, stats: &mut IdleStats) -> Stop;    // stub: tier-1 variant

// verification (machine/sb/shadow.rs, validate.rs)
pub struct Divergence { pub at_insn: u64, pub pc: u32, pub field: String }
pub fn lockstep(m: &mut Machine, sb: &mut SbState, chunk: u64, total: u64, until: u32, stop: Option<u32>) -> Result<u64, Divergence>;
pub fn bisect(before: &Machine, sb_cfg: &SbConfig, chunk: u64, until: u32, stop: Option<u32>) -> Divergence;
pub fn validate_trace(m: &Machine, sb: &Sb, cases: u32, seed: u64) -> Result<(), String>;

// native (feature sb-native)
pub fn available() -> bool;                                    // machine/sb/native/mod.rs
pub struct NativeBackend; impl NativeBackend { pub fn new() -> Option<Self> } impl Backend for NativeBackend { /* W3-NINT */ }
#[repr(C)] pub struct NativeCtl { pub table: *const u32, pub arena: *mut u8, pub limit: u64, pub exit: u32, pub target: u32, pub pad: [u32; 2] }   // abi.rs
pub const MASK_LOAD: u32 = 0x8FFF_FFFF; pub const MASK_STORE: u32 = 0x5FFF_FFFF; pub const MASK_IDX: u32 = 0x0FFF_FFFF;
pub struct NCtx<'a> { /* builder, cpu/ctl values, reg(n), set_reg(n, v), flag(i), set_flag(i, v), exit_generic(pc), exit(idx), dirty masks (W1-NCORE) */ }
pub fn lower_dp(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported>;     // native/lower_dp.rs; same for lower_mem, lower_flow; stubs: Err
// machine/../mem/raw.rs
#[derive(Clone, Copy)] pub struct RawMem { pub table: *const u32, pub arena: *mut u8 }
impl Mem { pub fn raw_parts(&mut self) -> RawMem; }
```

### 11.4 Seam (only W4-SEAM, after PR 689 and PR 687 are merged or this branch is rebased over them; at most 30 changed lines)

```rust
// machine/mod.rs: field, constructor, run() call site, and the method
pub struct Machine { ..., pub sb: sb::SbState }               // Machine::new(): sb: sb::SbState::default()
// in run():  if let Some(f) = self.run_tier(limit, until, stop) { ... }   instead of  self.run_cached(limit, until, stop)
impl Machine { fn run_tier(&mut self, limit: u64, until: u32, stop: u32) -> Option<Fault> {
    if !self.sb.cfg.enabled { return self.run_cached(limit, until, stop); }
    let mut sb = sb::SbState::take_real(&mut self.sb); let r = sb::run::run_fast(self, &mut sb, limit, until, stop); sb.put_back(&mut self.sb, &mut self.bus.mem); r } }
// machine/fastrun.rs, drop_stale_code(): after self.code.drop_page(p):  self.sb.on_stale(p, &mut self.bus.mem);
// machine/idle.rs, run_tick_idle(): first line:  if self.sb.cfg.idle_v2 { let mut sb = sb::SbState::take_real(&mut self.sb); let s = sb::loopwatch::run_tick_idle_v2(self, &mut sb, insns, stats); sb.put_back(&mut self.sb, &mut self.bus.mem); return s; }
// run.rs (W2-DRV owns the file, W4-SEAM adds the line once the field exists): drain_stale first calls sb.absorb_pending(&mut m.sb, &mut m.bus.mem)
```

`take_real` makes the take cheap (a handful of pointers; the placeholder holds an empty `Vec`, which does not allocate). While `run_fast` runs, `self.sb` is the placeholder; that is safe because it is inert (`cfg.enabled` false, so a nested `Machine::run` stays in tier 1) and records every stale page for the real state (I11, section 6). The skeleton already made `same_state` and `Machine::probe_loop` of `machine/idle.rs` `pub(super)` (a visibility change, no logic) so W1-IDLE and W3-LOOP reuse them instead of copying.

## 12. Work packages

Waves: a wave starts when all its dependencies are merged into the working branch. Waves 1 to 3 touch no existing file except through S0. No work package runs a wall-clock benchmark; each runs its own tests and `cargo clippy -p nord-arm --all-targets -- -D warnings` (CARGO_BUILD_JOBS=4). Nothing is pushed or merged by a work package.

| ID | Wave | Owns (disjoint) | Depends on | Content | Test obligation |
|---|---|---|---|---|---|
| S0 | 0 | everything in 11.1, 11.2 marked frozen; stubs for all other files | none | the skeleton of section 11; trivial test per file; kind-id test; `size_of::<Uop>() == 16`; `holds` equals `Cpu::cond` for 14 conditions x 16 nibbles; pack/unpack round trip | `cargo test -p nord-arm` and `--all-features` pass; every file has a test |
| W1-DP | 1 | `lower_dp.rs` | S0 | all 96 DP ids, specialised ids, pc folding (`add rd,pc,#x`, `mov rd,pc`), form 2 pc+12 rule, rd=15 and S-with-rd=15 to Generic, `mov pc,rm` to `EXIT_REG_ARM` through `Lowered`/`Flow` contract | decision tests (kind chosen, folded constants, rejected forms) for 300 random words per class; no semantic test here (L0 covers it in W3-VERIF) |
| W1-MEM | 1 | `lower_mem.rs` | S0 | word/byte immediate and register offsets, half/signed/double, `ldm/stm` S=0, literal fold with page registration through `fold`, faulting `ldrd/strd` forms stay Generic | same style |
| W1-FLOW | 1 | `trace.rs`, `lower_flow.rs` | S0 | trace formation of 4.2, `BuildOpts`, polarity of guards, inline depth, `GUARD_REG_EQ` returns, loop-back, exits with static `done`/`last_pc`, `n_guest`, page list, `words` snapshot, caps; mapped/unhooked checks (I9) | shape tests: follow B/BL, polarity, stop at hooked/unmapped page, caps 1/2/3/128, `n_guest` equals longest path, `words` equals memory |
| W1-XALU | 1 | `exec_alu.rs` | S0 | DP arms per (OP, S, FORM), shifter, nibble arithmetic, native `mul/mla` | each arm against `Cpu::dp_exec`/`arm_mul` on 2,000 random states per (op, S, form) plus edge values; Uop built by hand from the field table |
| W1-XMEM | 1 | `exec_mem.rs` | S0 | memory arms generic over `SbBus`, ldm/stm via one address computation, rotate/mask rules, write-back rules | against `ldst_exec`, `arm_ldst_half`, `arm_ldm` on a logging bus (registers equal, ordered access log equal), post-index with `rn == rd`, base in list |
| W1-CACHE | 1 | `cache.rs`, `hot.rs`, `stale.rs` | S0 | cache (page arrays of 3.3 in place of the skeleton's `HashMap`s; `lookup`, `covering` and `on_stale` allocate nothing after a page's first insert), link-safe liveness, uop cap flush, `on_stale` with revalidation by `words`, volatile pages, `HOOK_C` re-mark, `HOOK_X` drop | data store on a code page keeps unchanged traces; changed word drops exactly the affected traces; folded literal change drops its trace; hook drops all; volatile after 5 changes; flush at cap; empty `Clone` |
| W1-IDLE | 1 | `idle.rs` | S0 | stage 1 of section 5, reusing `machine::idle::same_state` and `Machine::probe_loop` (made `pub(super)` by the skeleton; never a copy) | v2 equals the current `run_idle_skip` final state on the six existing idle tests plus 200 random programs with spin loops of period 1..40; probe state-compare count lower (counter) |
| W1-VGEN | 1 | `testkit.rs`, `testkit/snapshot.rs`, `testgen.rs` | S0 | deterministic `Rng`, biased random `Cpu`/`Machine` states, oracle runner (T0 on a clone), full-state snapshot and comparator with readable diff, the L1 program generator extended per section 8, logging bus | own tests: determinism, comparator detects each field it claims to compare (mutation of one field per test) |
| W1-CNT | 1 | `crates/nord-tools/src/bin/arm_sb_bench.rs` | S0 | mode 1 (counters, deterministic): per phase `probe_executed / executed`, fixed tick cost split, executed insns per tick at 1 ms and 24 kHz framing, golden hash/digest check; arguments as `arm-lle-bench`; uses main APIs only | tests for argument parser and table printer; exits non-zero on digest mismatch |
| W1-NCORE | 1 | `native/{mod,abi,codegen,emit,rt}.rs`, `mem/raw.rs` | S0 | Cranelift module wrapper `rt::Jit`, function skeleton, variables, exits epilogue including `Indirect`/`IndirectArm`, `NativeCtl`, `NCtx`, `Backend` plumbing helpers, `Mem::raw_parts`, the three-OS spike (tail-call ping-pong, executable memory, `Send`; CI only), `available()` | IR level, no native lowerers exist yet: `build_function` of hand-built `Sb`s verifies with `verify_function` (an `exit` lowered through the `emit::testing` helper terminates, `is_terminated`, an empty uop list gives `None`); `Indirect`/`IndirectArm` exits store `ctl.target` and the `OFF_THUMB` byte; `Jit::define` of a hand-written function runs through `rt::call` where `available()` is true, and the fallback is asserted where it is false; `offset_of!` offsets equal a live `Cpu`; masks equal `mem::HOOK_*` |
| W1-DOC | 1 | `docs/specs/arm0-superblocks.md`, `.beans/ns4-arm-sb--*.md` (the only package that creates the bean; later packages add no beans, W4-DOC ticks it) | S0 | spec (structures, exits, invalidation, exactness contract, L0 to L6) and the bean (Context, Higher Goal, Acceptance Criteria = gates, Out of Scope, Summary of Changes) | none (docs); links checked |
| W2-XCORE | 2 | `exec.rs`, `exec_flow.rs` | W1-XALU, W1-XMEM | `run_sb`, all flow, generic, `MRS/MSR_C`, fused arms (equal to their unfused expansions), stale exit, watch exit, loop accumulator | hand-built `Sb` per exit kind with exact `done`, `last_pc`, `cpu.insns`; loop-back with budgets; predicate skip; fault inside a trace; stale exit after a store into the own page; fused equal unfused on random states; every kind id has an arm |
| W2-DRV | 2 | `run.rs` | W1-FLOW, W1-DP, W1-MEM, W1-CACHE, W2-XCORE | section 4.1 in full: lookup, build, mark, chain with link patching and inline cache, guard-page test, budget test, stale drain, watch return, `run_sb_full`, tier switch rules, per-run build cap, flipped re-layout | build/hit/invalidate cycle on synthetic code; link patched after first lookup; dead trace not followed; build cap; budget below `n_guest` runs tier 1; `until`/`stop` inside a trace page |
| W2-PEEP | 2 | `fuse.rs`, `pred.rs`, `flagopt.rs` | W1-FLOW, W2-XCORE | cmp+Bcc fusion, mov pairs, ldr pairs (top bigrams from a counter), predicate grouping 2..8, dead-flag elimination across no store/guard/Generic/exit | each pass preserves state against the unpassed trace on 1,000 random traces and never changes `done`/`last_pc` of an exit |
| W2-NDP | 2 | `native/lower_dp.rs` | W1-NCORE, W1-XALU | native lowering of DP, shifts, flags, conditions, mul, specialised ids | per kind, native (through `rt::Jit`) against T2 `exec_alu` on random states, native tests assert the fallback where `available()` is false; mutants ids 800..=849 |
| W2-NMEM | 2 | `native/lower_mem.rs`, `native/lower_flow.rs` | W1-NCORE, W1-XMEM, W2-XCORE | page-table fast path, `ldm/stm` range check, guards, exits, loop-back with `ctl.limit` | native against T2 on random addresses including unaligned, page ends, `HOOK_R/W/C` pages, unmapped; every slow case leaves the interpreter to perform the access with identical device and auto-map accounting; a `PRED` is native only when its whole body is (otherwise `Unsupported` at the `PRED`); mutants ids 850..=899 |
| W3-LOOP | 3 | `loopwatch.rs` | W2-DRV, W1-IDLE | section 5 stage 2: arm, compare at dispatch points, skip, back-off, `run_tick_idle_v2`; the watch contract with W2-DRV is: while `SbState::watch.armed` the driver passes `watch = true` to the executor and, at every dispatch point with `pc == watch.pc`, sets `watch.hit` and returns; `run_sb` takes the `Watch` exit at `LOOP_BACK` in that case; W3-LOOP reads and writes only `SbState::watch` and the `Machine`, it edits no driver file | port of every test in `machine/idle.rs` through v2; random-program equivalence of final state hash v1 against v2 over 200 programs with spin period 1..40 and noise before; WFI semantics identical |
| W3-VERIF | 3 | `difftest.rs`, `mutants.rs`, `shadow.rs`, `validate.rs` | W2-DRV, W2-XCORE, W2-PEEP, W1-VGEN | L0, L1, L4, L5, lockstep with bisection | the layers of section 8 at CI bounds; the mutation suite must kill >= 95 %; registers the `(id, name)` pairs of the blocks reserved in `mutants.rs` (W1-DP 0..=99, W1-MEM 100..=199, W1-FLOW 200..=299, W1-XALU/W1-XMEM 300..=399, W2-XCORE 400..=499, W2-PEEP 500..=599, W2-DRV/W1-CACHE 600..=699, comparator 700..=799, native 800..=899) and checks every registered id has a site |
| W3-NINT | 3 | `native/backend.rs`, `native/link.rs`, `native/difftest.rs` | W2-NDP, W2-NMEM, W2-DRV | `impl Backend for NativeBackend` wrapping `rt::Jit`, link slots, `EXIT_STEPPED`/`EXIT_SLOW` reporting, exit sweep, T3 against T2 against T0 | L0 to L5 with the backend switched; exit sweep per trace; `Machine: Send` assertion |
| W4-SEAM | 4 | `machine/mod.rs`, `machine/fastrun.rs`, `machine/idle.rs` (hunks of 11.4 only), `sb/run.rs` (the one `absorb_pending` line of `drain_stale`) | W3-LOOP, W3-VERIF, W3-NINT, and PR 689 + PR 687 merged or rebased over | the four hunks; a grep proving `drop_stale_code` is the only pop; TD-22 split only if separately agreed | all `machine` tests with `sb.enabled` false and true; `run_sb_full` equals `run` with the seam on |
| W4-BENCH | 4 | `crates/nord-tools/src/bin/arm_sb_ab.rs` | W4-SEAM, W1-CNT | the single A/B of gates G2 and G3: interleaved runs, thread cycles, `LoadPercentage` recorded, median [min..max], tier counters | parser and printer tests; refuses to report a verdict when host load is above 25 % (prints "indicative") |
| W4-DOC | 4 | `docs/research/arm0-sb-results.md` plus hunks in `docs/{DECISIONS,ROADMAP,TECH_DEBT,ARCHITECTURE,LEGAL}.md`, `docs/specs/{README,arm0-runtime}.md`, `.beans/ns4-lle2*` | W4-BENCH | results note (Checked, Sources, Question, Implications, Follow-ups), one DECISIONS entry (the SHARC track's design note also plans D260: whichever track documents second takes the next free number after fetching origin, and the two entries cross-link; main's highest is D259 with D257 and D258 unused) superseding the deferral of D245/D250 for the tiers that pass their gates, D129 and D20/D128 wording, ROADMAP and bean checkboxes | none |

Parallelism: wave 1 has 11 independent work packages; wave 2 five; wave 3 three; wave 4 three. The native work packages (W1-NCORE, W2-NDP, W2-NMEM, W3-NINT) only need the frozen contract and can run beside the T2 packages.

## 13. Gates

Numbers use thread CPU time or deterministic counters. The A/B protocol is the repository's: 7 interleaved runs, one warm-up discarded, median [min..max], a gain counts only when the min..max ranges do not overlap, host `LoadPercentage` sampled before each run, valid below 25 %, otherwise reported as indicative (the SHARC spike host was 77 to 100 % loaded throughout, so a quiet window must be arranged).

| Gate | When | Pass | Stop result and consequence |
|---|---|---|---|
| G0 counters | after W1-CNT, W1-IDLE; no tier needed | (a) `probe_executed / executed` per phase; (b) fixed tick cost = tick time - executed insns x 5.3 ns, per phase; (c) executed insns per tick at the 24 kHz framing; (d) dynamic Thumb and plain-step share; (e) budget calibration if a delay-loop reference exists. Rules: if (a) >= 30 % in idle and key phases, idle v2 stage 1 is the first thing switched on; if (a) < 30 %, the 45 to 48 us is elsewhere and W3-LOOP waits for a diagnosis (rule K5); if (a) < 30 % and (b) < 10 us and the true budget <= 200 k, tiers T2 and T3 are built but stay off (nothing to gain); if (d) Thumb > 3 %, open a Thumb front end follow-up | tiers off by default; idle v2 only |
| G1 exactness | blocks every other gate and every default-on decision | L0 >= 1 M words per family and L1 >= 2,000 programs locally with zero divergence; L2 identical at 100/150/200/300/456 k for T2 on, off, `HOT = 0`, cap 3, idle v2 per tick (1,900 ticks); L3 lockstep clean over boot + 6 phases; L5 kill rate >= 95 % | an unexplained divergence not root-caused inside its work package reverts that family to `GENERIC`; more than 3 reverted families, or a reverted family above 1 % of dynamic instructions, stops T2 and T3 (T1 plus idle v2 remain) |
| G2 tier 2 | after W4-SEAM, deterministic counters first, then one A/B | coverage >= 85 % of guest insns in traces and `GENERIC` <= 8 % on the load phase; >= 15 guest insns per trace entry (today 7.8 per block); tier-1 slice share <= 15 %; A/B heavy ticks T2 >= 1.35x T1, ranges not overlapping | below 1.25x: T2 stays off by default and exists as fallback and oracle only; between 1.25x and 1.35x: stays off pending G3 |
| G3 tier 3 | after G1 and the three-OS spike | exits (`Generic` + slow memory) <= 1 % of dynamic instructions on the load phase; A/B T3 >= 2.5x T1 and >= 1.8x T2; heavy tick under 1 ms at 300 k in the least disturbed run; synchronous compile of the heavy region <= 50 ms or hidden by background mode; `Machine: Send` and `native::available()` fallback tested | exits > 3 %, or < 2.0x T1, or spike failure on any CI OS without a clean cfg fallback: T3 stays off in releases; the ARM side is T1 + idle v2 + (T2 if G2 passed) with a decoupled queue |
| G4 steady state | with the best enabled tier and idle v2 | steady tick thread CPU <= 10.4 us at the 24 kHz framing (0.25 core) and <= 25 us at the 1 ms framing (about 40 % of today's 63 to 72 us) | fail with every tier: ARM keeps the decoupled producer thread with a lead; this locks the hybrid for the ARM half and the number is handed to the SHARC verdict |
| G5 system fit | with the SHARC track's current per-core numbers | ARM thread <= 0.5 core steady and <= 1.0 core in the heavy stretch at the calibrated budget; ARM + SHARC <= 16 - 2 cores (DAW headroom) | the combined figure, not ARM alone, decides whether exact real-time LLE is attempted; ARM never lifts a SHARC NO-GO |
| Stop rule | after G3 | one design review; default-on stays blocked until D246 Gate 3 and the D22 render_hash re-baseline rules are met | |

## 14. Open risks

| # | Risk | Mitigation |
|---|---|---|
| R1 | Judge disagreement: the score ranking puts ARM-SB first, but all three per-judge picks for `arm` were ARM-X. | This note follows the ranking and absorbs ARM-X's native path (T3); if G2 fails and G3 passes, the product tier is ARM-X's approach on ARM-SB's IR. |
| R2 | Every speed figure is a model calibrated on one measured number (17 cycles). The predecode step was predicted 5x and measured 2.0x. | Counters first (G0), one A/B per gate, no claim before G2/G3. |
| R3 | Unit risk: "thread cycles" may be nominal-clock cycles, the mission quotes 4.5 GHz boost while 17 cycles at 5.3 ns implies about 3.2 to 3.4 GHz. Ratios assume model cycles and measured cycles share a unit; if not, the design is up to 27 % better than modelled. | Report ns, not cycles, in G2 and G3. |
| R4 | The 45 to 48 us fixed idle-tick cost is unexplained; the probe is the hypothesis. At 24 kHz it is the dominant ARM cost. | G0 (a), (b); idle v2 stage 1 is exact regardless of the cause. |
| R5 | The instruction budget per ms is ASSUMED (200 k, bound 300 to 456 k), and nord-lle ticks at 24 kHz with 200 k instructions per tick (4.8 G virtual insns/s); both conventions cannot be true. | G0 (c), (e); the cost model is given at both framings. |
| R6 | Tier 2 floor: register file in memory and one shared indirect jump bound it near 1.4x likely; the 30 % conditional soft-float code is dependency-chain bound. | T3 is the answer; T2 is not sold as real time. |
| R7 | Executable memory and unsafe: macOS hardened runtime, DAW sandboxes, `JITModule` auto-`Send` unproven, `return_call_indirect` unproven on Windows and macOS arm64. | Feature off by default, runtime `available()`, automatic fallback to T2, spike in W1-NCORE, dispatcher-return fallback for chaining. |
| R8 | Page-granular device flags: the tick page also holds data; every access to it exits native code at 150+ cycles. | Gate G3 cap of 1 %, demotion rule, follow-up F5 (helper call without register spill). |
| R9 | `cranelift-jit` whole-module reset can leak pages on Windows (reported in the ARM-X proposal, not verified here). | One long-lived module, 32 MB cap, follow-up F7. |
| R10 | Three implementations of one semantics (interpreter handlers, T2 arms, T3 lowering). A change to `arm_dp.rs` without the matching arm is a silent divergence. | L0 and L1 run in CI at bounded cost; mutation tests; a doc rule in the spec and in `code-structure.md`'s refactor clause. |
| R11 | Seam conflicts: PR 689 and PR 687 edit `machine/mod.rs`; PR 689 edits `fastrun.rs`; decision numbers D252 to D259 are already used on main or on branches. | All new code in new files; W4-SEAM last, at most 30 lines; fetch before numbering. |
| R12 | Goldens are for OS v1.66 and one program; PR 687 (OS 1.68 layouts) may change the digests. | L2 re-derives the goldens with T1 on the current main before comparing. |
| R13 | The 1.9 % plain steps are not split into Thumb, hooked pages and misaligned pc; their cost (46 cycles) is ASSUMED. | G0 (d); if Thumb > 3 %, add a Thumb front end. |
| R14 | Policy: D245 and D250 defer translation; LEGAL section 3 says decompression is the only transformation; D129 says the app remembers only a path. | One DECISIONS entry in W4-DOC: translation happens in memory on the user's own image, nothing persisted, no firmware bytes, hot addresses or generated code in the repository; the SHARC track uses the same wording. |
| R15 | LoopWatch misses loops whose period does not pass a dispatch point; trace-build latency can add tens of microseconds on a cold tick. | A missed loop costs speed only; build caps per `run()` call. |
| R16 | The three bench claims "4.8 G virtual insns/s" and "ARM share of host" depend on SHARC sizing (8.1 measured, 17.3 projected budget periods vs 16 cores). | G5 uses the SHARC track's current numbers at decision time. |
| R17 | Parallel work packages cannot run the benchmarks, so a slow lowering or a wrong fusion is found only at G1/G2. | `Uop::n`, `Exit::done` and the `fuse_pair`/`expand` pair are checked by tests per package; the trace builder, driver and executors meet first in W3-VERIF, so the first integration failures land there, in one place. |
| R18 | Mutant ids, DECISIONS numbers and `docs/specs/README.md` rows are the cross-track merge points (SHARC D260, ROADMAP). | Mutant id blocks reserved per package; DECISIONS and ROADMAP edits only in W4-DOC, the README row is added by W1-DOC (wave 1) and only updated by W4-DOC (wave 4), never in parallel. |
| R19 | A native `PRED` or side exit emitted before the lowerer knows it is `Unsupported` leaves a dangling block (Cranelift verifier failure or a wrong exit). | Lowerers return `Unsupported` before emitting; `PRED` is all-or-nothing; `build_function` returns `None` for an unterminated function; the verifier runs in every native test. |

## Implications

* The ARM side is a latency and fixed-cost problem first and a throughput problem second. Idle v2 stage 1 is exact by construction, needs no tier, and should ship before anything else is switched on.
* Tier 2 alone is a modest gain (1.1x to 1.9x modelled) and cannot hold every heavy tick under 1 ms above about 200 k instructions per ms; it earns its place as the portable tier, the executor of `Generic` uops and the oracle of tier 3. Tier 3 is what changes the picture (3.3x likely) and is the only option that removes the 25 to 69 ms producer lead at 300 k to 456 k.
* The bit-exact claim rests on one rule: anything doubtful is the interpreter's own handler. A lowering hole costs speed, so coverage can grow independently per family, and a bug in one family is a one-line revert to `Generic`.
* `nord-arm` stays dependency-free and unsafe-free by default. Merging `sb-native` makes it the first dependency and the first unsafe code of the crate, which needs a DECISIONS entry.
* ARM cannot rescue or sink the SHARC verdict: even tier 3 likely uses 0.13 core steady, 0.3 to 0.7 core in the heavy stretch.

## Follow-ups

* F1 Calibrate the instruction budget per ms (a firmware delay loop with known wall-clock duration, or the PLL setup in the boot loader).
* F2 Measure the dynamic Thumb and hooked-page share of the 1.9 % plain steps; add a Thumb front end only above 3 %.
* F3 Cap the producer's lead outside heavy stretches to 1 to 3 ticks (PR 689 follow-up), so live MIDI is not delayed by the program-load queue.
* F4 Software TLB for T2 memory uops (10 to 20 % on memory ops, ASSUMED) behind its own A/B.
* F5 T3 helper-call slow path for flagged but mapped pages (about 30 cycles instead of 150), at the price of the "calls no Rust" property.
* F6 Zero the low 64 KB only when dirty (`HOOK_W` dirty bit in `SysBus::on_write`, existing file).
* F7 Replace whole-module reset by an RX arena with generation free (shared with the SHARC track's JIT runtime).
* F8 Background compile with a bounded SPSC channel and an install-time word compare.
* F9 Repeat all A/B runs on a quiet host with raised Windows timer resolution; none of the benchmark numbers of this note were produced under the 25 % load rule.
* F10 Decide with the owner whether the engine framing is 1 ms with 200 k instructions or 24 kHz with 8.3 k (R5); every steady-state figure here scales with it.
