# ARM0 native Rust runtime: wall-clock per 1 ms tick

**Checked:** 2026-11-24 (first run 2026-11-23; predecode cache, tail, queue and budget measured 2026-11-24).

**Sources:** `crates/nord-arm` + `crates/nord-tools/src/bin/arm_lle_bench.rs`; user OS v1.66 images `target/fw_unpacked/bank0_CODE.bin` (loaded at `0xC0000000`) and `bank0_SRAM.bin` (read-only, never committed); a 256 KB EEPROM image built by `tools/arm_program_load.build_image` from `target/oled/eeprom.bin` plus factory program `B3 Chords n Solo.ns4p`; Python reference `tools/arm_org_stream.py`, `tools/arm_steady.py`, `tools/arm_dsp_return.py`; [pno-jit-arm-profile](pno-jit-arm-profile.md) (Python/Unicorn cost); [dsp-to-arm-return](dsp-to-arm-return.md); [arm-tick-timers](arm-tick-timers.md) (24 kHz timer, D130); [arm-dsp-live-link](arm-dsp-live-link.md) (7,340 count); [arm-oled-render](arm-oled-render.md) (display flush).

**Question:** Can ARM0 run the real OS v1.66 boot, program load and key events inside Rust below 1.0 ms wall-clock per 1 ms virtual tick (`ns4-lle2`, Gate 1a), and what is the tail, the message latency through a decoupled queue and the instruction budget per ms?

**Answer in one line:** a predecode cache doubles the instruction rate (median 93 to 187 M insns/s in an interleaved A/B, state and message content identical to the plain `Cpu::step` path at four tick budgets); the mean is 0.11 ms (p50 0.07), 0.2 to 0.5 % of ticks (4 to 9 of 1,900) still run over 1 ms (max 1.5 ms, never more than 3 in a row, worst backlog up to 0.55 ms), so "< 1 ms on every tick" is still NOT strictly met; through a decoupled queue drained once per ms no message waits more than 1 ms in the simulation.

## Method

Machine: AMD64 Family 25 Model 33 (AMD, Zen 3 class, 32 logical processors, shared with other sessions: every number is noisy, A/B comparisons are best of N), Windows 11, release build.

```
cargo run --release -p nord-tools --bin arm-lle-bench -- \
  <bank0_CODE.bin> <bank0_SRAM.bin> <eeprom_b3.bin>
```

Environment switches: `NO_FULL=1` (skip the full-execution comparison run), `BOOT_ONLY`, `PHASE_SCALE` (percent of the phase lengths), `TICK_INSNS` (instructions per tick, default 200,000), `MICRO=1` (dispatch rates on synthetic code), `PACED=1` (producer/consumer threads, `PACED_CAP` queue ticks), `STEADY_BOOT=1` (the `arm_steady.py` boot, null-call histogram).

1. Boot to the main loop `0xC00A0248` with the full board attached (panel, 4 I2C EEPROMs, UART2) and idle skipping on.
2. Real timer on, DSP port logging and the DSP return hooks installed, 64 MB of arena reserved so on-demand mapping cannot reallocate during the run.
3. Six phases: load 800 ticks, idle 300, one key on 200, one key off 200, eight keys on 200, eight keys off 200. One tick = `TICK_INSNS` ARM instructions of virtual time (200,000, ASSUMED, see "Instruction budget") plus one timer IRQ.
4. **The timed region of each tick contains** the MIDI send (first tick of a phase, includes the UART ISR), the tick run with idle skipping, and `drain_messages_into` (message hand-over plus the DSP return sync).
5. The same phases run again with full execution (no skipping) on a clone. The machine state hash and an FNV digest of the full message list (path, comm, words, pc) are compared after every phase.

## Predecode cache (follow-up 1)

