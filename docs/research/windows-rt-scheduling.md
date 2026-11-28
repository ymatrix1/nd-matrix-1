# Windows real-time scheduling for the LLE threads: what to set and what to leave alone

- **Checked:** 2026-11-28 (from memory of the Microsoft documentation; the pages below were **not re-fetched** in this session, and nothing was run on Windows)
- **Sources:** Microsoft Learn: `SetPriorityClass`, `SetThreadPriority`, `SetThreadAffinityMask`, `SetThreadIdealProcessor`, "Scheduling Priorities", "Multimedia Class Scheduler Service", `AvSetMmThreadCharacteristics`; Microsoft "Windows Internals" chapter on thread scheduling and DPCs. Code: `crates/nord-sharc/src/runner/rt_sched/`, `runner/pass_sched/`.
- **Question:** which process and thread settings reduce the wake-up jitter of the spin-barrier pass threads (8 SHARC cores plus a coordinator, 46.08 us per pass, all threads synchronize every pass) without risking the machine, and what must stay off?

Tags: **VERIFIED** = reproduced by a command, **EMPIRICAL** = measured, **ASSUMED** = reading of the documentation. Everything below is ASSUMED: no measurement exists yet.

## 1. What the module sets

| Setting | Call | Value | Why |
|---|---|---|---|
| Process class | `SetPriorityClass` | `HIGH_PRIORITY_CLASS` (0x80) | Lifts all threads of the process above normal ones. Process wide, so every thread applying it is a repeat (harmless). |
| Thread priority | `SetThreadPriority` | `THREAD_PRIORITY_TIME_CRITICAL` (15) | Highest level inside the High class (base priority 15). |
| MMCSS | `AvSetMmThreadCharacteristicsW("Pro Audio")` | once per thread | The service raises the thread inside its own range and reserves CPU share for the task; "Pro Audio" is the stock audio task. |
| Placement | `SetThreadAffinityMask` (one bit) or `SetThreadIdealProcessor` | from `plan_affinity` | One logical processor per spinning thread. `Hard` forbids migration; `Ideal` is a hint. |

Off unless `NS4_RT_SCHED=1`. Scheduling changes timing, never output: the module cannot reach emulator state, and the pass loop synchronizes on atomics whatever the OS does (`rt_sched::apply::tests::policy_never_changes_results` pins the contract).

## 2. Why not `REALTIME_PRIORITY_CLASS`

- Real-time class threads (priority 16 to 31) outrank the kernel threads that flush the disk cache, service the mouse and keyboard and run the audio engine. Nine threads that spin 100 % of the time at that level can starve them and hang the machine (ASSUMED from the documented warning on `SetPriorityClass`).
- It needs the "increase scheduling priority" privilege; without it Windows silently gives High, so the code would not even do what it says.
- The gain over High + TIME_CRITICAL + MMCSS is small once each thread owns a processor: the jitter that remains comes from interrupts and DPCs (section 4), which no user priority outranks.
- The type has no `Realtime` variant and a unit test checks that the High value is not 0x100.

## 3. Placement

- **Reserved processors.** The first two logical processors are left free (`RESERVED_LOGICAL`): the system and the audio driver's interrupts and DPCs tend to run on the lowest-numbered processors (ASSUMED; confirm with `xperf`/WPA DPC data per CPU).
- **Few cores.** With fewer than `threads + 2` logical processors the plan does not pin and reports `NoPin::TooFewLogical`. Pinning a spinning thread onto the audio driver's processor is worse than letting the scheduler move it. On a 4-logical machine this means 2 threads at most.
- **Hyper-threading siblings.** On SMT machines logical processors `2k` and `2k+1` share one physical core, and two spinning threads on siblings each run well below full speed. `plan_affinity(.., smt_pairs = true)` fills the even processors first (one thread per physical core) and the odd ones only when it must. Detecting SMT (`GetLogicalProcessorInformationEx`) is a follow-up; until then the caller passes the flag. With SMT the two reserved processors are one physical core.
- **More than 64 logical processors** need processor groups; the plan caps at 64 (`MAX_MASK_CPUS`) and group-aware pinning is a follow-up.
- **Process affinity limits** (job objects, `start /affinity`) can make an absolute bit invalid: the call fails and the step reports `Failed(Os{..})`; the thread keeps running unpinned.

## 4. What the module cannot fix

| Source | Effect | Mitigation outside this module |
|---|---|---|
| DPCs and ISRs of drivers (GPU, network, storage, audio) | Run above every thread priority on the processor they land on, tens to hundreds of us | Keep the audio driver's processor out of the plan (done by reserving); LatencyMon/WPA to find the offender |
| Antivirus and filter drivers | File I/O stalls, scan bursts | Exclusion for the working directory; no file I/O in the pass loop |
| Timer interrupt (15.6 ms default, 1 ms with `timeBeginPeriod`) | Periodic preemption | The threads spin, they do not sleep, so the timer resolution does not delay wake-ups |
| Power management (core parking, C-states, frequency scaling) | Slow first pass after idle | High-performance power plan; spinning keeps the processor awake |
| SMT sibling load | See section 3 | Plan flag |
| MMCSS throttling | The service lowers a task's share if it uses the CPU for long at high priority (ASSUMED: the documented 20 % non-realtime reserve) | Measure with the latency helper; fall back to not joining MMCSS if the tail gets worse |

## 5. Measuring

`rt_sched::latency::measure_wakeup(iterations, gap_ns, setup)` spins a waiter thread on an atomic, stamps `Instant` (QueryPerformanceCounter on Windows) in the publisher and returns the observed delays; `percentiles` reduces them to min, p50, p90, p99, p99.9 and max. `setup` runs on the waiter thread, so the same measurement can be repeated with `apply_to_current_thread(..)` inside it. No `Sleep` is involved. The synthetic-sample test checks the percentile arithmetic only; **no latency number exists yet**.

## Implications

- The policy is cheap to add and safe by construction (no real-time class, no pinning onto the reserved processors, off by default), but its benefit is unmeasured. Treat it as an experiment until section 5 shows a smaller p99.9 on real hardware.
- A machine with 4 logical processors cannot give each of 8 cores a thread of its own at all; the LLE budget needs the cores regardless of this module.

## Follow-ups

- Wire `rt_sched::apply_if_enabled` into `pass_sched::run_ticks_with`, at the start of each worker closure (`pass_sched/sched.rs`, inside the `std::thread::spawn` body before `work(..)`, role `ThreadRole::Core(index)`) and at the start of the coordinator loop (role `ThreadRole::Coordinator`).
- Run `measure_wakeup` on Windows with and without the policy; record p50/p99/p99.9 here and tag them EMPIRICAL.
- Detect SMT and processor groups; use `GetLogicalProcessorInformationEx`.
- Decide whether a Linux backend (`sched_setaffinity`, `SCHED_FIFO` needs rtkit/limits) is wanted.
