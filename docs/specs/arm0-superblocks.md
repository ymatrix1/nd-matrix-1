# ARM0 superblock tiers (`crates/nord-arm/src/machine/sb/`)

Status: **skeleton (S0, commit 1b06822a) plus this spec (W1-DOC), 2026-11-28.** Bean [ns4-arm-sb](../../.beans/ns4-arm-sb--arm0-superblock-tiers-and-exact-idle-skipping-v2.md). Decision: none yet. W4-DOC records one; until it does, the deferral of translation in [D245](../DECISIONS.md) and [D250](../DECISIONS.md) stands and nothing here is on by default. Design, cost model, work packages and risks: [arm0-jit-design](../research/arm0-jit-design.md). Baseline this builds on: [arm0-runtime](arm0-runtime.md), [arm0-rust-runtime](../research/arm0-rust-runtime.md).

Code: `crates/nord-arm/src/machine/sb/` (tier 2, safe Rust; `native/` is tier 3, feature `sb-native`), `crates/nord-arm/src/mem/raw.rs`, `crates/nord-tools/src/bin/arm_sb_bench.rs` (counters, gate G0), `arm_sb_ab.rs` (A/B, gates G2 and G3).

Every speed figure in this spec is a model (`ASSUMED`) until gates G2 and G3 have run. At the skeleton every lowerer and executor family is a stub that returns "not handled", so each instruction becomes a `GENERIC` uop and the stack equals the interpreter by construction (`skeleton_stack_equals_the_interpreter_on_a_synthetic_program`); the work packages of section 11 replace the stubs one file at a time.

## 1. Scope and tiers

| Tier | What | Default | Switch | Role |
|---|---|---|---|---|
| T0 | `Cpu::step` per instruction (`Machine.predecode = false`) | kept | none | the oracle |
| T1 | predecode cache, `run_cached` (blocks of 7.8 instructions, EMPIRICAL) | on | none | baseline and cold code |
| T2 | trace superblocks over a flat `Uop` IR, chained exits, exact idle skipping v2 | off | `SbConfig.enabled`, `NS4_SB=1` | portable fast tier, executor of `GENERIC`, in-process oracle of T3 |
| T3 | Cranelift native code compiled from the same `Sb` | off | cargo feature `sb-native` and `NS4_SB_NATIVE=1` | performance tier |

* Every tier works in place on the same `Cpu` and `SysBus`. Switching converts nothing; at every exit the state equals the interpreter's state after the same number of instructions.
* Fallback ladder: T3 exit or compile failure falls to T2, a missing trace to T1, `predecode = false` is T0. Each step is a configuration switch or a runtime condition (executable memory unavailable, `bus.probing`, Thumb, hooked page, misaligned pc, budget below `n_guest`).
* Runtime switches, all default off, no other feature flag exists: `NS4_SB=1` (tier 2 with all peephole passes, default hotness 24), `NS4_SB_IDLE2=1` (idle v2), `NS4_SB_NATIVE=1`, `NS4_SB_HOT=n`, `NS4_SB_VALIDATE=n`. `SbConfig::on()` (tests, tools) is tier 2 with `hot = 0`.
* No existing code path calls into `sb` until W4-SEAM wires `Machine::run_tier` (at most 30 changed lines in `machine/{mod,fastrun,idle}.rs`, section 6). The skeleton edits existing files only by one `pub mod sb;`, `#[cfg(feature = "sb-native")] mod raw;`, the `pub(super)` visibility of `same_state` and `Machine::probe_loop` in `machine/idle.rs`, and Cargo features.

Not translated:

| Item | Handling |
|---|---|
| Thumb | `bx` and loads into `pc` with bit 0 set exit with `cpu.thumb = true`; T1 and T0 run Thumb. A Thumb front end only if the dynamic Thumb share exceeds 3 % (counter in G0). |
| MMU, TLB, caches, permissions | Not modelled (VA = PA, CP15 is a stub, all pages RWX, ASSUMED by `mem.rs`). Nothing to invalidate on `mcr c7/c8`. If an MMU model is added, section 6 must grow. |
| WFI | 0 encodings in the benchmark images; `mcr p15,0,rd,c7,c0,4` is `GENERIC` and raises `Fault::Wfi` exactly as before. |
| `MUL`, `MLA` | Lowered to `MUL`, `MULS`, `MLA`, `MLAS` (ids 108..=111, WP W2-MUL). |
| Long multiplies, halfword multiplies and the other `SLOW` classes | `GENERIC` (no kind id allocated). |
| Page mapping | The tier never maps a page (I9). |

## 2. Invariants

Every work package obeys these. "Pinned by" names the test or file that holds the rule at the skeleton; rows marked with a package are pinned by that package's tests when it lands.

| # | Rule | Pinned by |
|---|---|---|
| I1 | A trace depends only on code words and folded literal words, never on register or flag values; no speculation. `call_preserving` restoring the `Cpu` cannot invalidate a trace. | `Sb::words`; `the_skeleton_builds_one_generic_end_trace_that_snapshots_its_word`; W1-FLOW (`words` equals memory) |
| I2 | No uop other than `GENERIC*` and `EXIT*` reads or writes `r15`. Pc-dependent constants (`add rd,pc,#x`, `bl` link, literal loads) are folded at build time; every other use of `r15` is `GENERIC`. Enforced twice: each lowerer returns `Lowered::Generic`, and `decode::lower_or_generic` overrules any lowering whose uops still name `r15` (`decode::touches_r15`, which also catches `r15` in an `STM` list and `rd >= 14` of `ldrd`/`strd`). | `a_uop_that_names_r15_is_overruled_into_one_generic` |
| I3 | `done`, `last_pc` and `n_guest` are static; `cpu.insns` changes only at exits. | `straight_line_generic_uops_end_in_a_direct_exit_with_the_static_counts`, `exit_targets_follow_the_exit_kind`; L6 |
| I4 | The flag nibble `f = n<<3 \| z<<2 \| c<<1 \| v` (layout of `Cpu::nzcv`) is authoritative inside the executor; `cpu.n/z/c/v` are written at every exit and around `GENERIC`; `q`, `thumb`, `irq_off`, `fiq_off`, `mode`, `spsr`, `cp15_ctrl` change only inside `GENERIC` and `MSR_C`. | `pack_and_unpack_round_trip_and_touch_only_the_four_flags`, `the_flags_survive_the_nibble_round_trip_through_generic_uops` |
| I5 | Memory effects occur in guest order through the `SysBus` accessors with `bus.pc` set to the accessing instruction, so device models, the poll breaker, probing and auto-mapping behave exactly as in T1. | `SbBus::set_pc`; W1-XMEM (ordered access log on a logging bus) |
| I6 | A store that leaves `stale_code` non-empty ends the trace before the next uop. | `a_store_into_decoded_code_leaves_through_the_early_exit_before_the_next_uop` |
| I7 | A trace is entered only when `left >= n_guest` and none of its pages holds `until` or `stop`. | `run_sb_full_equals_run_on_a_program_with_until_and_stop`; W2-DRV |
| I8 | Anything not lowered is `GENERIC`, executed by the interpreter's own `exec_decoded`. A missing lowering costs speed, never exactness. | `an_unlowered_instruction_becomes_one_generic_uop` |
| I9 | The tier never maps a page: building reads only mapped, unhooked pages. | `nothing_is_built_on_unmapped_hooked_or_misaligned_code_and_nothing_is_mapped` |
| I10 | T3 native code calls no Rust function and exits to the driver for anything outside its fast path. | `native/abi.rs` tests; W2-NMEM |
| I11 | No stale page is lost: pages that go stale while the real `SbState` is taken out are recorded by the placeholder and forwarded before the next drain. The driver drains after every trace run whatever the exit kind, so a chain never continues into a trace that just died. | `pages_that_go_stale_while_the_state_is_out_reach_it_when_it_is_put_back`, `a_placeholder_records_pages_once_and_touches_nothing_else` |

