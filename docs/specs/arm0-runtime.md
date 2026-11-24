# ARM0 runtime (`crates/nord-arm`)

Status: v0.1 (2026-11-23), bean `ns4-lle2`, decision [D245](../DECISIONS.md). Measurements: [arm0-rust-runtime](../research/arm0-rust-runtime.md).

Native Rust ARM926EJ-S (ARMv5TE) interpreter that boots the user's OS v1.66 ARM0 image with the hardware the Python harness stubbed. Opt-in crate (no dependencies, not linked by the audio engine). Firmware bytes are never committed: the tests use synthetic in-memory code.

## Layout

| Module | Role |
|---|---|
| `cpu/` | `Cpu`, ARM and Thumb decode (`arm*.rs`, `thumb*.rs`), `alu.rs`, `Bus` trait. WFI (`mcr p15,0,rd,c7,c0,4`) returns `Fault::Wfi`. |
| `mem.rs` | 4 GiB sparse paged memory, per-page flags `HOOK_R/W/X`, unmapped access maps a zero page. |
| `machine/mod.rs` | `Machine::run(max, until, stop_pc)`, `call_preserving`, hooks table, null-call guard. |
| `machine/hooks.rs` | `Hook` enum: `Return`, `BumpNew`, `BoardConfig`, `SpiTransfer`, `SkipInsn`, `ZeroR5`, `BoardPresent`, `DspRead`. |
| `machine/boot.rs` | Firmware load at `0xC0000000`, boot stubs, init array, `run_tick`, timer IRQ 22 injection. |
| `machine/sysbus.rs` | Tick counter `0x80001DC0`, GPIO echo, poll breaker, fake DSP ports (path 1 `0xA0011000`, path 2 `0xA0010000`), port store grouping into messages. |
| `machine/{panel,i2c,uart}.rs` | Front panel on CS2, four 64 KB I2C EEPROMs (program memory), UART2 16550 MIDI input. |
| `machine/grouper.rs` | `Grouper`/`Message`: incremental grouping of port stores into ARM to DSP messages, state kept in the bus so a message split across a drain keeps its first words. |
| `machine/system.rs` | `boot_system`, `send_midi`, `drain_messages(_into)` (completed messages since the last call). |
| `machine/dsp_return.rs` | DSP to ARM SPI reads (`0xC00119A4`, `0xC00117E8`, `0xC0066E64`): SY idle voice mask, then shadow of the ARM's own messages, then 0. |
| `machine/idle.rs` | Idle skipping. |

## Time model

One tick = 1 ms virtual = 200,000 instructions (ASSUMED) followed by one timer IRQ. Interrupts are not CPU exceptions: the AINTC handler `IRQ_TABLE[irq]` is called with all registers and the CPSR preserved (sp lowered by `0x800`, lr = `SENTINEL`), as in the Python harness. The low 64 KB is zeroed between ticks and null calls return 0.

## Idle skipping (exact)

From state `S0`, a probe runs up to `MAX_PERIOD = 4096` instructions on an instrumented bus. If the CPU returns to the same registers/CPSR and, meanwhile, no store changed memory, no device page (tick, poll windows, GPIO, DSP ports) was touched, no hook ran and no unmapped page was mapped, the state equals `S0`. Then `k * L` instructions are added to the virtual counter without executing and the remainder runs for real. `WFI` (both `run_wfi`, the full path, and the skipping path) completes the instruction and skips the rest of the budget; `Machine::wfi` counts it. The tick interrupt is the only wake-up modelled (ASSUMED); `inject_irq` ignores the CPSR I bit like the Python harness (`irq_while_masked` counts it). Probes start after 64 executed instructions and back off to 8192. Verified by `arm-lle-bench`: state hash equal to full execution after every phase.

## Bounds and logs

Capped: `null_calls` (4096, `null_call_count` counts all), `Mem::auto_mapped` (4096, `auto_mapped_count`), `I2cBus::log` (4096), bus messages (`MESSAGE_CAP` 262,144, `dropped_messages`). `run` stops with `Stop::HookLoop` after 64 hook runs without a CPU step. The arena can be reserved with `Mem::reserve_pages` so on-demand mapping does not reallocate while timed. `BumpNew` never frees: heap and arena grow with each `operator new` (not observed to grow during the benchmark phases, 37 MB after boot).

## Acceptance (`ns4-lle2`)

| Item | Status |
|---|---|
| Opt-in native core, choice recorded | done, D245 |
| Stubs ported, no Python callbacks | done (timer, ports, SPI read return partial: `ns4-lle5` for live layers) |
| Idle / WFI / poll fast-forward | done; state and message content equal to full execution on the 6 benchmark phases |
| Mean below 1.0 ms/tick on real boot + load + keys | done: 0.20 to 0.34 ms |
| Every tick below 1.0 ms | NOT met: 4.8 % of load ticks (32 in a row, backlog 73 ms), 0.5 to 1.0 % of key ticks |
| Synthetic tests only | done |
