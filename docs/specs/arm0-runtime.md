# ARM0 runtime (`crates/nord-arm`)

Status: v0.2 (2026-11-24), bean `ns4-lle2`, decisions [D245](../DECISIONS.md), [D248](../DECISIONS.md). Measurements: [arm0-rust-runtime](../research/arm0-rust-runtime.md).

Native Rust ARM926EJ-S (ARMv5TE) interpreter that boots the user's OS v1.66 ARM0 image with the hardware the Python harness stubbed. Opt-in crate (no dependencies, not linked by the audio engine). Firmware bytes are never committed: the tests use synthetic in-memory code.

## Layout

| Module | Role |
|---|---|
| `cpu/` | `Cpu`, ARM and Thumb decode (`arm*.rs`, `thumb*.rs`), `alu.rs`, `Bus` trait. WFI (`mcr p15,0,rd,c7,c0,4`) returns `Fault::Wfi`. |
| `cpu/predecode.rs`, `cpu/arm_cached.rs` | Predecode cache entries (`Entry`, `ArmPage`, `decode_arm`, `ends_block`, `COND_MASK`), `Cpu::exec_decoded` / `exec_pure`, and the 128-entry handler table `Table::ARM` (const-generic data-processing handlers). |
| `mem.rs` | 4 GiB sparse paged memory, per-page flags `HOOK_R/W/X/C`, unmapped access maps a zero page, `entry` / `*_e` accessors that reuse one page lookup, page-wise `zero`, `stale_code` list. |
| `machine/mod.rs` | `Machine::run(max, until, stop_pc)`, `call_preserving`, hooks table, null-call guard. |
| `machine/hooks.rs` | `Hook` enum: `Return`, `BumpNew`, `BoardConfig`, `SpiTransfer`, `SkipInsn`, `ZeroR5`, `BoardPresent`, `DspRead`. |
| `machine/boot.rs` | Firmware load at `0xC0000000`, boot stubs, init array, `run_tick`, timer IRQ 22 injection. |
| `machine/sysbus.rs` | Tick counter `0x80001DC0`, GPIO echo, poll breaker, fake DSP ports (path 1 `0xA0011000`, path 2 `0xA0010000`), port store grouping into messages. |
| `machine/{panel,i2c,uart}.rs` | Front panel on CS2, four 64 KB I2C EEPROMs (program memory), UART2 16550 MIDI input. |
| `machine/grouper.rs` | `Grouper`/`Message`: incremental grouping of port stores into ARM to DSP messages, state kept in the bus so a message split across a drain keeps its first words. |
| `machine/system.rs` | `boot_system`, `send_midi`, `drain_messages(_into)` (completed messages since the last call). |
| `machine/dsp_return.rs` | DSP to ARM SPI reads (`0xC00119A4`, `0xC00117E8`, `0xC0066E64`): SY idle voice mask, then shadow of the ARM's own messages, then 0. |
| `machine/idle.rs` | Idle skipping (probe on the cached path through `SysBus` probe mode). |
| `machine/fastrun.rs` | `CodeCache` (decoded pages), `run_cached` (block loop), `step_cached`, `drop_stale_code`. |
| `machine/probe.rs` | `SysBus::probe_read/probe_write/probe_watch`: record whether a probe touched anything impure. |
| `machine/{metrics,paced,micro}.rs` | Benchmark support: percentiles, backlog and queue simulation, message digest; producer/consumer threads; dispatch micro-benchmark. |

## Time model

One tick = 1 ms virtual = 200,000 instructions (ASSUMED) followed by one timer IRQ. Interrupts are not CPU exceptions: the AINTC handler `IRQ_TABLE[irq]` is called with all registers and the CPSR preserved (sp lowered by `0x800`, lr = `SENTINEL`), as in the Python harness. The low 64 KB is zeroed between ticks and null calls return 0.

## Idle skipping (exact)

From state `S0`, a probe runs up to `MAX_PERIOD = 4096` instructions on an instrumented bus. If the CPU returns to the same registers/CPSR and, meanwhile, no store changed memory, no device page (tick, poll windows, GPIO, DSP ports) was touched, no hook ran and no unmapped page was mapped, the state equals `S0`. Then `k * L` instructions are added to the virtual counter without executing and the remainder runs for real. `WFI` (both `run_wfi`, the full path, and the skipping path) completes the instruction and skips the rest of the budget; `Machine::wfi` counts it. The tick interrupt is the only wake-up modelled (ASSUMED); `inject_irq` ignores the CPSR I bit like the Python harness (`irq_while_masked` counts it). Probes start after 64 executed instructions and back off to 8192. Verified by `arm-lle-bench`: state hash equal to full execution after every phase.