One decoded page (1,024 `Entry { insn, cond mask, op, run }`) per guest 4 KiB code page, built on first execution, driven by a 128-entry handler table (`h_dp::<OP, S, FORM>` const generics for the 96 data-processing forms). `run` is the number of straight-line instructions before the next block ender (B, BL, BX, loads or data processing into r15, anything on the slow path); non-enders run without per-instruction pc, condition or limit handling. Page flag `HOOK_C` marks a page with a decode: a store to it, or a new execute hook on it, drops the decode before the next fetch, so self-modifying code and hooks keep their semantics. The idle probe runs on the same fast path through a probe mode of `SysBus` (`probing`, `impure`). Details: [arm0-runtime](../specs/arm0-runtime.md).

| Build | Boot rate (executed insns/s) | Source |
|---|---|---|
| Old interpreter (`Cpu::step` per instruction) | 93.5 M median (93 to 95), 7 interleaved runs | A/B on the shared machine, see "Measurement conditions" |
| + per-instruction decode cache | 136 M | same |
| + block execution, inline B/BL | 167 M | same |
| + probe on the cached path, page-wise zero | 187 M median (184 to 189), 7 interleaved runs | same |

About 2.0x, **not the targeted 5x**. Straight-line synthetic code reaches about 470 M insns/s (`MICRO=1`), so dispatch is no longer the limit: the firmware is branchy and load/store heavy and each guest instruction still costs about 5 ns of host work (memory entry lookup, device checks, flags). `codegen-units = 1` gave +3 % and was not adopted. A faster core needs a different technique (translation to host code or superblocks with cached flags), not a better dispatch loop.

**Equivalence (every existing test kept, plus new ones).** Boot state hash `0960ef4eefb7fad5` and the per-phase message digests (load `221c6f69751f1f8e`, idle `0cb5cb143a3b7106`, key1 on `67ab63840268b92d`, key1 off `dbdec8588970d602`, key8 on `17622e4962354279`, key8 off `8f24476a8b30f9ee`) and per-phase state hashes (`68a8abb37e4e1d0b`, `8221207b8b72c8e4`, `97893e12e5db4be8`, `d2eb2e9801341ef5`, `a5e39de19c49b79e`, `6be2c14282a84986`) are **identical** to the old interpreter's (parent commit built with the digest lines added) at the default 200 k budget. At 100 k, 300 k and 456 k the parent-commit binary was not run; there the comparison is this tree with `PREDECODE=0` (plain `Cpu::step`) against the cache, and all four budgets (100 k, 200 k, 300 k, 456 k) give identical output: 13 digest lines each, boot state hash plus six per-phase state hashes and message digests (`oldnew.py` in the scratch folder, raw output `target/lle/oldnew.txt`). The skipping run equals the full run in all phases, but both run on the predecode path (the bench clones the machine), so that check is idle skipping against full execution, not old against new. Unit tests: a 200,000-word random differential of `exec_decoded` against `Cpu::step` (registers, cpsr, bus access log), a 60-program random differential of `Machine::run` with `predecode` on and off including self-modifying code, store and hook invalidation tests, probe-mode tests. `WFI` count is 0 in this workload and is covered by `wfi_has_one_semantic_in_the_full_and_the_skipping_path`.

## Measurement conditions and variance

The machine is shared (three Claude sessions, a codebase indexer and two spinning `find` processes ran throughout), so wall-clock numbers are noisy and the tail (p99, p99.9, max, over-1-ms counts) is the most sensitive. Conditions: AMD Ryzen 9 5900XT (16 cores, 32 logical, 3.3 GHz base), Windows 11, power plan "Alto desempenho" (High performance), release build, same build flags for both binaries. No `cargo` or `rustc` process was running during the A/B below; host CPU load sampled just before each run was 49 to 99 % (median 87 to 88 %), i.e. **never idle**, so absolute values are an upper bound on cost and the other cores were busy.

A/B, load phase, `NO_FULL=1`, one warm-up run of each binary discarded, then 7 runs of each **interleaved** (old, new, new, old, ...; `ab_tail.py` in the scratch folder, raw rows in `target/lle/ab_tail.json`). Median (min to max):

