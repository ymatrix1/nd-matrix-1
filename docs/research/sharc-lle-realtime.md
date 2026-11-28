# Whole-Pass AOT and multi-core SHARC pass benchmark at 48 kHz: verdict NO-GO

Checked: 2026-11-24 (bounded experiment: same day)
Sources: ORG/PNO Core1/Core2 SRAM images (local-only, `target/sram/{ORG,PNO,FX}/`, produced from `.reference/` as in `org-system-run.md`); ORG Core2 messages `docs/research/data/org-system-load-core2.script` flattened to pass 0 plus `--msg 0:0x3e021=1`; `crates/nord-sharc/src/core/{aot_whole,aot_mem,aot_emit,aot_lower,aot_hot,aot_glue}.rs`, `runner/pass_sched.rs`, `sharc-aot --whole-pass`, `sharc_jit_diff --aot-pcs/--aot-top/--bench`, `crates/nord-tools/src/bin/lle_sharc_bench.rs`; baselines `sharc-aot-multi-block.md` (PR #466 / D118), `pno-jit-arm-profile.md`; bean `ns4-lle3`.
Question: can Whole-Pass AOT (`sharc-aot` generates Rust, `rustc -O` compiles it into the binary) with native lowering of memory, SIMD, register-move and shift instructions, run on a 1-thread-per-core spin-barrier scheduler, execute every SHARC core of the Nord Stage 4 inside the real-time budget of 46.08 us per pass (2 frames at 43 402.34 Hz, ASSUMED in `org-system-run.md`)?

## Result in one paragraph

**NO-GO on this prototype.** The whole-pass AOT is bit-exact (ORG Core2: 57 blocks, boot 100 000 steps + 300 passes = 1 854 395 steps matched 1:1, full state and SRAM compared every 20 000 steps) and faster than the interpreter, but only by x1.3 to x1.4 per pass. The slowest measured core (ORG Core2, 6 280 steps per pass) needs **~370 us per pass with AOT, 8.0x the budget** (repeat runs 8.0 to 9.8x; the interpreter needs 12 to 13x). PNO Core2 (4 266 steps) needs 7.2 to 7.9x on the interpreter. Real time needs 6 280 steps / 46.08 us = **136 Msteps/s per core**; the AOT delivers 10 to 17 Msteps/s. The fast-f32 mode is no faster than exact mode (explained below). Closing the gap needs roughly a further 8x on the hottest core, which the structure of the current lowering cannot reach without the changes under "What would close it". A follow-up gap analysis and two more lowering rounds (section "Gap analysis and lowering rounds") moved the best ORG Core2 case only from x8.0 to x7.0; the curve flattens far above x2.

**Bounded redesign experiment (section "Bounded redesign experiment", later the same day): ORG Core2 x7.2 (min of 10 runs, no load check, a best case) to x4.6 of the budget (fast-f32 x4.0), still bit-exact in exact mode; it stopped at the pre-set checkpoint (x3.5) because that was not reached. Lowering v2 stages A and B (sections below) then took ORG Core2 to x3.5 (median of 7 interleaved runs, band 158.9 to 164.4 us) and the fast-f32 tier to x2.93 (min of 7, not diffed 1:1 against the interpreter). x2 in exact mode stays out of reach, for the reason in "Verdict after stage B".** PNO Core2 had no AOT module and was the slowest benchmarked core at x6.7; the section "PNO Core2 AOT module" (2026-11-27) builds it (x2.9 on a loaded host) and ORG Core2 is the slowest again.

## 1. Whole-Pass AOT prototype

| Item | Result |
|---|---|
| Block discovery | pcs executed by the real pass (`--aot-pcs`, 4 301 pcs for ORG Core2) cut into blocks at flow instructions plus their 2 delay slots, DO, stack ops and idle; batches never start inside delay slots |
| Native lowering | `DirectMem`, `DagMem` (pre/post-modify, immediate offsets), SIMD twin writes and broadcast loads, `Move(Other/RegSwap)`, `ShiftImm`, `Multifunction`, immediate loads, via `MemXfer` (`core/aot_mem.rs`) |
| Generated module | ORG Core2: 57 blocks, 210 chunk functions of up to 24 items, 3 637 native memory/move sites, 466 generic `aot_step` fallback sites, ~1.1 MB of Rust (never committed: derived from firmware, D20/D128) |
| Exactness | unit test `xfer_equals_the_interpreter_on_random_states` (about 20 instruction shapes, scalar and SIMD, 23 seeds each, full register file and memory compared) and the real pass diff: `sharc_jit_diff ... --aot-top 999 --chunk 20000` printed `AOT DIFF OK: 57 blocks, boot 100000 steps + 300 passes (1854395 steps) matched 1:1`. Run-time coverage 85.4% of audio-pass steps inside blocks, 3.4% of them native compute (`pe`) |
| Per-pass gain | `--bench`: interpreter 0.851 ms/pass (7.38 Msteps/s), AOT 0.634 ms/pass (9.91 Msteps/s), **x1.34**; inside the blocks 6 370 steps took 896 us interpreted vs 546 us AOT (x1.64, ~100 ns per step: far from native speed) |

Compile time is the practical limit (EMPIRICAL, this host, 32 logical cores): inlining the whole `step` body at all 4 103 sites ran > 50 CPU-minutes and was aborted; one function per block with 3 600 inlined transfers took 12.5 min; chunking into 24-item functions with out-of-line `MemXfer` handling builds the `nord-sharc` crate with the module in about 2 min 15 s. Five images would add ~10 min of `rustc` per release build, so AOT builds use a separate target dir (`CARGO_TARGET_DIR=target/aot`) and ordinary builds stay firmware-free.

## 2. Multi-core benchmark

`lle_sharc_bench` runs one OS thread per core. The coordinator publishes a tick number in one atomic, each worker spins on it, runs one pass and counts in on a second atomic, the coordinator spins until all have counted in. No mutex, no channel, no allocation in the loop (`runner/pass_sched.rs`). Passes: 16 warm-up + 300 timed, ARM messages delivered before the pass they name. ORG Core2 uses the 912 flattened ARM messages; PNO cores have no message script yet (idle-state passes).

Host: Windows 11, 32 logical cores, `--release`. Latency is the wall time of the pass only (message delivery excluded). Values come from one run; repeats varied by about +-15%. Two earlier runs on a busy host were 2x slower and are discarded.

Exact 40-bit mode, 3 cores concurrently (PNO c1, PNO c2, ORG c2):

| Core | Engine | Steps/pass | Mean us | p50 us | p99 us | Worst us | Mean / 46.08 us |
|---|---|---:|---:|---:|---:|---:|---:|
| PNO Core1 | interpreter | 1 214 | 74.0 | 71.9 | 112.8 | 117.4 | x1.6 |
| PNO Core2 | interpreter | 4 266 | 332.9 | 319.3 | 520.0 | 584.7 | x7.2 |
| ORG Core2 | AOT | 6 280 | 367.6 | 360.6 | 557.3 | 662.9 | x8.0 |
| tick (slowest core + barrier) | | | 373.8 | 361.9 | 582.4 | 664.4 | x8.1 |

Total CPU per tick 774.5 us = 16.8 budget periods of one host core. The same three cores, all interpreter: ORG Core2 470.9 us (x10.2), PNO Core2 364.9 us, PNO Core1 84.3 us, tick 483.2 us, total CPU 920 us (20.0).

ORG Core2 alone (300 passes each): interpreter 585 / 553 us; AOT 450 / 437 / 410 / 370 us.

Fast-f32 mode (`--tier approx`, module `ORG2F`), 3 cores: ORG Core2 mean 468.8 us, p99 996 us, tick 483 us, total CPU 921.6 us (20.0); alone 463.7 to 471.2 us. **No faster than exact mode.** The approximate kernels only replace the native `pe` compute batches, which are 3.4% of the executed steps; the other 96.6% (memory, move, flow) run the same code in both tiers. Fast-f32 is also not bit-exact, so it would need a listening/null test; it buys nothing until the compute share of the pass is native.

Jitter: p99 is 1.5x to 2.5x the mean for the 300 to 450 us passes and the worst pass is 1.8x the mean, so a real-time budget needs headroom above the mean, not only below it.

### Not measured (blockers, listed on purpose)

| Core | Why |
|---|---|
| ORG Core1, FX Core1/Core2 | the `org` and `pno` presets fault at boot on these images (`unimplemented at 0x1286f4` for FX Core2, `0x121182` for ORG Core1 with the PNO preset): their entry pcs, gates and message scripts are not in `live_presets.rs`. Needs their own presets (follow-up) |
| SY1SY2, SMP | interrupt-driven (`live_sy`, `live_smp`), no poll loop: `pass_sched` runs poll-loop passes only. Needs the `SyModel` ISR driver as a job type (follow-up). Their step counts per audio interrupt come from `sy-smp-audio.md`, not from this benchmark |
| 128/256/512-frame buffers | a buffer of N frames is N/2 passes, so the per-pass budget is unchanged (512 frames = 256 passes in 11.8 ms). Larger buffers only absorb jitter, they do not lower the throughput needed |

## 3. Risks found

| Risk | Finding |
|---|---|
| Compile time | 12.5 min to > 50 CPU-min for naive emission; ~2 min per image with chunking; cannot go into the default build or CI |
| Dynamic SRAM code invalidation | SRAM code can be rewritten at run time. Block entry memcmp-checks the code snapshot (`aot_code_matches`), and the native store path invalidates decode-cache slots only inside the SRAM code extent. **Known hole:** self-modifying code *inside* a running block is not detected until the next entry. Not seen in the ORG/PNO passes (the 1:1 diff would fail), unproven for SY/SMP overlays |
| Computed jumps | `JUMP (Md,Ix)`, register `CALL` and `RTS` end the block and return to the dispatcher; delayed flows still run through `Core::step`. 466 of 4 103 sites stay generic (`aot_step`), mostly flows, DO setup and conditional compute |
| Dispatch cost | each block entry does a binary search, statistics and a code memcmp; the 300-pass diff run counted 25 126 block calls, 8 650 misses and 11 699 bails (about 29 bails per pass) that fall back to the interpreter |
| Fast-path guard | the native memory path needs no peripherals, no read watch, no site-hit capture; any of them silently turns the AOT off for the core (`aot_fast_mem`) |
| Mode divergence | fast-f32 is not bit-exact and has no listening test |

## What would close it (not built here)

Real time needs about 8x more than the AOT has, at ~100 ns per AOT step today:

1. Emit each memory/move site as straight-line Rust with constants folded (address mode, register numbers, SIMD flag known at generation time) instead of calling the generic `aot_xfer(&MemXfer)`. Largest item; it also grows compile time.
2. Keep the register file and DAG registers in locals across a whole block (the `Pe` idea of `sharc-aot-multi-block.md`, extended to memory ops), so loads and stores do not round-trip through `Core`.
3. Native `JUMP/CALL/RTS` with a static target table and native `DO` loops, so most bails disappear.
4. Replace the per-entry memcmp with a code-generation counter and the binary search with a direct table.
5. Skip work: idle fast-forward of poll loops, and not running cores whose output is unused.

A combined x8 from items 1 to 4 is a stretch and costs ~10 min of `rustc` per build. Options for `ns4-lle4` and later: (a) treat SHARC LLE as non-real-time (render-ahead/offline) and keep the HLE engines for live play; (b) run items 1 to 3 as a bounded experiment on ORG Core2 with a checkpoint of <= 180 us/pass (x4) before committing Gate 2 to real-time SHARC; (c) more threads do not help: each core already has its own thread and the slowest core sets the tick.

## Gap analysis and lowering rounds (follow-up, same PR)

Method: `lle_sharc_bench --profile` (`core/aot_prof.rs`, `rdtsc` around every fallback step, every block call and every compute field; off by default). ORG Core2, one core, 316 passes (16 warm-up), the shipped module. Profile overhead inflates the pass (about 1.4 to 1.9 M cycles per pass against ~1.0 M unprofiled), so read the shares, not the absolute cycles.

**Why "3.4% native" was misleading.** The 3.4% counted only the `pe` compute batches. The memory, move and shift transfers lowered by `aot_mem.rs` were not counted as native: they are 76.6% of the steps (4 810 of 6 280 per pass). The problem is not coverage, it is the cost of a lowered step.

Where the pass time goes (ORG Core2, exact tier, round 2 build; shares of pass time):

| # | Cause | Steps/pass | Share of time | Note |
|---|---|---:|---:|---|
| 1 | Lowered transfers, memory/move/DAG glue (`aot_xfer` + `aot_tail`), compute excluded | 4 810 | ~37% | about 100 to 150 cycles per step: generic `MemXfer` match, `Core` field traffic, sram bounds checks |
| 2 | Compute fields inside lowered transfers (2 752 per pass): 1 981 on a generated `Pe` kernel, 771 on `exec_compute_op` | 2 752 ops | 29% | **140 to 210 cycles per op for both paths** (includes ~40 cycles of profile overhead): the exact 40-bit float arithmetic itself, not the dispatch, dominates; `Pe` load/store of ~170 bytes per op adds on top |
| 3 | Instructions after a bail-out, run by the interpreter until the next block start (`Move:Other(4a)` 5.3%, `DagMem` 3.1%, `DirectMem` 2.0%, `ComputeBoth` 1.8%, `LoadImm` 1.6%, `Move:Other(15b)` 1.4%, ...) | 876 | 18.3% | all of the "unblocked" steps are mid-block: 29 bails per pass |
| 4 | `lean_pass_aot` loop glue and idle check | | 9.6% | partly profile overhead |
| 5 | Generic `aot_step` inside blocks (`Move:Other(4b)` 1.5%, `DirectMem` 1.4%, `ComputeCond` 0.5%, `Flow:JumpIndirect` 0.7%) | 284 | 4.5% | forms `MemXfer` does not lower (conditional, long-word) and flows |
| 6 | Block entry/exit dispatch (search, guard, code memcmp) | 83 calls | 1.2% | |
| 7 | Entry refused by the guard | 24 | 0.3% | all "hardware loop ends inside block" |

No cost came from computed jumps (33 `JumpIndirect` steps per pass, 0.7%) or from SRAM code invalidation (no "SRAM code changed" refusal in 300 passes).

**The curve (ORG Core2, one core, mean us per pass, best of the repeats, noisy host: repeats vary +-15% and busy-host runs were up to 1.7x slower).** The shipped state was re-verified at the end: `AOT DIFF OK: 57 blocks, ... 1854395 steps matched 1:1`, `--bench` x1.364 (interpreter 0.871 ms, AOT 0.639 ms per pass). Every row except the interpreter was checked 1:1 against the interpreter with `sharc_jit_diff --aot-top 999 --chunk 20000` (boot 100 000 + 300 passes, 1 854 395 steps).

| Build | Lowered share of steps | Best us/pass | x budget (46.08 us) | `--bench` speedup vs interpreter |
|---|---:|---:|---:|---:|
| Interpreter | 0% | 531 to 585 | 11.5 to 12.7 | 1.00 |
| v1: transfers lowered, compute through `exec_compute_op` | 76.6% | 370 | 8.0 | x1.34 |
| v2: compute fields of transfers on generated `Pe` kernels (one out-of-line kernel per distinct op, 110 kernels; inlining one `Pe` per site made `rustc` use 19 GB and was killed) | 76.6% | 322 | 7.0 | x1.38 |
| v3: blocks cut after every guard-changing instruction (795 blocks, no bail-out tail) | 81.8% | 377 | 8.2 | x1.14 |

v3 raised the covered share to 97% of the steps but dispatch grew from 1.2% to 9.2% of the time (each entry searches, checks the guard and memcmps the code), so it was **not faster and was reverted** (not in the PR; the test for it went with it). Two rounds after v1 moved the best case from x8.0 to x7.0. **The curve flattens far above x2**: stopping this line as the orchestrator asked. What the profile says would still be needed is a different design, not a third round: per-site constant-folded code with the register file in locals for a whole block (rows 1 and 3), and compute that is cheaper than ~100 cycles per 40-bit op (row 2), where only the fast-f32 tier can help.

**Fast mode.** Exact and fast-f32 now share one lowered code path: the generated module differs only in the kernel functions' `Kern::Exact` / `Kern::Approx` argument (`aot_pe_both`), chosen at generation time (`--tier pe|approx`). Measured with the shipped v2 build, final re-run on a busy host (interpreter 643 us): ORG Core2 exact 502 / 527 / 488 us against fast-f32 378 / 365 / 378 us (about 25% faster, x7.9 to x8.2 of the budget); an earlier session gave exact 444 / 449 / 502 against fast 350 / 409 / 433 us (10 to 20%). Fast-f32 gains because compute is 29% of the time and the host-`f32` kernel is cheaper than the exact 40-bit one; it stays far from real time. **Not done:** the Fast-vs-Exact error in dB of the audio on a real pass for this build (state error measured 2026-11-27 on silent passes, see `sharc-kernel-speed.md`). The earlier approximate tier measured 112 to 141 dB SNR (window values) on the 8-voice pass, worst measured pass 103.8 dB (`sharc-aot-native-float.md`); `sharc_jit_diff --aot-audio --buf ...` with the approximate module compiled first would reproduce it, but the multi-module build embeds the exact module first. Fast-f32 stays opt-in until that number exists.

### Missing cores: what each needs

| Core | Result here | Smallest fix |
|---|---|---|
| ORG Core1 | boots under the `org` preset but no pass returns to the poll loop (hits the 2 000 000-step cap, idle false) | it is a Core1 section: start it by the P4I interrupt like PNO Core1 (`irq`), with its own idle pc; both need to be read from its listing (not derived here) |
| FX Core1, FX Core2 | the `org`/`pno` presets fault at boot (**fixed in `ns4-lle4`: presets `fx1`/`fx2` and `FxDrive` in `PassJob`, [sharc-audio-bus](sharc-audio-bus.md)**) | `FX1`/`FX2` in `runner/fx_run.rs` already hold entry `0x1211E3`, idle pcs `0x12128B` / `0x12125B`, triggers, devs and the Core1 start by `irq (15, 0x41)`: add `fx1`/`fx2` presets built from them, and call `FxPeer::before_pass` (the Core1 hand-over counter Core2 waits for) as a per-tick hook in `pass_sched` |
| SY1SY2, SMP | not poll-loop passes | a `PassJob` kind that runs the `live_sy` / `live_smp` interrupt pass (`SyModel`) instead of `lean_pass`; presets exist (`sy_preset`, `smp_preset`) |

### Total CPU across the eight cores

Measured (3 cores together, exact mode, v1 AOT build): PNO Core1 74 us, PNO Core2 333 us, ORG Core2 368 us. The other five are not measured. ASSUMED for the projection: ORG Core1 equals ORG Core2 (same organ program, `dsp-image-hunt.md` 5.17), FX Core1/2, SY1SY2 and SMP each equal PNO Core1 (74 us; probably optimistic for FX, which `fx-contexts.md` shows busier). Sum: 74 + 333 + 350 + 350 + 4 x 74 = about 1 400 us per 46.08 us period = **about 30 budget periods of host CPU**. Fraction of real time each host can reach (the slowest core, about 350 us, is a floor on one core's pass time whatever the core count; the packing figure is sum / host cores):

| Host cores for the emulator | Wall us per period | Fraction of real time |
|---:|---:|---:|
| 4 | ~350 (sum / 4 = 350) | ~13% |
| 6 | ~350 (sum / 6 = 234, floored by the slowest core) | ~13% |
| 8 | ~350 (floored by the slowest core) | ~13% |

With every host from 4 cores up the slowest core, not the core count, decides: real time needs it ~7.6x faster even with unlimited cores. Only PNO Core1 (x1.6 measured) is within reach of one core of real time.

## Bounded redesign experiment (ns4-lle3 follow-up 2)

Question: can ORG Core2 get from x7.0 to at most x2 over budget, bit-exact in exact mode, by (1) per-site folded code, (2) the register file in locals, (3) native control flow and (4) less per-step bookkeeping? Checkpoint set beforehand: after the first two steps the best core must be at least x2 faster than the AOT before (about x3.5), else stop.

Method. Host: AMD Ryzen 9 5900XT (16 cores, 512 KB L2 per core, 32 MB L3 per CCD), Windows 11, other sessions building and running at the same time, so a single run varies by +-15% and more. All "min" figures are the minimum mean of 10 runs of `lle_sharc_bench --engine aot` on ORG Core2 (300 passes, 16 warm-up, 912 ARM messages), binaries kept per step and run alternately. Every step was verified `AOT DIFF OK ... 1854395 steps matched 1:1` (boot 100 000 steps + 300 passes, state and SRAM every 20 000 steps) and the unit tests (`cargo test -p nord-sharc --lib`). Tags: EMPIRICAL for every timing.

### What the profile showed first

| Finding | Evidence |
|---|---|
| The 29 bail-outs per pass were not delay slots. Two big blocks (`0x1273c9`, 676 steps, and `0x127cda`, 958 steps) left the native path at step 51 and 824: a `BIT SET IRPTL` (software interrupt) and a mode write, which `aot_sig` correctly treats as guarded state. They put ~760 steps per pass (12%) on the interpreter. | temporary print of block, step, pc and guard fields on the 300-pass diff (12 300 bails); the software-interrupt instruction is a type 18a bit operation at `0x127434` |
| An interpreter step and an AOT step cost about the same at that point (~100 cycles), so removing the bail-outs did not move the time (best of 6: 299 us before and after). | step 0 below |
| The generated module was 512 k x86 instructions (~2.5 MB) for 3 637 transfer sites. A pass runs each site once, so it fetches ~2.2 MB of code against 512 KB of L2. | `rustc --emit asm` instruction count of the module; exe size 4.94 MB with the module, 2.44 MB after step 1 |
| Of the ~123 cycles per compute op on the generated `Pe` kernel, about half was copying the ~170-byte element state in and out of `Pe`. | micro-benchmark: `aot_pe_both` (both elements) 84 ns, `Pe::exec` alone 13 ns per element |

### Steps and measurements

| Step | Change | Min mean us | x budget | `--bench` speedup vs interpreter |
|---|---|---:|---:|---:|
| Before (PR #660 build) | whole-pass AOT as merged | 333.5 | 7.24 | x1.27 to x1.36 |
| 0. Fewer bail-outs (commit `perf(sharc): AOT blocks end at guard-changing instructions`) | a system register write or bit operation ends its block (273 blocks); a store re-checks the guard only after an access outside the SRAM (`Core::aot_slow`); a delayed flow keeps its delay slots in the block. Bails 29 to 0 per pass, interpreted mid-block steps 876 to 256 | ~300 (best of 6, not A/B'd in the same minute) | ~6.5 | x1.32 |
| 1. One handler per transfer shape (commit `perf(sharc): AOT generated code shares one out-of-line handler`) | `hN(core, pc, imm)`: 3 637 sites in 404 handlers; module code ~2.5 MB to ~0.4 MB; build 87 s to 53 s | 251.5 | 5.46 | x1.69 |
| 2. In-place compute (`Pe<'a>` borrows the registers, flags and ASTAT bits of the core instead of copying them) | kernel 123 to 74 cycles per op | 210.9 | 4.58 | x2.05 |
| 2, fast-f32 module (`ORG2F`) | same code, approximate kernels | 184.4 | 4.00 | |

Per core, final build, three cores concurrently (4 runs, mean us, x of 46.08 us): PNO Core1 interpreter 68.5 to 70.1 (x1.49 to x1.52); PNO Core2 interpreter 307 to 352 (x6.67 to x7.65); ORG Core2 AOT 214 to 221 (x4.65 to x4.79); tick (slowest core plus barrier) 311 to 355 us; total CPU per pass 592 to 642 us = 12.8 to 13.9 budget periods. ORG Core2: 6 280 steps in 211 us = 29.8 Msteps/s (18.9 at 333 us); real time needs 136 Msteps/s.

### Why steps 1 and 2 were built this way and registers in locals were not

- The brief named "per-site folded code". The measurement says the opposite layout wins: a pass runs each site once, so per-site inlined code pays instruction fetch and decode for every step, and the merged prototype lost more to code size than folding constants gained. Sharing one handler per shape (immediate as argument) keeps the folded body but fetches it once per shape.
- "Registers in locals" needs a straight-line function that holds the register file across many sites. With shared out-of-line handlers every call is a boundary, so the state lives in memory (the core or a stack struct) anyway, and the layout that could keep registers in machine registers is the large inlined one that lost. The part of the idea that did pay is the element copy per compute op, removed by working in place (step 2). A full locals design was therefore not built.
- Native `DO`, `CALL` and `RTS` were not built: after step 0 no block bails (0 per pass) and the interpreter-run steps are 4.1% of the steps; the remaining flow cost is in dispatch (4.0%) and loop glue (5.8%).

### Where the time goes now (ORG Core2, exact, `--profile` of the final build; shares of pass time, the profile adds overhead)

| Cause | Steps or ops per pass | Share | Note |
|---|---:|---:|---|
| Compute fields on generated `Pe` kernels | 2 147 ops | 17.1% | 74 cycles per op (was 123): the exact 40-bit arithmetic, 13 ns per element in the micro-benchmark |
| Compute fields through `exec_compute_op` | 842 ops | 13.2% | 146 cycles per op: op families `Pe` does not model (shifter, fixed multiplier, other float ops) |
| Lowered transfers without compute | ~4 000 steps | ~35% | transfer handlers, DAG and SRAM access, glue (remainder of the native share) |
| Generic `aot_step` inside blocks | 480 | 9.8% | `Move:Other(4b)` 3.0%, `DirectMem` 2.7%, conditional compute |
| Rest of a block run by the interpreter after a refused entry | 256 | 8.5% | entry refused: hardware loop end inside a block (24 per pass), interrupt event due inside a block (14) |
| Loop glue and idle check | | 5.8% | |
| Block entry and exit | 273 blocks | 4.0% | search, guard, code check |

### Checkpoint and verdict

After the first two steps ORG Core2 is at x4.58, above the x3.5 line, and each step bought less: -25% (shared handlers), -16% (in-place compute), 0% (bail-outs). At this point the estimate was that compute alone is about 2 989 ops x 2 elements x 13 ns = 78 us (x1.7). That estimate is withdrawn: it multiplied a per-element cost by a count of ops that already cover both elements. Stage B measures 2 831 ops x 59 cycles per both-element op = 167 k cycles = about 37 us at 4.5 GHz (x0.8, 25% of the pass). **Stopped as the checkpoint prescribes at that time; the valid argument for x2 not being reachable is in "Verdict after stage B".** The hybrid verdict (HLE for live play, LLE for oracle and offline work, ROADMAP Gate 1b) is unchanged: ORG Core2 moved from x7.2 to x4.6 (x3.5 after stages A and B), but it is still 3.5 times too slow on its own core, and PNO Core2 (x6.7 on the interpreter) would need the same AOT module before it benefits.

What would still change the picture (not done here): fast-f32 as the live tier needs a listening or null test and a cheaper non-compute path (x2.93 after stage B, a min of 7, not a median, and not diffed 1:1); an AOT module for PNO Core2 (its `--aot-pcs` run is not part of this change); modelling the 842 ops per pass that still ran on `exec_compute_op` in `Pe` (done in stage A: 842 to 88 per pass).


## Lowering v2 stages

Design: [sharc-lowering-v2](sharc-lowering-v2.md). Protocol: section 8 there (no cargo/rustc running, 7 interleaved runs per binary after warm-up, medians with min and max; Ryzen 9 5900XT, desktop, no throttling seen). Tags: EMPIRICAL. The bit-exact check after every slice is `AOT DIFF OK: 274 blocks, boot 100000 steps + 300 passes (1854395 steps) matched 1:1`.

### Stage A: coverage of the executed steps (report)

Built as slices on the existing whole-pass path (`aot_pe.rs`, `aot_whole.rs`, `aot_hot.rs`); the separate `aot2/` IR tree was not needed for this coverage and was not started (stage B decides that).

| Slice | Change | ORG Core2 median us (min..max) | x budget |
|---|---|---|---:|
| before (step 2 of the bounded experiment) | | 210.7 (209.5..212.0) | 4.57 |
| 1 | fixed ALU ops `0x0A 0x21 0x22 0x29 0x40..0x42 0x61..0x63` on `Pe` | 205.3 (202.8..214.5), within noise | 4.46 |
| 2, 3 | float ALU (`0x89 0x8A 0xA1 0xB0 0xC5 0xC9 0xD9`) and shifter (shifts, rotates, bit, field) ops on `Pe`, all float multifunction ALU halves | 189.1 (186.8..195.6), outside noise | 4.10 |
| 4 | a block ends at the last instruction of a `DO ... UNTIL LCE` loop and its tail does the loop-back (guard no longer refuses it) | 190.4 (186.0..196.0), within noise; entry misses per pass 11 412 to 4 206 | 4.13 |

Per core, final build (5 runs, three cores together, mean us): PNO Core1 interpreter 63.8 to 67.8 (x1.4 to x1.5); PNO Core2 interpreter 305 to 320 (x6.6 to x6.9); ORG Core2 AOT 190 to 193 once, 207 in a noisy first run (x4.1 to x4.5). Fast-f32 module ORG2F: 163.4 us (x3.55, minimum of 5 runs, not a median; the fast module is not diffed 1:1 against the interpreter, its kernels are approximate).

Coverage now: ops on `exec_compute_op` 842 to 88 per pass (97 cycles per op there, 64 on `Pe`); 481 generic `aot_step` steps remain (7.7% of steps, 10.3% of time: 1.3x the average step, so lowering all of them gains about 2%); 255 steps per pass (9.4% of time) run on the interpreter after the entry guard refuses 14 blocks per pass because an interrupt-controller event is due inside them; those are not a coverage gap but a timing guard.

### Verdict at the end of stage A

Stage A's line was at most x3.7 (120 cycles per step); the measured value is x4.1 (about 135 cycles per step at 4.5 GHz). The rows that stage A could still remove (generic steps about 2%, refused entries up to about 5%) cannot close that gap. What remains is the average cost of an ordinary step: lowered transfers (about 83 cycles each), 64 cycles per compute op, block glue (about 6%) and dispatch (about 4%). That is the target of stage B. An ablation of the per-step bookkeeping in `aot_tail` was attempted and discarded: removing it breaks the loop and delay-slot state so the pass ends early, which makes the timing meaningless.

### Stage B: what the measurements allowed (report)

Method for this stage: read the compiled handlers (`cargo rustc --emit asm` of the module) instead of guessing. A shared transfer handler for a plain `DM` load carried the whole inlined `finish_step` (loop, arithmetic loop, delayed branch), the circular-buffer latch and the interrupt-window close, about 80 cold instructions per handler, and called `write_ureg` out of line for a constant register code. Every kernel carried five inlined interrupt-latch scans for a case (a pending arithmetic exception) that is rare.

| Slice | Change | ORG Core2 median us (min..max, 7 interleaved runs) | x budget |
|---|---|---|---:|
| before (stage A end) | | 189.1 (186.6..197.0) | 4.10 |
| B1 | handler tail: four compares on the hot path, everything else in a cold out-of-line `aot_tail_slow`; `aot_write_ureg` inline for the data and DAG register files | 164.6 (161.4..169.2), outside noise | 3.57 |
| B2 | `aot_note_sticky` out of line (code size) | 160.9 (158.9..164.4), within noise | 3.49 |

1:1 diff after each: `AOT DIFF OK: 274 blocks, boot 100000 steps + 300 passes (1854395 steps) matched 1:1`. Per core, final build (5 runs together, mean us): PNO Core1 interpreter 61.0 to 67.5 (x1.3 to x1.5), PNO Core2 interpreter 299.8 to 310.1 (x6.5 to x6.7), ORG Core2 AOT 160.6 to 163.5 (x3.5). Fast-f32 module ORG2F: 134.9 us (x2.93, minimum of 7 runs, not a median and without a band: a re-run with the protocol was not possible because another session's builds were loading the host). The fast module is not diffed 1:1 against the interpreter (approximate kernels), so this figure carries no exactness claim.

B2 against B1 is inside the noise bands (158.9..164.4 against 161.4..169.2 us): no gain is claimed for B2. The x3.5 reported for the final build is the median of the final binary, not the better of the two slices.

Tried and discarded (no gain or invalid):

| Try | Result |
|---|---|
| `codegen-units = 1` for the bench build | 166.4 (164.7..175.1) against 161.7 (158.3..163.7): no gain, slightly worse |
| removing `aot_tail` bookkeeping, skipping the transfer bodies | the pass state diverges (passes end early or hit the step limit), so the timing means nothing; both ablations dropped |

Why the stage B design (registers and flags in native locals per superblock, dead-flag and lazy-flag elimination) was not built:

- The compute kernels cost 2 831 ops per pass x 59 cycles per both-element op = about 37 us, 25% of the pass. Flags are about 10 of ~250 instructions of a kernel, so lazy flags and dead-flag elimination are worth at most ~3% of that share (under 1% of the pass), well under the noise band. Only a cheaper exact add or multiply moves the compute share (`aot_fast.rs`, another session). The 13 ns per element of the micro-benchmark is a per-element figure and is not compared with the 59 cycles per both-element op here.
- Native locals need straight-line code that holds the register file across many sites, which the earlier experiment showed loses to shared out-of-line handlers (code bytes fetched per pass, 2.5 MB against 0.4 MB). The transfer state (DAG registers, data registers) is already in L1 in the core struct; the transfer cost is instruction count and fetch, which B1 attacked directly.
- What remains by profile (share of pass time): compute kernels 25%, interpreter-run steps 21% (481 generic `aot_step` steps 11%, 255 steps after 14 blocks per pass refused because a timer event is due inside them 10%), dispatch 5%, loop glue 7%, lowered transfers the rest. Each is worth 2 to 5% to the next slice; none reaches the x2.5 line (about 115 us).

### Verdict after stage B

ORG Core2 exact: x7.2 (merged prototype, min of 10 runs without a load check, a best case) to x3.5 (median of 7 interleaved runs of the final build), bit-exact. Fast-f32 tier x2.93 (min of 7, not diffed 1:1). PNO Core2 on the interpreter is the slowest core (x6.5 to x6.7) and has no AOT module; PNO Core1 is the closest (x1.3 to x1.5, interpreter).

Why x2 is not reachable in exact mode, from the stage B numbers only: compute is 25% of the 161 us pass (about 40 us, x0.9). Even if compute cost nothing, the other 75% is about 121 us = x2.6, above x2. The identified rows in that 75% (interpreter-run steps 21%, loop glue 7%, dispatch 5%, transfers the rest) add up to about -15% of the pass at best, which gives x3.0 and not x2. x2.5 would need roughly -28%.

## PNO Core2 AOT module and consistent per-core table (2026-11-27, `ns4-lle3`)

Question: what does the slowest interpreted core (PNO Core2, x6.5 to x7.5) cost with its own whole-pass AOT module, and what do the three benchmarked cores need together after the faster 40-bit kernels (`sharc-kernel-speed.md`)?

### Module and exactness

| Item | Result |
|---|---|
| Census | `sharc_jit_diff target/sram/PNO/core2_sram.bin --pc 0x125c88 --boot 100000 --passes 300 --warmup 16 --dev 0x282c0380=1 --dev 0x3108d008=5 --trigger 0xB00E2 --idle-pc 0x125DDF --msg-base 0 --aot-rank 40:8 --aot-pcs target/lle/pno2.pcs`: 4 069 executed pcs, 4 266 steps per pass. PNO Core2 runs the same loader and the same pc ranges as ORG Core2 (`0x125eb2..0x12712d`, `0x127cdf..0x12869a`, `0x1273ce..0x127a84`), but with no ARM message script it is an idle-state pass (no keys, no strips): 71% of the steps are moves |
| Generation | `sharc-aot IMG --whole-pass pno2.pcs --force 0x125DDF --tier pe` and `--tier approx`; tags `PNO2` / `PNO2F` in `NORD_SHARC_AOT_HOT` next to `ORG2` / `ORG2F`; 625 KB of Rust per tier; the bench binary builds in 1 min 40 s |
| 1:1 diff | `AOT DIFF OK: 265 blocks, boot 100000 steps + 300 passes (1279800 steps) matched 1:1 (strict state + SRAM every 20000 steps); block calls=142989 steps=1298495 misses=4201 bails=0; audio passes: 93.69% of steps covered (4.59% native)`, on the first run. **No lowering coverage gap was hit, so no source code changed**: the module is new generated data (never committed, D20/D128) |
| Not checked | The fast module `PNO2F` is not diffed 1:1 (approximate kernels). A pass with ARM messages (keys, voices) is not covered: no PNO Core2 message script exists. If PNO Core2 under load behaves like ORG Core2 it costs 6 280 steps, not this idle pass |

### Measurement

Protocol: before every round no cargo/rustc/link process was running (0 in all rounds), 2 warm-up runs, then 9 interleaved rounds (`lle_sharc_bench --engine aot|interp`, one core per run, 300 passes + 16 warm-up, mean us of the pass), median with min..max. **The host was not quiet:** other sessions kept it at 74 to 87% CPU load in every round (`Win32_Processor.LoadPercentage`, Ryzen 9 5900XT; no compiler running, so tests and tools). ORG Core2 exact reads x3.9 here against x3.5 on the quiet host of stage B (160.9 us), so absolute figures are about 10% high and the bands are wide. Tags: EMPIRICAL.

| Core | Engine | Steps/pass | Median us (min..max, 9 runs) | x of 46.08 us |
|---|---|---:|---|---:|
| PNO Core2 | interpreter | 4 266 | 346.1 (322.2..397.7) | 7.51 |
| PNO Core2 | AOT exact | 4 266 | 135.1 (125.2..143.2) | 2.93 |
| PNO Core2 | AOT fast f32 (not diffed 1:1) | 4 266 | 119.2 (115.4..140.6) | 2.59 |
| ORG Core2 | interpreter | 6 280 | 468.9 (446.1..695.4) | 10.18 |
| ORG Core2 | AOT exact | 6 280 | 179.5 (167.9..190.5) | 3.90 |
| ORG Core2 | AOT fast f32 (not diffed 1:1) | 6 280 | 148.9 (145.2..170.8) | 3.23 |
| PNO Core1 | interpreter | 1 214 | 60.3 (58.7..122.2) | 1.31 |

PNO Core2 AOT exact against its interpreter: 135.1 against 346.1 us (x2.56), bands far apart: real. Fast against exact on PNO Core2: 119.2 (115.4..140.6) against 135.1 (125.2..143.2), the bands overlap, so **no gain is claimed** for fast f32 on PNO Core2. ORG Core2 fast against exact (148.9 (145.2..170.8) against 179.5 (167.9..190.5)) also overlaps at the edges; an earlier 7-round run gave 158.4 against 197.4 us with the same ordering, so the direction repeats but the size is not pinned.

Three cores at the same time (7 runs per tier, one thread each, spin barrier; median and min..max of the run means):

| Tier | PNO Core1 us | PNO Core2 us | ORG Core2 us | Tick us (slowest + barrier) | Total CPU per pass us |
|---|---|---|---|---|---|
| exact | 75.7 (66.9..96.3) | 137.1 (132.7..198.7) | 180.7 (171.3..244.3) | 183.6 (173.7..257.8) | 395.1 (370.9..538.0) = 8.6 budgets |
| fast f32 | 71.0 (65.3..98.1) | 131.9 (118.0..169.8) | 160.6 (142.7..200.8) | 165.9 (145.6..215.6) | 379.9 (326.1..468.8) = 8.2 budgets |

### Where the PNO Core2 pass goes (`--profile`, one run, shares of the profiled pass)

The profiled pass is 596 199 cycles = 176.5 us wall in the same run, against 135.1 us unprofiled (+31%): the profiler costs one timestamp pair per fallback step, compute op and block call (about 3 000 per pass), so the shares are order-correct but inflated for the categories with many pairs.

| Share of the pass | Time | Steps | Note |
|---|---:|---:|---|
| Compute fields on generated `Pe` kernels | 18.8% | 1 712 ops, 66 cycles/op | exact 40-bit arithmetic plus the timestamp pair |
| Compute fields through `exec_compute_op` | 1.6% | 90 ops, 108 cycles/op | |
| Rest of the native steps (transfers, moves) | 36.9% | 3 047 native steps in all | |
| Generic `aot_step` inside blocks | 14.0% | 475 | `Move:Other(4b)` 4.3% (167 steps: conditional or long-word forms `MemXfer` does not lower), `DirectMem` 3.7%, `DagMem` 1.4%, `Move:Other(4a)` 1.1%, conditional compute 0.7% |
| Interpreter after a refused entry (timer event due inside the block) | 12.7% (+1.3% refused entries) | 255 steps, 14 entries | guard `steps + n > irq.due` |
| Block entry and exit | 6.6% | 477 calls | search, guard, code memcmp |
| Loop glue and idle check | 8.2% | | |

Interpreter-run steps in total: 744 of 4 266 steps (17.4%) take 28% of the profiled time.

Validation against the unprofiled wall clock, **partial**:

1. Profiler overhead: 41 us (176.5 against 135.1 us) over about 3 000 timestamp pairs is about 46 cycles per pair, consistent with two `rdtsc` reads (not measured separately).
2. Compute share, a real A/B: the approximate kernels (`PNO2F`) save 15.9 us (12%, bands overlap) on PNO Core2 and 30.6 us (17%) on ORG Core2, while the profile puts exact compute at 20.4% of the profiled pass. A cheaper kernel does not remove the op, so the A/B is a lower bound and agrees with the profile.
3. **Not validated:** the shares of interpreter-run steps (28%), dispatch (6.6%) and glue (8.2%). No ablation exists for them (removing them changes the pass; see "Tried and discarded" above).

### Step 4: no step above the noise band

Candidates and the most each could give (profile shares, not deflated for the timestamp overhead, so upper bounds):

| Candidate | Upper bound | Why not built |
|---|---:|---|
| Lower the conditional/long-word `T4a/T4b` forms (167 steps) | ~2% | needs condition evaluation in the shared handler plus the SIMD `cond_y` rule; far below the band |
| All generic `aot_step` forms (475 steps) | ~7% | many small shapes, each below the band |
| Run the 255 steps after a refused entry natively | ~8% | needs mid-block entry or an event-split block; it touches interrupt timing, so it needs the diff with the event pattern; below the 14% min..max spread of this host |
| Dispatch (6.6%) and glue (8.2%) | a few % each | more blocks made dispatch worse in v3 (see "Gap analysis and lowering rounds") |

The measured spread of the exact PNO Core2 pass is 125.2..143.2 us (14% of the median) on this loaded host; no candidate alone is above it, so by the stop rule nothing was built, and a change below the band could not be called real here. A quiet host (stage B: 158.9..164.4 us, a 3% band) would make the 7 to 8% candidates measurable: re-measure there first.

### Total host need and verdict

| Set of cores (exact; AOT where a module exists) | Sum of medians us | Budget periods | Slowest core |
|---|---:|---:|---|
| Measured now: PNO Core1 + PNO Core2 + ORG Core2, loaded host | 60.3 + 135.1 + 179.5 = 374.9 | **8.1** | ORG Core2 x3.90 |
| Same in fast f32 (`PNO2F`, `ORG2F`, not diffed 1:1) | 60.3 + 119.2 + 148.9 = 328.4 | 7.1 | ORG Core2 x3.23 |
| Before this change (stage B, interpreter for PNO Core2) | 61 + 305 + 161 = 527 | 11.4 | PNO Core2 x6.6 |
| Projection to 8 cores (ASSUMED: ORG Core1 = ORG Core2; FX1, FX2, SY1SY2, SMP = PNO Core1) | 60.3 + 135.1 + 2 x 179.5 + 4 x 60.3 = 795.6 | **17.3** | ORG Core2 |

- **Total host need across the three benchmarked cores: 8.1 budget periods** of one host core (8.6 measured concurrently, tick 183.6 us), against 11.4 before this change and 17 to 20 for the first prototype; about 10% less on a quiet host.
- **The slowest core is ORG Core2 again** (x3.9 loaded, x3.5 quiet); PNO Core2 in the idle state is x2.9. More host cores do not help: every core already has a thread and the slowest one sets the tick.
- **Verdict unchanged:** pure real-time SHARC LLE is not reached (the slowest core is still about 3.5 times over budget) and neither is x2; the hybrid conclusion (HLE for live play, LLE for oracle, offline and selected cores, Gate 1b) stands. The new fact is that PNO Core2 no longer bounds the set, and only PNO Core1 (x1.3) is within x1.5. If PNO Core2 under load costs like ORG Core2 (6 280 steps), the three-core need rises to about 9.1 periods (ASSUMED, not measured: no message script).

Reproduce (local-only images, `target/sram/{PNO,ORG}/`; messages `org2.msgtokens` as above):

```bash
# census: sharc_jit_diff command in the table above; module:
sharc-aot target/sram/PNO/core2_sram.bin --whole-pass target/lle/pno2.pcs --force 0x125DDF --tier pe --out target/lle/pno2_whole.rs
export NORD_SHARC_AOT_HOT="ORG2=...;ORG2F=...;PNO2=$PWD/target/lle/pno2_whole.rs;PNO2F=..."
CARGO_TARGET_DIR=target/aot cargo build --release -p nord-tools --bin lle_sharc_bench -p nord-sharc --bin sharc_jit_diff
# 1:1 (module PNO2 alone in the env): sharc_jit_diff IMG <same args without --aot-rank> --aot-top 999 --chunk 20000
target/aot/release/lle_sharc_bench --engine aot --core PNOc2=target/sram/PNO/core2_sram.bin,org,,PNO2
```

## Stages C and D: event-aware blocks (D1) and what was not built (2026-11-27, `ns4-lle3`)

Question: do event-split blocks (D1), then superblocks (D2) and static analysis (C1 to C3) of [sharc-lowering-v2](sharc-lowering-v2.md) bring ORG Core2 exact to x2.5? Tags: EMPIRICAL unless stated.

### D1 result (deterministic, `lle_sharc_bench --profile`, per pass; `AOT DIFF OK` 1:1 for both modules)

| Counter | ORG2 (v1) | ORG2E (D1) | PNO2 (v1) | PNO2E (D1) |
|---|---:|---:|---:|---:|
| refused block entries | 14 | 0 | 14 | 0 |
| interpreter steps after a refusal | 255 | 0 | 255 | 0 |
| native steps | 5 047 | 5 286 | 3 047 | 3 286 |
| generic `aot_step` steps | 481 | 496 | 475 | 490 |
| diff coverage of audio passes | 95.62% | 99.97% | 93.69% | 100.00% |
| diff block misses (all passes) | 4 206 | 5 | 4 201 | 0 |

The 15 extra generic steps are the 15 steps of the former interpreted tails that are generic forms. The "native steps" row is the static bookkeeping of `aot_enter` (each call adds the block's compiled native count), so it counts steps that an event path ran on the interpreter as native; the real figure is the new counter `ev_steps` (steps run by the event paths): **4 807 over boot + 300 passes for both ORG2E and PNO2E (at most 16 per pass, against 255 interpreter steps per pass in v1)**.

Strengthened diff (review of #688): the strict state compared every 20 000 steps now also covers `imask`, `irq.taken`, `irq.stall`, `irq.idle`, `irq.window_len` and `pending` (`irq.due` is not compared: it is derived from the next event cycle and the lowered-op unit tests do not step the controller). `AOT DIFF OK` holds for ORG2, ORG2E, PNO2, PNO2E. **The real passes take no interrupt: 0 interrupts taken and 0 splits in all four runs** (the events are timer and controller pokes), so the `SPLIT` path (an interrupt taken at an event) is exercised only by `aot2/ev_blocks.rs` (hand-built blocks equal to the interpreter with the event at every position on a delayed branch and a loop end, and with a scheduled interrupt), not by the firmware run. Generated source: ORG2 672 584 B, ORG2E 714 465 B (+6.2%); PNO2 628 160 B, PNO2E 666 164 B (+6.0%). 

| Single-module `sharc_jit_diff` build (release, loaded host, 2 builds each, seconds) | ORG2 | ORG2E |
|---|---:|---:|
| `cargo build` wall time | 105, 135 | 106, 107 |
| executable size | 1 400 320 B | 1 446 912 B (+46 592 B, +3.3%) |

Compile time is not measurably different (bands overlap, host loaded). Code bytes grew 3.3% of the executable (+6% of generated source): one `steps >= irq.due` compare per handler and per generic step, and one pre-check per batch.

### Wall-clock: not separable from noise

Every round ran on a saturated host (`LoadPercentage` 88 to 100%, 5 to 17 cargo/rustc/link processes), so absolute figures are x4 to x7 over budget instead of the x3.5 to x3.9 of quiet runs and carry no claim. Interleaved A/B in one binary (9 rounds, 2 warm-up, p50 us of the pass, median (min..max)):

| Core | v1 | D1 | Reading |
|---|---|---|---|
| ORG Core2 exact | 282.3 (266.9..297.2) | 268.5 (231.7..307.7) | within noise (bands overlap) |
| PNO Core2 exact | 210.2 (165.4..224.4) | 184.8 (163.6..203.3) | within noise (bands overlap) |

Mean-us runs (9 rounds, ORG2 341.8 (319.1..427.9), ORG2E 321.5 (283.5..440.3)) agree in direction and are also within noise. D1 is kept on the deterministic counters (refusals and interpreter steps after a refusal fall, `ev_steps` is small); wall-clock is **not shown to regress and not shown to help**: with these bands a 10 to 15% regression cannot be excluded. A quiet-host re-measure is needed before the `perf` claim is made.

### Why D2 and C1 to C3 were not built

By the profiles (shares inflated by the profiler, so upper bounds): generic `aot_step` forms 490 steps = at most 14% of the PNO pass and 8% of ORG (`Move:Other(4b)` 172 steps, `DirectMem` 72, `DagMem` 48, `Move:Other(4a)` 46), block entry/exit 5%, loop glue 1 to 8%. Each lowering slice (one MemXfer form with its random-state test against the interpreter) is worth under 4%; direct chaining of blocks (D2) replaces only the dispatcher's search, not the guard or the code check, so under 3%; constants and range proofs (C1, C2) act inside native steps whose cost is instruction fetch of shared handlers (code bytes per step is the budget, see the bounded experiment). All are below the 3% quiet and 14% loaded min-max bands of this host, so by the stop rule none can be shown, and none was built. **ORG Core2 exact at or below x2.5 is not reached**; the last quiet measurement (stage B, 158.9..164.4 us) is x3.5, the host here cannot give a better one. Re-measure D1 on a quiet host before building anything further.

Reproduce: `target/lle/gen.sh` (modules incl. `--ev`), `target/lle/build.sh OUT`, then `NORD_SHARC_AOT_TAG=ORG2E sharc_jit_diff IMG $(cat org2.run) --aot-top 999 --chunk 20000` and `lle_sharc_bench --engine aot --profile --core ORGc2=...,ORG2E` (scripts are local, same commands as "Reproduce" below with `sharc-aot --ev`).

## Alternatives (what to do instead)

- **HLE hybrid** (recommended by the numbers): run LLE only for cores that fit the budget and keep the Rust HLE engines for the rest. On these measurements only PNO Core1 (x1.6 over, closest) could ever fit; ORG/PNO Core2 are x7 to x8 over.
- **Slower-than-real-time LLE** for non-live uses: preset rendering and null tests of the HLE engines against the firmware.
- **Bounded redesign experiment** (done, section above): shared handlers, in-place compute and fewer bail-outs took ORG Core2 from x7.2 to x4.6 and stopped at the checkpoint; lowering v2 stages A and B then reached x3.5; x2 is not reachable in exact mode with this design (see "Verdict after stage B").

## Implications

- Gate 2 beans (`ns4-lle4` and later) must not assume real-time SHARC execution. Measured gap on the slowest benchmarked core: x8.0 with the first AOT prototype, x10 to 13 interpreted; now ORG Core2 x3.5 with AOT and PNO Core2 x6.5 to x6.7 on the interpreter. All benchmarked cores together need 17 to 20 host cores of budget.
- `pass_sched` and `lle_sharc_bench` are reusable for `ns4-lle4`/`ns4-lle5` and for the follow-up experiment.
- D118 (whole-pass AOT) is answered: bit-exact and buildable, not real time.

## Follow-ups

- Presets and message scripts for ORG Core1 (FX Core1/Core2 done in `ns4-lle4`); SY1SY2/SMP as ISR-driven jobs in `pass_sched`, so the 300-pass benchmark covers all five chips.
- AOT module for PNO Core2: done (section "PNO Core2 AOT module"). Still open: a PNO Core2 message script for a loaded pass; a re-measure on a quiet host; lowering of the 475 generic `aot_step` steps and of the 14 refused block entries per pass (each below the noise band, see Step 4 there).
- Fast-vs-Exact: measured 2026-11-27 on ORG and PNO Core2 with `fast_vs_exact` (state error 2e-5 relative, bounded; `sharc-kernel-speed.md`, D256). The SNR in dB of the output is still open: both streams are silent.
- Listening/null test for the fast-f32 tier, only if the compute share of the pass becomes native.

## Reproduce

```bash
# images and message scripts: see org-system-run.md (local-only)
# per-step rounds: regenerate the modules, rebuild, run the 1:1 diff and the benchmark, then take the minimum mean of 10
# `lle_sharc_bench --engine aot --core ORGc2=...,ORG2` runs per binary, alternating binaries
cargo run --release -p nord-sharc --bin sharc_jit_diff -- IMG ARGS --aot-pcs target/lle/org2.pcs
cargo run --release -p nord-sharc --bin sharc-aot -- IMG --whole-pass target/lle/org2.pcs --force 0x125DDF --tier pe --out target/lle/org2_whole.rs
export NORD_SHARC_AOT_HOT="ORG2=$PWD/target/lle/org2_whole.rs;ORG2F=$PWD/target/lle/org2_whole_approx.rs"
CARGO_TARGET_DIR=target/aot cargo build --release -p nord-tools --bin lle_sharc_bench
target/aot/release/lle_sharc_bench --engine aot --passes 300 \
  --core PNOc1=target/sram/PNO/core1_sram.bin,pno \
  --core PNOc2=target/sram/PNO/core2_sram.bin,org \
  --core ORGc2=target/sram/ORG/core2_sram.bin,org,target/lle/org2.msgtokens,ORG2
```