Real-time rule (`.agents/rules/dsp-and-realtime.md`): trace building, compilation, revalidation and every `HashMap` or `Vec` growth of the cache happen in the dispatch path of `Machine::run`, which on the audio thread runs ahead of `process_block`; the engine's `process_block` and `render_frame` never call into this module. A page going stale is rare (program load, SRAM overlay); the only allocation it causes outside the cache is the first push to `SbState::pending`.

`unsafe` is denied in `machine/sb/mod.rs`. Only `native/rt.rs` allows it (code-pointer transmute, the C-ABI call, `unsafe impl Send` of the JIT module). `mem/raw.rs` returns raw pointers, which is safe to do.

## 3. Structures (`uop.rs`, `uop/{k,fused}.rs`, `cache.rs`, `hot.rs`, `mod.rs`)

All indices; raw pointers exist only in `native/` and `mem/raw.rs`.

### 3.1 `Uop`

16 bytes, `repr(C)` (`a_uop_is_16_bytes`): `kind, cond, rd, rn, rm, rs, sh, n: u8`, `imm: u32`, `pc: u32`. `pc` is the guest address of the first covered instruction (written to `bus.pc` before an access, and the fault pc). Lowerers emit unconditional uops; the builder wraps a conditional instruction in a `PRED` (`decode::wrap_pred`) and `pred.rs` merges runs.

Conventions pinned by S0:

| Rule | Detail |
|---|---|
| Early-exit index | A memory uop and `GENERIC` keep the index of their early (`Stale`) exit in `cond`; every control uop (`GUARD`, `GUARD_REG_EQ`, `LOOP_BACK`, `EXIT*`, `GENERIC_END`, fused guards) keeps it in `rs`. `Uop::exit_idx` and `Uop::set_exit_idx` hide the difference. The trace builder allocates early exits; the lowerers cannot (they see no exit table). |
| `n` | Guest instructions the uop covers: 1, 2 when fused, 0 for `PRED` and pure control uops. An instruction that lowers to several uops puts `n = 1` on the first and `n = 0` on the others. Summing `n` over the uops of a path in program order (a `PRED` body included: a failing condition still retires) gives the `done` of the exit that path leaves through. Bookkeeping for validators and the fuser, never an input of an executor (counts come from `Exit::done`, I3). |
| Specialised uops | Non-S specialised uops keep `sh = 0` (the fuser relies on it). |
| Fused uops | `fuse_pair(a, b)` is the only constructor (W2-PEEP decides when to call it), `expand(u)` the only decoder and the definition: executed in order, `bus.pc` set per access, the second uop reading the state the first left. Executors and native lowerers call `expand` or are tested against it. |
| Page numbers | `Sb::pages` holds `addr >> 12`, the unit of `Mem::stale_code`. |
| Registry | `k::family(kind)` is the static table every dispatcher (executor, native codegen) goes through, so no package edits another's file to register itself. An unknown id is `unreachable!`; `every_id_is_either_registered_to_one_family_or_unreachable` enumerates all 256. |

Kind ids (`k::COUNT = 165`, `k::DEFINED = 145`; the gaps 129..=143, 155..=159 and 165..=255 are reserved, not allocated):

| Ids | Family (executor) | Kinds |
|---|---|---|
| 0..=95 | `Dp` (`exec_alu`) | generic data processing, `k::dp(form, opcode, s) = form*32 + opcode*2 + s`; form 0 immediate, 1 register shifted by immediate, 2 register shifted by register; equal to the predecode handler ids (`the_generic_dp_ids_match_the_predecode_handler_ids`) |
| 96..=107 | `Dp` | specialised: `MOV_IMM MOV_REG MVN_IMM ADD_IMM SUB_IMM ADD_REG SUB_REG CMP_IMM CMP_REG TST_IMM TST_REG ORR_IMM`, same semantics as the generic id they replace |
| 108..=111 | `Dp` | `MUL MULS MLA MLAS` = `k::mul(acc, s)` = `108 + acc*2 + s`: `rd = rm * rs (+ rn)` modulo 2^32, N and Z from the result when S, C and V untouched (ARMv5), as `Cpu::arm_mul` cases 0 and 1; one instruction like any other (`Exit::done`) |
| 112..=128 | `Mem` (`exec_mem`) | `LDR_IMM LDRB_IMM STR_IMM STRB_IMM LDR_REG LDRB_REG STR_REG STRB_REG` (112..=119), `LDRH LDRSH LDRSB STRH LDRD STRD` (120..=125), `LDM STM` (126, 127), `LDR_ABS` (128) |
| 144..=154 | `Flow` (`exec_flow`) | `PRED GUARD GUARD_REG_EQ LOOP_BACK EXIT EXIT_REG EXIT_REG_ARM GENERIC GENERIC_END MRS MSR_C` |
| 160..=164 | `Fused` (`exec_flow`) | `CMP_GUARD_I CMP_GUARD_R TST_GUARD_I MOV_PAIR LDR_PAIR` |

Operand layout:

| Kind | rd | rn | rm | rs | sh | imm |
|---|---|---|---|---|---|---|
| DP generic | dest | first operand | Rm (forms 1, 2) | Rs (form 2) | form 1: `type<<5 \| amount (0..31)`, special encodings (LSR/ASR #0 = 32, ROR #0 = RRX) resolved by `alu::shift(.., imm = true)`; form 2: `type<<5`; form 0: bit 0 = 1 when rot == 0 (shifter carry = C) | form 0: rotated immediate |
| DP specialised | dest | first | Rm | | | immediate |
| `MUL MULS MLA MLAS` | Rd (instruction bits 19..16) | accumulator (bits 15..12; 0 for `MUL`) | Rm (bits 3..0) | Rs (bits 11..8) | | |
| `LDR/LDRB/STR/STRB` (`_IMM`, `_REG`; a register offset shifted by `ROR #0` = RRX stays `GENERIC`) | Rd | Rn | Rm (reg offset) | flags: b0 U (add), b1 P (pre), b2 WB (`!P \|\| W`) | reg offset: `type<<5 \| amount` | 12-bit offset |
| `LDRH LDRSH LDRSB STRH LDRD STRD` | Rd | Rn | Rm | flags as above plus b3 REG | | 8-bit offset |
| `LDM`, `STM` (S bit and empty list stay `GENERIC`) | | Rn | | flags | popcount | register list |
| `LDR_ABS` (literal on an unfolded page) | Rd | | | | | absolute address |
| `PRED` | | | | | | skip count (`cond` = condition 0..=13; skips the next `imm` uops when it fails) |
| `GUARD` | | | | exit index | | 0 (leaves through exit `rs` when condition `cond` holds) |
| `GUARD_REG_EQ` (inlined return) | | Rn | | exit index | | expected value (takes the exit when `r[rn] != imm`) |
| `LOOP_BACK` | | | | exit index | | (re-enters at uop 0 while `left >= acc + n_guest`, else takes the exit) |
| `EXIT`, `EXIT_REG` (`bx`, interwork), `EXIT_REG_ARM` (`mov pc,rm`, no interwork) | | Rn | | exit index | | |
| `GENERIC` (non-ender), `GENERIC_END` (ender, then a `Dynamic` exit) | | | | exit index (END) | | instruction word; `GENERIC` keeps its early exit in `cond` |
| `MRS`, `MSR_C` (`cpsr_c` immediate or register, T never written, `mode_changes` counted) | Rd | Rm | | b0 = SPSR (`MRS`) or immediate form | | rotated immediate |

Fused uops (`uop/fused.rs`) are each defined as their unfused expansion, `n = 2`, the second instruction at `pc + 4`: `CMP_GUARD_I` = `CMP_IMM ; GUARD`, `CMP_GUARD_R` = `CMP_REG ; GUARD`, `TST_GUARD_I` = `TST_IMM ; GUARD` (the guard leaves through exit `rs` on condition `cond`; its `done` and `last_pc` already count the branch), `MOV_PAIR` = two `MOV_IMM` (`rd`, `rn` = second destination, `imm = a | b << 16`, each below 2^16), `LDR_PAIR` = two `LDR_IMM` with a shared base `rn` (`rd`, `rm` = second destination, no write-back, `imm = a | b << 16`, each below 2^12).

### 3.2 `Exit` and `Sb`

`Exit { kind, reg, done: u16, target, last_pc, link: SbId, ic_pc: [u32; 2], ic_id: [SbId; 2], hits }`. `Sb { entry, pages: [u32; 4], n_pages, n_guest: u16, uops, exits, words: [(address, word)], live, enters, native }` (`Sb::from_parts` derives `pages` from `words` and enforces `MAX_UOPS = 128`, `MAX_EXITS = 64`, `MAX_PAGES = 4`).

* `done`: guest instructions executed from the start of the current iteration through the last one retired on this path. `last_pc`: its address (becomes `bus.pc`). `n_guest`: the most instructions on any path, one full loop iteration included. `words`: every guest word the trace was built from, folded literals included (the basis of revalidation).
* A trace is single entry with side exits and at most one loop-back to its head.

| `ExitKind` | Target | Taken when | Driver after the run |
|---|---|---|---|
| `Direct` | constant `target` | a trace that ends at a static branch or at a cap, the side of a `GUARD`, `EXIT` | patch `link`; chain if the successor is live and passes the entry tests |
| `Indirect` | `r[reg]`: bit 0 sets `thumb` and the target is `& !1`, else `& !3` | `EXIT_REG` (`bx`) | two-way inline cache, patched after the first lookup |
| `IndirectArm` | `r[reg] & !3` | `EXIT_REG_ARM` (`mov pc,rm`) | as `Indirect` |
| `Dynamic` | `cpu.r[15]` after the instruction | `GENERIC_END`, a faulting ender included (the instruction is counted; the fault stays in `cpu.fault`) | re-dispatch |
| `Fault` | reserved, never emitted | | the driver takes `cpu.fault` after every run whatever the exit kind |
| `Stale` | static: the pc after the instruction | a store left `stale_code` non-empty (memory uops and `GENERIC`) | break, drain, re-dispatch; never chain |
| `LoopBack` | static: the entry | the budget cannot cover another iteration, or `watch` is set (the executor then leaves after every lap) | patch `link`, chain; T1 finishes the budget |
| `Watch` | the dispatch point | reserved: no builder or executor emits it (the watch leaves through the `LoopBack` exit, see section 5); the driver still treats it as a watch return | `watch.hit = true`, return to the idle driver |

The exit is applied by whoever runs the trace (executor or native epilogue) before it returns: `cpu.insns += done` (plus the loop accumulator), `*left -= done`, `cpu.r[15] = target`, `bus.pc = last_pc`, the flag nibble written back. `RunOut { exit, target }` only names the exit and the new pc. Two reserved `RunOut::exit` values are never indexes of `Sb::exits`: `backend::EXIT_SLOW = u16::MAX` (nothing ran and nothing was applied, e.g. `bus.probing`: the driver runs the trace with `exec::run_sb`) and `backend::EXIT_STEPPED = u16::MAX - 1` (native code left to the interpreter, which stepped one instruction; applied, no link is patched, the driver re-dispatches at `target`).

### 3.3 Cache, hotness, state

| Item | Role |
|---|---|
| `SbCache` (`cache.rs`) | arena `Vec<Sb>` (a dead trace keeps its slot so ids stay stable but frees its uops, exits and words); lookup by entry pc; `covering(page)`; `is_live` (a link or inline-cache entry to a dead trace is never followed); `volatile` pages; `uops_total` against `uop_cap` (flush all on overflow, done by the driver). The skeleton uses hash maps; W1-CACHE replaces them with `at: per page Box<[u32; 1024]>` (word to `SbId + 1`) and `by_page: Vec<Vec<SbId>>`, after which `lookup`, `covering` and `on_stale` allocate nothing past a page's first insert, and adds a per-page change counter (the volatile rule) and an `epoch`. |
| `HotTable` (`hot.rs`) | dispatch counts per pc (`bump`, `reset`, `mark_failed`, `failed`); W1-CACHE moves them to per page `Box<[u16; 1024]>`. |
| `SbState` (`mod.rs`) | `cfg, cache, hot, stats, watch, backend: Option<Box<dyn Backend + Send>>`, `placeholder`, `pending: Vec<u32>`. `Clone` keeps `cfg` and drops everything else, so a cloned `Machine` starts cold but correct. `SbState` and `Machine` are `Send` (`the_state_and_the_machine_stay_send`). |
| Placeholder protocol (`stale.rs`) | The driver takes the real state out with `SbState::take_real(&mut m.sb)`, which leaves a placeholder. While it is out, `on_stale` on the placeholder only records the page (deduplicated) in `pending`; `absorb_pending` forwards the records to the real state and `put_back` ends the borrow. A stale page therefore reaches the real state even when a nested `Machine::run` (`call_preserving`, `inject_irq`, a hook) pops `stale_code`. The placeholder is inert (`cfg.enabled` false), so a nested run stays in T1, which is exact. |

Constants, all `ASSUMED`: chosen from the 7.8-instruction block length (EMPIRICAL, PR 689 counters) and the 2 KB hot routine of the heavy stretch; the first counters run (gate G0) fixes them, `NS4_SB_*` overrides them.

| Constant | Value | Meaning |
|---|---|---|
| `hot` | 24 | dispatches of a pc before a trace is built there |
| `slice` | 32 | instructions per T1 stretch between dispatch points |
| `max_uops`, `max_guards`, `max_pages` | 128, 6, 4 | trace caps (never above `MAX_UOPS`, `MAX_EXITS`, `MAX_PAGES`) |
| `inline_depth` | 2 | nested `BL` calls inlined |
| `builds_per_run` | 4 | traces built per `Machine::run` call at most (about 512 lowered instructions) |
| `uop_cap` | 4,000,000 | uops (64 MB) in the cache before it is flushed |
| `volatile_after` | 4 | a page whose code words really changed more often than this becomes T1 only |
| `validate_cases` | 0 | random states per new trace (L4); the real-firmware CI job uses 8 |

## 4. Execution

### 4.1 Driver (`run.rs`)

`run_fast(m, sb, limit, until, stop) -> Option<Fault>` replaces the `run_cached` call of `Machine::run` with the same contract: it runs at least one instruction or returns. `limit` is an absolute `cpu.insns` value.

```
loop:
  drain_stale(m, sb)                                   // forward stale_code to sb.on_stale, then m.drop_stale_code()
  pc = cpu.r[15]; left = limit - cpu.insns
  if left == 0 or pc & 3 != 0 or cpu.thumb or pc == until or pc == stop or HOOK_X on pc's page: return None
  if id = cache.lookup(pc) and sb[id].n_guest <= left and no page of sb[id] holds until/stop:      // I7
      run the chain: backend.run(id) if compiled, else exec::run_sb        // T3 else T2
        apply the exit; after ANY exit kind: take cpu.fault -> return it; drain_stale
        EXIT_SLOW -> run the same trace with exec::run_sb;  EXIT_STEPPED -> re-dispatch at target, never link
        Stale -> re-dispatch;  Watch -> watch.hit = true; return None
        Direct/Indirect/IndirectArm: patch link or inline cache, bump hits; continue the chain only into a live
          successor that fits the budget, passes the until/stop test and is aligned, ARM state, unhooked
      continue
  if hot.bump(pc) >= cfg.hot and builds < cfg.builds_per_run and not hot.failed(pc):
      trace::build -> mark_code(pages), cache.insert, optional backend.compile, optional validate_trace; builds += 1; continue
      on None: hot.mark_failed(pc)
  r = m.run_cached(min(limit, cpu.insns + cfg.slice), until, stop); drain_stale; if r is Some(fault): return r
```

* Hotness is sampled at dispatch points (T1 slice boundaries and trace exits). A loop of period P is sampled at the pcs that are multiples of `slice` modulo P, so every loop is found; a trace may start mid-block, which is harmless because traces depend only on code words (I1).
* No taken-branch profile is collected: static layout (backward taken, forward not taken). A guard exit whose `hits` exceed half the trace's `enters` after 64 entries triggers one rebuild with the prediction flipped; other hot side exits grow side traces through `hot.bump` on their target.
* `run_sb_full(m, sb, max, until, stop_pc)` copies the outer loop of `Machine::run` (hooks, `until`/`stop`, budget, plain step) and calls `run_fast` where `run` calls `run_cached`. It is the entry for tests and tools before the seam and stays as the equivalence oracle after it.

### 4.2 Trace formation (`trace.rs`, `lower_*.rs`)

| Step | Rule |
|---|---|
| Per instruction | `decode`; control flow (`Insn::is_ender`) goes to `lower_flow`, everything else to `decode::lower_or_generic` (static registry: `lower_dp`, `lower_mem`, the predicate wrap, the `GENERIC` fallback). |
| Follow | Unconditional `B` and `BL` (`BL` emits `MOV_IMM r14, pc + 4`, inline depth up to 2; at depth > 0 the callee's `bx lr` becomes `GUARD_REG_EQ r14 == return pc` and the trace continues); the predicted side of `Bcc` (the other side is a `GUARD` exit). |
| End | An indirect branch (`EXIT_REG`, `EXIT_REG_ARM`), a generic ender (`GENERIC_END`), an unmapped or `HOOK_X` page, or a cap (128 uops, 6 guards, 4 pages). |
| Loops | Returning to the start pc emits `LOOP_BACK`; returning to another pc already in the trace ends with an `EXIT` (no inner loops). Conditional `BL` is `GENERIC`. |
| Literals | A pc-relative load folds to `MOV_IMM` when the literal's page is mapped, unhooked and added to the page list; the word goes to `words`. |
| Faulting forms | Odd `Rd` for `ldrd`/`strd` and `r14` pairs stay `GENERIC`. So does a register offset shifted by `ROR #0` (RRX): it reads the carry flag, which the memory arms do not have, and `exec_mem::reg_offset` panics on it. |
| After each instruction | The builder allocates the early exit of every store uop and every `GENERIC` (static `done`, `last_pc`) and stores its index with `Uop::set_exit_idx`. |
| Passes | `fuse::run`, `flagopt::run`, `pred::run`, in this order, each gated by its `BuildOpts` flag. Each preserves state against the unpassed trace and never changes the `done` or `last_pc` of an exit. |

### 4.3 Executor (`exec.rs`, `exec_alu.rs`, `exec_mem.rs`, `exec_flow.rs`)

One `loop` over the uops, dispatching on `k::family`, with `cpu.r`, the flag nibble `f` and the index `i` in locals.

* Data processing arms are monomorphised per `(OP, S, FORM)` const generics like the interpreter's `h_dp`; the semantics are those of `Cpu::dp_exec` (logic ops keep V and take the shifter carry, arithmetic uses `alu::add_c`, `adc`/`sbc`/`rsc` use the pre-instruction C).
* Memory arms are generic over `SbBus` and call the same `read32/write32/...` as the interpreter's handlers: word loads rotate by `8 * (a & 3)`, stores use `a & !3`, halfword `a & !1`, write-back is skipped on a load with `rn == rd`. `bus.set_pc(u.pc)` precedes every access (I5). After a store, `bus.stale()` ends the trace through the uop's early exit (I6).
* `PRED` skips the next `imm` uops when its condition fails. `GENERIC` (a non-ender: cannot fault or jump) writes the nibble to `cpu`, calls `exec_pure`, reloads the nibble and leaves through its early exit when a store made code stale. `GENERIC_END` calls `exec_decoded`, reloads the nibble and leaves through its `Dynamic` exit; a fault stays in `cpu.fault`.
* `LOOP_BACK` adds the iteration length to a local accumulator and re-enters while `left >= acc + n_guest`.

### 4.4 Budget

Entry requires `left >= n_guest` (I7), so no trace stops mid-way; the last stretch of a budget runs in T1 with the exact limit (`run_cached` already honours it). A conditional instruction whose condition fails still counts one instruction (static `done`).

## 5. Idle skipping v2 (`idle.rs`, `loopwatch.rs`)

The existing rule (`machine/idle.rs`, [arm0-runtime](arm0-runtime.md)) is exact: from state `S0`, if after `L` instructions the registers, CPSR, SPSR and CP15 control equal `S0` and no store changed memory, no device page was touched, no hook ran and no unmapped page was mapped, `k * L` instructions are skipped. Idle v2 changes only the schedule and the comparison granularity. It reuses `same_state` and `Machine::probe_loop`, never a copy.

| Stage | Change | Exactness argument | Owner |
|---|---|---|---|
| 1 (T1 only, no trace needed) | `probe_idle_v2`: call `same_state` only when `r[15] == start.r[15]`; hoist the `HOOK_X` and `stop_pc` checks to page changes; after a failed probe grow the next slice geometrically past `MAX_SLICE` (up to 2^20, never past the tick end) | `same_state` compares `r`, which contains `r[15]`, so equal pc is a necessary condition and the first matching instant is unchanged; the skip itself is untouched; a larger slice only delays the next probe | W1-IDLE |
| 2 (needs T2) | `LoopWatch`: arm when one trace self-links four times in a row or the same entry pc is dispatched twice; take `S0` at that dispatch point with `bus.probing = true`; compare at dispatch points and `LOOP_BACK` where `pc == S0.pc`; skip `k * L`; disarm with back-off on impurity or after `MAX_PERIOD` instructions | the instants inspected are a subset of the instants the interpreter inspects, so a missed loop costs speed only; the probe runs through the same instrumented `SysBus` accessors | W3-LOOP |

Watch contract between W2-DRV and W3-LOOP: while `SbState::watch.armed` the driver passes `watch = true` to the executor and, at every dispatch point with `pc == watch.pc`, sets `watch.hit` and returns; `run_sb` leaves a `LOOP_BACK` through its own exit (kind `LoopBack`, target the loop head) after every lap in that case, never through an exit of kind `Watch` (decided with the interpreter as oracle, `run/watch_tests.rs`). The dispatch point at `watch.pc` counts only after at least one instruction of the current `run_fast` call: arming at pc `P` and calling at `P` runs the loop once instead of returning at once; `run_sb_full` ends the call with `Stop::Budget` when it sees `watch.hit`. W3-LOOP reads and writes only `SbState::watch` and the `Machine`. `run_tick_idle_v2` is `Machine::run_tick_idle` built from the stage-1 and stage-2 functions (it must not call `run_tick_idle`: the seam forwards that function here, so it would recurse).

Equivalence to the current path is proven per tick, not only per phase: the per-tick state hash of idle v2 equals the current `run_tick_idle` for all 1,900 ticks at five budgets (gate G1). Zeroing the 64 KB of low memory only when dirty (about 1.5 to 2 us per tick, ASSUMED) needs a `HOOK_W` dirty flag inside `SysBus::on_write`, an existing file; it is follow-up F6 for the seam owner, not in this work.

## 6. Events, interrupts, overlays, invalidation

| Event | Handling |
|---|---|
| Timer interrupt | Not asynchronous: `inject_irq` runs the AINTC handler as a nested `call_preserving` at tick boundaries, so no trace is ever interrupted. I1 makes the saved and restored `Cpu` irrelevant to traces. The handler (SRAM) is traced like any code. |
| Budget end, `until`, `stop_pc` | I7; the remainder runs in T1; the `stop`/`until` page test is an exact compare, never a bloom filter. |
| Code hooks (stubs, null-call guard) | Hooked pages are never traced and a trace never follows into one. `add_hook` sets `HOOK_X` through `add_flag`, which queues a `HOOK_C` page in `stale_code`; `on_stale` drops every trace on a page that now has `HOOK_X`. The 1.9 % plain steps (EMPIRICAL, PR 689) stay on `Cpu::step`. |
| Self-modifying code, SRAM overlays, program loads | Every page a trace reads is marked `HOOK_C` (`Mem::mark_code`). A store to it takes the `Mem` slow path, queues the page and clears the flag; the store uop ends the trace (I6); the driver drains the queue into `SbState::on_stale(page, mem)`, which revalidates: it compares each covering trace's `words` with memory, keeps unchanged traces and re-marks `HOOK_C`, drops changed ones, resets links to them and releases their backend code. A `HOOK_X` page drops all. A page whose words changed more than `volatile_after` times is flagged volatile: no traces, T1 only. Stores into the executing trace, the next word or a folded literal's page are covered because the store uop exits before the next uop. The skeleton's `on_stale` is the always-correct subset (drop every trace that read the page). |
| `drop_stale_code` is the single pop | `Machine::drop_stale_code` (called by `run`, `run_cached`, `step_cached`) is the only production pop of `stale_code` (VERIFIED by reading at the design commit; a grep is obligation W4-SEAM). The seam forwards each popped page to `self.sb.on_stale`; the placeholder protocol of section 3.3 covers pops made while the driver holds the state. |
| Device pages | T2 goes through `SysBus` (exact by construction). T3 exits to the interpreter for any flagged page (I10). |
| `Machine::clone`, threads | `SbState::clone` is empty. `Machine: Send` is asserted at compile time (`the_state_and_the_machine_stay_send`); the T3 JIT module is wrapped in a documented single-owner `unsafe impl Send`. |
| Arena reallocation (T3) | Only Rust code maps pages, never native code; `NativeCtl.arena` is refreshed before every native entry. |

The seam (W4-SEAM, after PR 689 and PR 687 are merged or this branch is rebased over them; the only package that edits `machine/{mod,fastrun,idle}.rs`):

```rust
// machine/mod.rs: field, constructor, run() call site, and the method
pub struct Machine { ..., pub sb: sb::SbState }               // Machine::new(): sb: sb::SbState::default()
// in run():  if let Some(f) = self.run_tier(limit, until, stop) { ... }   instead of  self.run_cached(limit, until, stop)
fn run_tier(&mut self, limit: u64, until: u32, stop: u32) -> Option<Fault> {
    if !self.sb.cfg.enabled { return self.run_cached(limit, until, stop); }
    let mut sb = sb::SbState::take_real(&mut self.sb);
    let r = sb::run::run_fast(self, &mut sb, limit, until, stop);
    sb.put_back(&mut self.sb, &mut self.bus.mem); r }
// machine/fastrun.rs, drop_stale_code(): after self.code.drop_page(p):  self.sb.on_stale(p, &mut self.bus.mem);
// machine/idle.rs, run_tick_idle(): first line:  if self.sb.cfg.idle_v2 { take_real; loopwatch::run_tick_idle_v2; put_back; return }
// sb/run.rs drain_stale: first call sb.absorb_pending(&mut m.sb, &mut m.bus.mem)   // the one W4-SEAM line in a W2-DRV file
```

## 7. Exactness contract

Oracle: `Cpu::step` with `predecode = false` on a `SysBus`. The tiers are second and third implementations of the same semantics; they never replace the interpreter, and every doubt resolves to `GENERIC`. A bug in a family costs a revert of that family to `GENERIC`, not a wrong result.

Observable state compared everywhere (`testkit/snapshot.rs`; each field has a test that mutates it and demands a named diff): `r0..r15`, `n z c v q`, `thumb`, `irq_off`, `fiq_off`, `mode`, `mode_changes`, `spsr`, `cp15_ctrl`, `insns`, the fault, `bus.pc`, `Mem::state_hash`, `device_reads`, `poll_hits`, messages (path, comm, words, pc), `auto_mapped(_count)`, `tick_t`, I2C, panel and UART state, and the `Stop` reason.

| Layer | What | Where | Bound |
|---|---|---|---|
| L0 per uop | random state x random word per family (all 96 DP ids x conditions, shift amounts 0, 1, 2, 30, 31, 32, 33, 63, 64, 255, 256, every P/U/W combination, `rn == rd`, unaligned, page-end addresses, `ldm/stm` over a page end, all 16 NZCV), traces of 1 to 3 instructions, against `Cpu::step` on a logging bus (registers, flags, ordered bus access log, `bus.pc`) | `difftest.rs` (`l0_case`, `l0_sweep`) | CI: `NS4_FUZZ_CASES` (default 60) scaling 20,000 words per family; local gate 1,000,000 words per family |
| L1 programs, three-way | plain `Cpu::step`, T1, T1+T2 (T3 when available) on the generator of `testgen.rs` extended with loops, `bl`/`bx lr` pairs, conditional runs, literal loads, stores into the own, next and literal page, hooks added between slices, Thumb entry, undefined words; budgets 1..7 and 1..400 so the budget expires at every uop position; `hot` in {0, 1, 24}; trace caps {1, 2, 3, 128}; `until` and `stop` inside traces | `difftest.rs`, `testgen.rs` | CI 60 programs; local 2,000 or more |
| L2 golden firmware | boot state hash, six phase message digests and state hashes at 200 k, whole-run digests at 150 k, 200 k, 300 k, 456 k instructions per tick (values: design note section 8); at 100 k to 456 k with T2 on, off, `hot = 0`, cap 3, idle v1 against v2 per tick. Goldens are re-derived with T1 on the current main first (OS v1.68 support may change them). Local only, skipped when `target/fw_unpacked` is absent, nothing printed (D20, D128) | `arm-sb-bench` | zero differences |
| L3 lockstep | chunks of 100,000 instructions on boot and every tick: clone the `Machine`, run the reference on the clone, compare; on mismatch restore and halve the chunk to one instruction and report pc, word, register and flag deltas | `shadow.rs` | clean over boot and 6 phases at 5 budgets |
| L4 translation validation | with `validate_cases = K`, every newly built trace runs against the interpreter on K random states over a private mini machine holding copies of the trace's pages; registers biased to 0, 1, `0x7FFFFFFF`, `0x80000000`, `0xFFFFFFFF`, mapped RAM, device pages and unmapped pages; a failing trace is dropped, its entry pc added to the deny list (`hot.mark_failed`) and `shadow_mismatches` bumped (loud by design) | `validate.rs` | K states per trace (CI real-firmware job 8) |
| L5 mutation | at least 40 mutants (flip a carry rule, drop a rotate, swap write-back order, wrong condition polarity, off-by-one `done`, wrong `last_pc`), each a `cfg(test)` branch `if mutants::is_active(N)`; each must be killed by L0 or L1 at the default case count; the comparator has mutants too | `mutants.rs` | kill rate 95 % or more |
| L6 invariants | sum of `done` equals the `cpu.insns` delta; no trace entered with `left < n_guest`; `live` and link consistency after invalidation; every kind id has an arm; every `Exit.done <= n_guest` | tests in `exec.rs`, `cache.rs`, `run.rs` | always on |

Mutant id blocks (reserved so the sites are written in parallel without a clash; W3-VERIF registers the `(id, name)` pairs and checks every id has a site): 0..=99 W1-DP, 100..=199 W1-MEM, 200..=299 W1-FLOW, 300..=399 W1-XALU and W1-XMEM, 400..=499 W2-XCORE, 500..=599 W2-PEEP, 600..=699 W2-DRV and W1-CACHE, 700..=799 comparator, 800..=849 W2-NDP, 850..=899 W2-NMEM.

T3 adds: its output is compared against the T2 executor on the same `Sb` (an in-process oracle at native speed) before it is compared with the interpreter; an exit sweep (for each trace and each k below its length, force a budget exit after k instructions and compare); the same L0 to L5 with the backend switched. Where `native::available()` is false the native tests assert the fallback instead of skipping.

Three implementations of one semantics exist (interpreter handlers, T2 arms, T3 lowering). A change to the semantics of `cpu/arm_dp.rs`, `arm_mem.rs`, `alu.rs` or `arm_cached.rs` lands in the same PR as the matching `exec_alu.rs` / `exec_mem.rs` arm and native lowering, or the family reverts to `GENERIC`; L0 and L1 in CI catch drift at bounded cost.

State of the layers at the skeleton: L0 and L1 run on the stub stack (`difftest.rs`, `testgen.rs`, `testkit/`); `shadow.rs` (L3) and `validate.rs` (L4) fail closed (they report that nothing was compared); `mutants.rs` has the switch and the report but no mutants.

## 8. Tier 3 native (feature `sb-native`, `native/`)

* **Input and output.** One `Sb` becomes one `unsafe extern "C" fn(cpu: *mut Cpu, ctl: *mut NativeCtl) -> u32` returning the exit index. Registers `r0..r14` read by the trace and the four flags are Cranelift `Variable`s loaded at entry; the dirty subset and the defined flags are stored at every exit; `cpu.r[15]` and `cpu.insns` (`+= done`) are written by the exit epilogue. Field offsets come from `offset_of!(Cpu, ..)`; tests assert them against a live `Cpu`.
* **Memory.** `NativeCtl { table: *const u32, arena: *mut u8, limit: u64, exit: u32, target: u32, pad: [u32; 2] }`. A load is allowed when `((e & MASK_LOAD) - 1) <u MASK_IDX` with `e = table[a >> 12]` (mapped, no `HOOK_R`), a store when the same holds for `MASK_STORE` (no `HOOK_W`, no `HOOK_C`); `MASK_LOAD = 0x8FFF_FFFF`, `MASK_STORE = 0x5FFF_FFFF`, `MASK_IDX = 0x0FFF_FFFF`, asserted equal to `mem::HOOK_*` by a test. Host address: `arena - 4096 + ((e & 0x0FFF_FFFF) << 12) + (a & 0xFFF)`. An access wider than the page remainder, or any failed check, is an exit `Generic` at that pc with the state exact before the instruction; the driver then runs one `Cpu::step` through `SysBus`, drains stale and re-dispatches. Native code never runs while `bus.probing` is set.
* **Flags and conditions.** Computed from SSA operands; a compare followed by a guard fuses to one `icmp`; flag definitions overwritten later in the trace are dead (the rule of `flagopt.rs`).
* **Exits.** `Indirect` and `IndirectArm` exits are native (`ctl.target = r[reg] & !3`, or `& !1` with the `OFF_THUMB` byte store for `bx`). `Dynamic` stays driver-side because the `GENERIC_END` before it is never native. `LoopBack` is a native back-edge with a `cpu.insns + n_guest <= ctl.limit` check. Chaining v1 returns to the driver (about 20 cycles); v2 (spike-gated) patches a link slot and uses `return_call_indirect` with the Tail convention.
* **Lowerers.** A lowerer returns `Unsupported` before it emits anything. A `PRED` whose body is not entirely lowerable is `Unsupported` itself (its pc is the first instruction of the body, so the state before it is exact). `codegen::build_function` returns `None` for a uop list that does not end in a terminated block; the trace then stays on T2.
* **Code memory.** One `JITModule` per backend, 32 MB cap; on overflow the module is dropped after the pass boundary and traces are recompiled lazily (follow-up F7). Compilation happens on the dispatch thread (ASSUMED 150 us per trace, about 4 ms for the heavy region); a bounded SPSC background mode is follow-up F8.
* **Unsafe and dependencies.** Cranelift 0.116 is optional and already in the workspace through `nord-sharc` feature `jit`. Merging `sb-native` makes it the first dependency and the first unsafe code of `nord-arm`, which needs a DECISIONS entry (W4-DOC). `native::available()` is a runtime probe whose answer is never persisted (D129); when it is false everything runs on T2, exactly.
* **Spike (W1-NCORE).** Two-trace ping-pong with `return_call_indirect`, executable-memory allocation and `Send` of the module on ubuntu, windows and macos CI (the `sb-native` tests run under `--all-features` on all three). The spike is written and checked with `cargo check`, not run locally on the other two systems; an unproven target keeps `available()` false. At the skeleton `rt::probe` answers false without touching executable memory.

## 9. Cost model (ASSUMED)

Method: rows are cycles per 1,000 guest instructions of the firmware static mix (a proxy for the dynamic mix); the measured baseline is the unit (`ns = 5.3 x cycles / 17`), so no clock frequency enters. The full row tables are in design note section 10. The predecode step was predicted 5x and measured 2.0x, so a bottom-up sum is not trusted until G2 and G3.

| Tier | cycles per instruction, best / likely / worst | against 17 (EMPIRICAL, PR 689) | M insns/s likely |
|---|---|---|---|
| T1 | 17 (least disturbed run) | 1.0x | 189 |
| T2 | 8.95 / 12.1 / 15.2 | 1.9x / 1.4x / 1.1x | 265 |
| T3 | 3.6 / 5.1 / 9.2 | 4.7x / 3.3x / 1.9x | 625 |

Heavy tick load in host cores (budget x ns / 1e6; under 1.0 fits in 1 ms; the instruction budget per ms is not measured, 200 k is ASSUMED and 300 k to 456 k is the bound): T2 likely 0.75 / 1.13 / 1.72 at 200 k / 300 k / 456 k; T3 likely 0.32 / 0.48 / 0.73, times 1.5 on a loaded host (0.48 / 0.72 / 1.10). Steady state at 24 kHz framing (3,384 executed instructions per tick): T1 0.43 core, T2 0.31, T3 0.13, each plus the fixed per-tick cost (45 to 48 us today at the 1 ms framing, VERIFIED arithmetic on EMPIRICAL data). Consequence: T2 alone does not hold every heavy tick under 1 ms at 300 k or more; T3 does in the likely case; idle v2 stage 1 is the first lever because it needs no tier.

## 10. Gates

Numbers use thread CPU time or deterministic counters. The A/B protocol is the repository's: 7 interleaved runs, one warm-up discarded, median [min..max], a gain counts only when the min..max ranges do not overlap, host `LoadPercentage` sampled before each run and valid below 25 %, otherwise reported as indicative (an unknown load is not below the limit; `arm-sb-ab` refuses to print a verdict above it). No work package runs a wall-clock benchmark; wall-clock runs happen at G0, G2 and G3 only, after the counters.

| Gate | When | Pass | On failure | Status |
|---|---|---|---|---|
| G0 counters | after W1-CNT and W1-IDLE; no tier needed | (a) `probe_executed / executed` per phase; (b) fixed tick cost = tick time minus executed instructions x 5.3 ns, per phase; (c) executed instructions per tick at the 24 kHz framing; (d) dynamic Thumb and plain-step share; (e) budget calibration if a delay-loop reference exists. If (a) >= 30 % in idle and key phases, idle v2 stage 1 is the first thing switched on; if (a) < 30 %, the fixed cost is elsewhere and W3-LOOP waits for a diagnosis; if (a) < 30 %, (b) < 10 us and the true budget <= 200 k, T2 and T3 are built but stay off; if (d) Thumb > 3 %, open a Thumb front end follow-up | tiers off by default; idle v2 only | not run |
| G1 exactness | blocks every other gate and every default-on decision | L0 >= 1 M words per family and L1 >= 2,000 programs locally with zero divergence; L2 identical at 100/150/200/300/456 k for T2 on, off, `hot = 0`, cap 3, idle v2 per tick (1,900 ticks); L3 clean over boot and 6 phases; L5 kill rate >= 95 % | an unexplained divergence not root-caused inside its package reverts that family to `GENERIC`; more than 3 reverted families, or one above 1 % of dynamic instructions, stops T2 and T3 (T1 plus idle v2 remain) | not run |
| G2 tier 2 | after W4-SEAM; counters first, then one A/B | coverage >= 85 % of guest instructions in traces and `GENERIC` <= 8 % on the load phase; >= 15 guest instructions per trace entry (T1: 7.8 per block); T1-slice share <= 15 %; A/B heavy ticks T2 >= 1.35x T1, ranges not overlapping | below 1.25x: T2 stays off by default (fallback and oracle only); 1.25x to 1.35x: stays off pending G3 | not run |
| G3 tier 3 | after G1 and the three-OS spike | exits (`Generic` plus slow memory) <= 1 % of dynamic instructions on the load phase; A/B T3 >= 2.5x T1 and >= 1.8x T2; heavy tick under 1 ms at 300 k in the least disturbed run; synchronous compile of the heavy region <= 50 ms or hidden by background mode; `Machine: Send` and the `available()` fallback tested | exits > 3 %, or < 2.0x T1, or spike failure on any CI OS without a clean cfg fallback: T3 stays off in releases; ARM is T1 + idle v2 + (T2 if G2 passed) with a decoupled queue | not run |
| G4 steady state | with the best enabled tier and idle v2 | steady tick thread CPU <= 10.4 us at the 24 kHz framing (0.25 core) and <= 25 us at the 1 ms framing (about 40 % of today's 63 to 72 us) | fail with every tier: ARM keeps the decoupled producer thread with a lead; the number goes to the SHARC verdict | not run |
| G5 system fit | with the SHARC track's current per-core numbers | ARM thread <= 0.5 core steady and <= 1.0 core in the heavy stretch at the calibrated budget; ARM + SHARC <= 16 - 2 cores (DAW headroom) | the combined figure decides whether exact real-time LLE is attempted; ARM never lifts a SHARC NO-GO | not run |
| Stop rule | after G3 | one design review; default-on stays blocked until D246 Gate 3 and the D22 `render_hash` re-baseline rules are met | | |

## 11. Work packages and ownership

Every file starts with `OWNER: <package>`; after S0 a file belongs to one package. Frozen files (`uop.rs`, `uop/{k,fused}.rs`, `config.rs`, `stats.rs`, `decode.rs`, `bus.rs`, `backend.rs`, `mod.rs`) change only by amendment. Waves are merge waves: wave 1 has 11 independent packages, wave 2 five, wave 3 three, wave 4 three. Nothing is pushed or merged by a work package. The bean tracks status.

| Package | Wave | Owns | Depends on |
|---|---|---|---|
| S0 | 0 | the skeleton: everything frozen above and a stub for every other file | none |
| W1-DP, W1-MEM | 1 | `lower_dp.rs`, `lower_mem.rs` | S0 |
| W1-FLOW | 1 | `trace.rs`, `lower_flow.rs` | S0 |
| W1-XALU, W1-XMEM | 1 | `exec_alu.rs`, `exec_mem.rs` | S0 |
| W1-CACHE | 1 | `cache.rs`, `hot.rs`, `stale.rs` | S0 |
| W1-IDLE | 1 | `idle.rs` | S0 |
| W1-VGEN | 1 | `testkit.rs`, `testkit/snapshot.rs`, `testgen.rs` | S0 |
| W1-CNT | 1 | `crates/nord-tools/src/bin/arm_sb_bench.rs` | S0 |
| W1-NCORE | 1 | `native/{mod,abi,codegen,emit,rt}.rs`, `mem/raw.rs` | S0 |
| W1-DOC | 1 | `docs/specs/arm0-superblocks.md`, the bean, one row of `docs/specs/README.md` | S0 |
| W2-XCORE | 2 | `exec.rs`, `exec_flow.rs` | W1-XALU, W1-XMEM |
| W2-DRV | 2 | `run.rs` | W1-FLOW, W1-DP, W1-MEM, W1-CACHE, W2-XCORE |
| W2-PEEP | 2 | `fuse.rs`, `pred.rs`, `flagopt.rs` | W1-FLOW, W2-XCORE |
| W2-NDP | 2 | `native/lower_dp.rs` | W1-NCORE, W1-XALU |
| W2-NMEM | 2 | `native/lower_mem.rs`, `native/lower_flow.rs` | W1-NCORE, W1-XMEM, W2-XCORE |
| W3-LOOP | 3 | `loopwatch.rs` | W2-DRV, W1-IDLE |
| W3-VERIF | 3 | `difftest.rs`, `mutants.rs`, `shadow.rs`, `validate.rs` | W2-DRV, W2-XCORE, W2-PEEP, W1-VGEN |
| W3-NINT | 3 | `native/backend.rs`, `native/link.rs`, `native/difftest.rs` | W2-NDP, W2-NMEM, W2-DRV |
| W4-SEAM | 4 | hunks of section 6 in `machine/{mod,fastrun,idle}.rs`, one line of `sb/run.rs` | W3-LOOP, W3-VERIF, W3-NINT, PR 689 and PR 687 merged or rebased over |
| W4-BENCH | 4 | `crates/nord-tools/src/bin/arm_sb_ab.rs` | W4-SEAM, W1-CNT |
| W4-DOC | 4 | `docs/research/arm0-sb-results.md`, hunks in `docs/{DECISIONS,ROADMAP,TECH_DEBT,ARCHITECTURE,LEGAL}.md` and `docs/specs/{README,arm0-runtime}.md`, the bean's checkboxes | W4-BENCH |

Cross-track merge points: mutant id blocks (section 7), DECISIONS numbers (the SHARC track plans D260; whichever track documents second takes the next free number after fetching origin and the two entries cross-link) and the `docs/specs/README.md` row (added by W1-DOC, updated only by W4-DOC). DECISIONS and ROADMAP are edited only in W4-DOC.

## 12. Provenance, policy and open items

| Item | Tag | Note |
|---|---|---|
| 17 thread cycles per instruction (5.3 ns), 124 to 129 M/s loaded | EMPIRICAL | PR 689 note, least disturbed and loaded median |
| 7.8 instructions per T1 block, 1.9 % plain `Cpu::step` instructions, 3.3 to 3.4 k instructions and 63 to 72 us per idle or key tick | EMPIRICAL | PR 689 note, [arm0-rust-runtime](../research/arm0-rust-runtime.md) |
| Fixed idle-tick cost 45 to 48 us | VERIFIED arithmetic | 63 to 66 us minus 3,384 x 5.3 ns |
| All tier 2 and tier 3 cycle rows, the 1.35 / 1.7 spreads, the trace length (about 25 guest instructions per entry), compile costs | ASSUMED | confirmed or replaced at G2 and G3 |
| The constants of section 3.3 | ASSUMED | G0 fixes them |
| Gate thresholds of section 10 | ASSUMED | decision thresholds, not measurements |
| Instruction budget per ms (200 k; bound 300 k to 456 k) | ASSUMED | G0 (e), follow-up F1 |
| `abi.rs` masks and offsets | VERIFIED when the tests pass | derived from `mem.rs` and `Cpu`, no firmware involved |

Policy:

* Translation happens in memory on the user's own image; nothing is persisted (D129), and no firmware bytes, hot addresses, firmware-derived tables or generated code enter the repository (D20, D128). The tests use synthetic in-memory code; real-firmware layers (L2, L3, the benchmark bins) are local only and skip cleanly when the image is absent. `tools/fw-derived-baseline.txt` does not change. W4-DOC records one DECISIONS entry with the same wording as the SHARC track (D245 and D250 defer translation; LEGAL section 3 says decompression is the only transformation).
* Default builds keep `nord-arm` free of dependencies and `unsafe`; both exist only behind `sb-native`.

Open items:

* **Long multiplies.** `MUL`, `MLA` and their S forms have kind ids 108..=111 (W2-MUL: `lower_mul.rs`, executor arm `exec_alu::exec_mul_with`, differential against `Cpu::step` in `lower_mul/diff.rs`). `UMULL`, `UMLAL`, `SMULL`, `SMLAL` (two result registers, flags from a 64-bit value) and the ARMv5E halfword multiplies stay `GENERIC`: they have no id and no executor arm. W2-NDP can lower the four 32-bit ids natively.
* **Framing conflict (R5, follow-up F10).** `nord-lle` runs `TICK_INSNS = 200_000` per 24 kHz tick (4.8 G virtual instructions per second, against an AM1705 bound of 0.3 to 0.456 G); both framings cannot be true, and every steady-state figure scales with the one chosen. G0 (c) and (e) measure it; the cost model is given at both.
* **Goldens (R12).** The L2 goldens are for OS v1.66 and one program; PR 687 (v1.68 layouts) may change them, so they are re-derived on T1 first.
* **Conflicts (R11).** PR 689 and PR 687 edit `machine/mod.rs`; PR 689 edits `fastrun.rs`. All new code is in new files and W4-SEAM is last.
* **Files over 400 lines (TD-22).** `cpu/predecode.rs`, `mem.rs`, `machine/fastrun.rs` are not split by this work unless separately agreed; new files stay under about 400 lines and a package may split its own file with `mod x;` declared inside it.
* **Risks and follow-ups.** R1 to R19 and F1 to F10 of the design note stand; the ones that decide a gate are R2 (every speed figure is a model), R4 (the idle-tick fixed cost is explained only by hypothesis), R7 (executable memory and unsafe on three OSes), R8 (page-granular device flags exit native code at 150 or more cycles; G3 caps it at 1 %), R10 (three implementations of one semantics) and R17 (a slow lowering or wrong fusion is found only at G1 and G2, because parallel packages run no benchmark).