| Metric (load phase unless noted) | Old interpreter | Predecode cache | Outside noise? |
|---|---|---|---|
| Boot rate, M executed insns/s | 93.5 (93 to 95) | 187 (184 to 189) | yes, 2.0x, ranges do not overlap |
| Boot wall time, s | 2.48 (2.44 to 2.49) | 1.24 (1.23 to 1.26) | yes |
| Mean ms/tick | 0.270 (0.268 to 0.275) | 0.108 (0.105 to 0.112) | yes |
| p50 ms | 0.170 (0.170 to 0.171) | 0.070 (0.068 to 0.071) | yes |
| p99 ms | 2.53 (2.49 to 2.61) | 0.963 (0.952 to 0.994) | yes |
| p99.9 ms | 2.72 (2.56 to 3.61) | 1.27 (1.19 to 1.36) | yes (old max is a spike) |
| Max ms | 2.73 (2.65 to 3.62) | 1.48 (1.42 to 1.55) | yes |
| Ticks over 1 ms (of 800) | 36 (35 to 37) | 6 (4 to 8) | yes |
| Longest run over 1 ms | 32 (32 to 32) | 1 (1 to 3) | yes |
| Worst backlog, ms | 47.9 (47.4 to 50.6) | 0.48 (0.42 to 0.55) | yes |

Earlier single runs in this note (old: 43 ticks over 1 ms, backlog 61 ms; boot 83 to 90 M insns/s) were taken under heavier load and sit outside the old binary's A/B range: they show the noise, and the A/B medians above replace them as the quoted before values. The new core's figures agree across 12 runs (5 + 7): 4 to 9 ticks over 1 ms in all phases, max 1.42 to 1.55 ms, run at most 3.

**What is deterministic.** Executed instructions per tick do not depend on the host: 12,481 per load tick (mean), 3,288 to 3,448 in the other phases, identical in every run of both binaries, and every over-1-ms tick of the old core executed the full 200,000-instruction budget. So the tail is set by how many ticks do a full budget of real work (a count fixed by the firmware and the tick budget) times the host cost per instruction (about 5.3 ns now, about 10.7 ns before); OS scheduling only adds the jitter seen in the ranges above. I did **not** measure per-thread CPU time or count host cycles, so the wall-clock tail still includes preemption by the other load: a single spike coincident with other load (such as the old max of 3.62 ms in one run) is unproven. The conclusions drawn (6x fewer slow ticks, no backlog above 0.55 ms, still 4 to 9 ticks over 1 ms) rest on medians and on ranges that do not overlap.

Not repeated under these conditions (single runs, noise band not measured, treat as indicative): the tick-budget sensitivity table (the ordering of 300 k against 456 k in ticks over 1 ms, 23 against 16, is inside the noise; the growth of p99 and max with the budget is not), the queue simulation (derived from the same per-tick costs; its 1 ms maximum is quantised by the 1 ms drain and was the same in all five runs that printed it), and the `PACED=1` runs (OS scheduler dominated, already not quoted).

## Tick tail with the faster core (follow-up 2)

Skipping run, 1,900 ticks per run, five runs (`NO_FULL=1`), min to max of the five (see the previous section for the interleaved A/B and its conditions):

| Phase | Ticks | Mean ms | p50 | p99 | p99.9 | Max | Ticks > 1 ms | Longest run > 1 ms | Worst backlog ms | Executed insns/tick | Messages |
|---|---|---|---|---|---|---|---|---|---|---|---|
| load | 800 | 0.109 to 0.111 | 0.071 | 0.965 to 1.000 | 1.25 to 1.39 | 1.43 to 1.53 | 4 to 8 (0.5 to 1.0 %) | 1 to 2 | 0.43 to 0.53 | 12,481 | 15,337 |
| idle | 300 | 0.063 to 0.066 | 0.072 | 0.09 to 0.14 | 0.10 to 0.39 | 0.13 to 0.52 | 0 | 0 | 0 | 3,384 | 3 |
| key1 on | 200 | 0.063 to 0.065 | 0.072 | 0.09 to 0.13 | 0.13 to 0.21 | 0.19 to 0.48 | 0 | 0 | 0 | 3,288 | 126 |
| key1 off | 200 | 0.067 to 0.069 | 0.072 | 0.17 to 0.20 | 0.20 to 0.31 | 0.30 to 0.54 | 0 | 0 | 0 | 3,448 | 98 |
| key8 on | 200 | 0.069 to 0.072 | 0.072 | 0.12 to 0.40 | 0.40 to 0.45 | 0.92 to 0.95 | 0 | 0 | 0 | 3,365 | 1006 |
| key8 off | 200 | 0.070 to 0.072 | 0.072 | 0.11 to 0.39 | 0.38 to 0.47 | 0.97 to 1.01 | 0 to 1 | 0 to 1 | 0 to 0.01 | 3,426 | 969 |