## Predecode cache

`Machine.predecode` (default true). The cached loop is used when no hook is armed on the current page, the CPU is in ARM state and `pc` is word aligned; everything else (Thumb, hooked pages, misaligned pc) uses `Cpu::step`.

* One `ArmPage` of 1,024 `Entry { insn, mask, op, run }` per guest 4 KiB page, decoded whole on the first fetch. `mask` is the condition table row for the 16 NZCV values (`COND_MASK`), `op` the handler index (`op::*`; data processing = `DP_BASE + form * 32 + opcode * 2 + s`), `run` the number of straight-line instructions to the next block ender (B, BL, BX, SLOW, UNCOND, loads or data processing writing r15). A non-ender never changes `pc` except by `+4`, so a block executes without per-instruction limit, pc and hook checks.
* Handler ABI: `fn(&mut Cpu, &mut B, insn) -> bool`; `true` = fault stored in `cpu.fault`. The handlers call the same `dp_exec` / `ldst_exec` helpers as `Cpu::step`, so the two paths share one implementation of each instruction.
* Invalidation: page flag `HOOK_C` marks a page with a decode. A store to it, or `add_flag(HOOK_X)` on it, pushes the page into `Mem::stale_code` and clears the flag; `run` drops stale decodes before the next fetch. A page with a decode is never hooked. B and BL to another decoded page are followed inside the loop; `until` / `stop_pc` guards are checked at block boundaries and before every ender.
* Equivalence is the requirement, not an approximation: `exec_decoded` against `Cpu::step` on 200,000 random words (registers, cpsr, ordered bus accesses), `Machine::run` with `predecode` on and off on 60 random programs including stores into the code, and the benchmark digests (state hash and message content identical to the old interpreter).
* Measured: about 85 M to 185 M executed instructions per second on the boot (about 2.1x); see [arm0-runtime research](../research/arm0-rust-runtime.md). The remaining cost per instruction is memory and device handling, not dispatch.

## Probe mode

`SysBus.probing` makes `probe_read` / `probe_write` run before the device hooks and set `impure` when a probe touches a device page, an executable hook page or a store that changes memory. `probe_idle` runs the candidate loop on the cached path with probing on and compares registers and CPSR (`same_state`); the old `Probe` bus wrapper is gone, so probes run at normal speed (20 M of 232 M boot instructions run in probes).

## Bounds and logs

Capped: `null_calls` (4096, `null_call_count` counts all), `Mem::auto_mapped` (4096, `auto_mapped_count`), `I2cBus::log` (4096), bus messages (`MESSAGE_CAP` 262,144, `dropped_messages`). `run` stops with `Stop::HookLoop` after 64 hook runs without a CPU step. The arena can be reserved with `Mem::reserve_pages` so on-demand mapping does not reallocate while timed. `BumpNew` never frees: heap and arena grow with each `operator new` (not observed to grow during the benchmark phases, 37 MB after boot).

## Acceptance (`ns4-lle2`)

| Item | Status |
|---|---|
| Opt-in native core, choice recorded | done, D245 |
| Stubs ported, no Python callbacks | done (timer, ports, SPI read return partial: `ns4-lle5` for live layers) |
| Idle / WFI / poll fast-forward | done; state and message content equal to full execution on the 6 benchmark phases |
| Mean below 1.0 ms/tick on real boot + load + keys | done: 0.06 to 0.11 ms (0.20 to 0.34 before the predecode cache) |
| Every tick below 1.0 ms | NOT met: 5 to 9 of 1,900 ticks (0.3 to 0.5 %) over 1 ms, max 1.5 ms, at most 2 in a row, worst backlog 0.53 ms (was 4.8 % of load ticks, 32 in a row, backlog 61 ms) |
| Message latency through a decoupled queue | done (simulation): max 1 ms with a drain per ms, at most 2 ticks / 1,855 messages per drain; the real-thread run is too noisy on the shared machine to quote |
| Instruction budget per ms | bounded, not measured: <= clock x IPC (300 k to 456 k); 200 k stays ASSUMED; sensitivity measured at 100 k, 200 k, 300 k, 456 k |
| Synthetic tests only | done |