All phases: 4 to 9 ticks over 1 ms out of 1,900 (0.2 to 0.5 %), longest run 1 to 3, worst backlog 0.42 to 0.55 ms (backlog = work behind real time if ticks are consumed at one per ms and slow ticks carry over). Before (old interpreter, interleaved A/B medians): load mean 0.270, p50 0.170, p99 2.53, max 2.73, 36 of 800 load ticks over 1 ms (4.5 %), longest run 32, backlog 47.9 ms (first single run, under more load: 38 ticks, max 3.83, backlog 61.5 ms). A full 200,000-instruction tick cost about 2.1 ms before and costs about 1.03 ms now (`target/lle/old_full.txt`, `v3_full.txt`). The over-1-ms ticks are the same real firmware work as before (full 200,000-instruction ticks during program load); they got about 2x shorter, not 5x.

**Criterion "< 1 ms on every tick": NOT met.** 0.2 to 0.5 % of ticks exceed it, by at most 0.55 ms. The bean criterion stays unticked. For real-time use the margin is small but the tail is bounded: no backlog above 0.55 ms and no run above 3 ticks.

## Message latency through the decoupled queue (follow-up 2)

Simulation in the bench (`queue ...` lines, `metrics::simulate_queue`): the ARM thread produces one tick per recorded cost, a consumer drains the queue once per ms. Latency of a message = time from the end of its tick's virtual ms to the drain that delivers it.

| | p50 | p99 | p99.9 | max | Max queue | Max ticks per drain |
|---|---|---|---|---|---|---|
| Old interpreter, all phases | n/a | 59 ms | n/a | 61 ms | 1,800 msgs | 7 |
| Faster core, load | 0 to 1 ms | 1 ms | 1 ms | **1 ms** | 1,855 msgs | 2 |
| Faster core, key phases | 0 | 0 | 0 | 0 to 1 ms | 907 msgs (key8 on) | 1 to 2 |

Queue size: the worst backlog (up to 0.55 ms) is one tick of slack, the largest drain is 2 ticks and 1,855 messages (load phase, one tick carries up to about 1,000 messages). A bounded queue of 4 ticks / 4,096 messages suffices **only at the ASSUMED 200,000 instructions per tick**: at 300,000 the backlog is 9.4 ms with 21 slow ticks in a row and at 456,000 it is 17.0 ms with 14 in a row (budget table below), so the queue must be sized for the budget that is actually true (32 ticks covers both measured cases); the bench's real-thread run uses 64 ticks (64 ms, 120x the measured backlog) and the producer was blocked less than 0.5 ms in total, i.e. the queue was never full. `MESSAGE_CAP` in the bus is 262,144 messages.

**Real threads (`PACED=1`, three runs, EMPIRICAL, unreliable).** Producer thread paced at one tick per wall-clock ms, consumer thread draining once per ms with the OS sleep. Lateness is measured against the ideal delivery time. Best run: p50 and max 1.00 ms (load), 0 to 1 ms elsewhere; the other two runs 2 to 17 ms and 21 to 83 ms with 20 to 60 % of ticks late. Producer blocked time was 0.2 to 0.4 ms in all three runs, so the core kept up; the spread is the **consumer wake-up jitter of the shared machine** (Windows timer granularity, 32 threads used by other sessions), not core cost. This run exercises the queue and thread plumbing, not a latency number; it must be repeated on a quiet machine with raised timer resolution before a latency claim is made.

## Instruction budget per ms (follow-up 4)

`INSNS_PER_TICK = 200000` is a harness convention: the real hardware timer interrupt is at 24 kHz (41.67 us, [arm-tick-timers](arm-tick-timers.md) D130), and the Python harness runs 200,000 instructions per 1 ms of virtual time. The real figure cannot be measured without the hardware (no cycle counter is read by the firmware). It is **bounded**, not measured:

* Upper bound: the ARM926EJ-S is single-issue (IPC <= 1), so instructions per ms <= clock in kHz. The AM1705 clock is 375 or 456 MHz per the repository docs; the repository elsewhere assumes 300 MHz (12,500 cycles per 24 kHz interrupt). Bound: **<= 300 k (300 MHz), 375 k or 456 k (AM1705 clocks) instructions per ms**. The clock itself is programmed by the boot loader and not read from the images here (ASSUMED).
* Cache misses, load-use stalls, branch penalties and 16-bit device accesses put the real IPC well below 1 (typical 0.5 to 0.8 for control code), which at 300 to 456 MHz is about 150 k to 365 k per ms. The assumed 200 k is inside that range (IPC 0.67 at 300 MHz) but not pinned by it.

Sensitivity of the benchmark to the budget (skipping run, one run each, load phase; the other phases stay below 1.2 ms in all runs):

| `TICK_INSNS` | load mean | load p99 | load max | Load ticks > 1 ms | Longest run | Worst backlog | Full-exec cost per tick |
|---|---|---|---|---|---|---|---|
| 100,000 | 0.110 | 0.61 | 1.28 | 1 | 1 | 0.28 ms | 0.55 ms |
| 200,000 | 0.110 | 0.97 | 1.5 | 4 to 8 | 1 to 2 | 0.5 ms | 1.03 ms |
| 300,000 (about 300 MHz at IPC 1) | 0.110 | 1.45 | 1.73 | 23 (2.9 %) | 21 | 9.4 ms | 1.5 to 2.6 ms |
| 456,000 (456 MHz at IPC 1) | 0.109 | 2.17 | 2.39 | 16 (2.0 %) | 14 | 17.0 ms | 2.7 to 4.2 ms |

The mean does not move (executed instructions per tick stay at 12,500 because idle skipping removes the rest), the tail does: the load-phase ticks that run a full budget scale with it. State hash and message content equal the full run at every budget. If the hardware retires 300 k+ instructions per ms (an upper bound, not an expectation), the load tail needs a faster core or a decoupled thread with a backlog of up to 17 ms. If it retires 100 k, the tail disappears (1 tick over 1 ms in 800).

## Null-call count 747 against 762 (follow-up 3)

`STEADY_BOOT=1` runs the `tools/arm_steady.py` boot (display stubs on, no board) and prints a histogram of `(pc, lr)` per null call; the same histogram was taken from `arm_steady.boot_to_main_loop` with Unicorn (`uv run --with unicorn`, 36 s).

* Both runs stop at `0xC00A0248`; both hit the **same 68 distinct call sites** (`pc = 0`, different `lr`).
* 65 sites have **identical counts**. Three sites differ: `lr = 0xC007694C`, `0xC0076964`, `0xC0076978`, **8 times in Python, 3 times in Rust** (5 x 3 = 15 = 762 - 747). They lie inside the display flush `0xC0076924`, which sends the three command bytes `B0`, `00`, `10` through the display driver's vtable ([arm-oled-render](arm-oled-render.md)); with no display driver object in this boot, each send is a null call.
* So Python flushes the frame buffer 8 times during the boot and Rust 3 times; nothing else differs (VERIFIED, run). The flush is a periodic display refresh, so the likely cause is a difference in how often the boot's wait loops see timer ticks (Rust injects 6 wait-loop IRQs, the Python harness breaks polls with `enable_poll_breaker`; ASSUMED, not isolated). The difference is confined to this boot variant: the benchmark boot (board attached) ends with 25 null calls and its state matches between the skipping and full runs.
* **Documented, not fixed:** matching Python exactly would mean copying the Python poll-breaker timing into the Rust wait-loop model, which has no purpose (the Python count is the less accurate of the two, being unticked).

## Load-phase message count 15,337 against 7,340 (follow-up 3)

The two numbers count different things:

* **15,337** is every message the bench sees in the 800 load ticks: all DSP chips (ORG, PNO, FX, SMP, SY), all cores, path 1 and path 2.
* **7,340** is the PNO core 1, path 2 words only: [arm-dsp-live-link](arm-dsp-live-link.md) says "load phase (800 ticks) 7,340" for the stream into PNO core 1, and `tools/arm_org_stream.py` `select(messages, chip=PNO, core=1)` keeps only `m.chip == "PNO" and m.path == 2 and m.core == 1`. The same research finds ORG core 1, ORG core 2 and PNO core 2 receiving a common 876-word strip set (912 words per target in the load phase, [arm-dsp-live-link-synth](arm-dsp-live-link-synth.md)) and further FX and SMP traffic, which makes 15,337 total against 7,340 for one target plausible.
* **Consistent with the count, not proven:** the 7,340 stream and the 15,337 total differ in scope, and nothing here shows the PNO core 1 path 2 share of the Rust stream is 7,340.
* **Not recomputed per target.** `grouper::Message` carries `path` and `comm` but not the chip and core, which `tools/arm_dsp_route.py` derives from the comm object; the per-target split of the Rust stream was therefore not produced. The Rust stream is equal in content to the full-execution run (digest) and its load phase was the same length (800 ticks); the two counts are consistent, not proven equal per target. Follow-up: add the chip/core routing to `nord-arm` (needed for `ns4-lle3` anyway) and compare 7,340.

## Findings from the review of the first run

* A message split across a tick boundary lost its first words (the grouping state was rebuilt on each drain). Fixed: the grouper lives in the bus; test `a_message_split_across_two_drains_is_delivered_whole`.
* Stub returns now set the instruction set from `lr` bit 0 (test `stub_return_takes_the_instruction_set_from_lr`).
* `WFI` has one semantic in both run paths (the instruction completes, the rest of the budget is skipped); `Machine::wfi` counts it.
* Unbounded logs are capped (`null_calls`, `auto_mapped`, `I2cBus::log`, bus messages), `DspReturn::read` and the drain no longer allocate per call.

## Not verified

* `INSNS_PER_TICK = 200000` is ASSUMED; bounded above by clock x IPC (300 k to 456 k), not measured.
* The real-thread latency run is dominated by OS scheduling noise on the shared machine; only the simulated queue (deterministic given the measured tick costs) gives a latency figure.
* The cause of the display-flush count difference (5 more flushes in Python) is not isolated; the per-target split of the 15,337 messages was not computed.
* DSP return layers beyond the SY idle mask and the shadow of the ARM's own messages (live chip, initial image) are not ported (`ns4-lle5`); no key phase reads the SPI return path often enough to measure it here (`DSP return reads` was 0).
* `inject_irq` runs the handler even if the CPSR I bit is set (as the Python harness does); this happened once (`irq_while_masked`). `Cpu::mode_changes` was 0.
* Only OS v1.66 `bank0` (ARM0) and one program were measured, on one machine, shared with other sessions.

## Implications

* A native interpreter with a predecode cache is enough for the mean and for key events; decision [D245](../DECISIONS.md), update [D250](../DECISIONS.md).
* Idle skipping needs no firmware knowledge; it is exact on the 6 benchmark phases at four tick budgets (state and message content), not proven in general.
* A decoupled ARM0 thread with a queue of a few ticks keeps every message within 1 ms of its tick in the simulation. A lockstep design inside the 1 ms audio callback would still miss 0.2 to 0.5 % of ticks by up to 0.5 ms.

## Follow-ups

* A further 2x or more needs host-code translation or superblock caching, not more dispatch tuning (not started).
* Repeat the real-thread latency run on a quiet machine with raised Windows timer resolution (`timeBeginPeriod(1)`).
* Pin the instruction budget: calibrate against a firmware delay loop with a known wall-clock duration, or measure the PLL setup the boot loader writes.
* Chip/core routing in `nord-arm` and a per-target comparison with the 7,340 count (`ns4-lle3`).
* Port the live-chip and initial-image return layers (`ns4-lle5`).
