# ARM0 native Rust runtime: wall-clock per 1 ms tick

**Checked:** 2026-11-24 (first run 2026-11-23, re-measured after review).

**Sources:** `crates/nord-arm` + `crates/nord-tools/src/bin/arm_lle_bench.rs` (this PR); user OS v1.66 images `target/fw_unpacked/bank0_CODE.bin` (loaded at `0xC0000000`) and `bank0_SRAM.bin` (read-only, never committed); a 256 KB EEPROM image built by `tools/arm_program_load.build_image` from `target/oled/eeprom.bin` plus factory program `B3 Chords n Solo.ns4p`; Python reference `tools/arm_org_stream.py`, `tools/arm_steady.py`, `tools/arm_dsp_return.py`; [pno-jit-arm-profile](pno-jit-arm-profile.md) (Python/Unicorn cost); [dsp-to-arm-return](dsp-to-arm-return.md).

**Question:** Can ARM0 run the real OS v1.66 boot, program load and key events inside Rust below 1.0 ms wall-clock per 1 ms virtual tick (`ns4-lle2`, Gate 1a)?

**Answer in one line:** the mean is far below 1 ms (0.20 to 0.34 ms), but the tail is not: 4.8 % of load ticks run over 1 ms, in runs of up to 32 consecutive ticks (worst backlog 73 ms). The criterion "< 1 ms on every tick" is NOT met.

## Method

Machine: AMD64 Family 25 Model 33 (AMD, Zen 3 class, 32 logical processors), Windows 11, release build, one thread. The benchmark prints the CPU model itself.

```
cargo run --release -p nord-tools --bin arm-lle-bench -- \
  <bank0_CODE.bin> <bank0_SRAM.bin> <eeprom_b3.bin>
```

1. Boot to the main loop `0xC00A0248` with the full board attached (panel, 4 I2C EEPROMs, UART2) and idle skipping on.
2. Real timer on, DSP port logging and the DSP return hooks installed, 64 MB of arena reserved so on-demand mapping cannot reallocate during the run.
3. Six phases: load 800 ticks, idle 300, one key on 200, one key off 200, eight keys on 200, eight keys off 200 (the key phases are 5x longer than the first run, 40 ticks gave 2.5 % = one tick). One tick = `200000` ARM instructions of virtual time (ASSUMED, `INSNS_PER_TICK` of `tools/arm_steady.py`) plus one timer IRQ.
4. **The timed region of each tick contains** the MIDI send (`send_midi`, first tick of a phase, includes the UART ISR), the tick run with idle skipping, and `drain_messages_into` (message hand-over plus the DSP return sync).
5. The same phases run again with full execution (no skipping) on a clone. The machine state hash and the full message list (path, comm, words, pc) are compared after every phase.

## Results (`target/lle/bench_run3.txt`)

Boot: 232,625,931 virtual instructions (231,624,565 executed, 999,786 skipped), 4.0 s wall. After boot: arena 37 MB, heap_top `0x911F87D0`, 4413 auto-mapped pages, 25 null calls, 0 WFI.

| Phase | Ticks | Mean ms | p50 | p99 | Max | Ticks > 1 ms | Longest run > 1 ms | Worst backlog ms | Executed insns/tick | Messages |
|---|---|---|---|---|---|---|---|---|---|---|
| load | 800 | 0.337 | 0.192 | 3.622 | 5.047 | 38 (4.8 %) | 32 | 73.3 | 12,481 | 15,337 |
| idle | 300 | 0.199 | 0.208 | 0.308 | 0.421 | 0 | 0 | 0 | 3,384 | 3 |
| key1 on | 200 | 0.198 | 0.207 | 0.333 | 0.443 | 0 | 0 | 0 | 3,288 | 126 |
| key1 off | 200 | 0.213 | 0.214 | 0.352 | 1.262 | 1 (0.5 %) | 1 | 0.26 | 3,448 | 98 |
| key8 on | 200 | 0.219 | 0.216 | 0.574 | 2.603 | 2 (1.0 %) | 1 | 1.60 | 3,365 | 1006 |
| key8 off | 200 | 0.206 | 0.184 | 0.349 | 2.853 | 2 (1.0 %) | 1 | 1.85 | 3,426 | 969 |

"Backlog" = work behind real time if ticks are consumed at one per ms and slow ticks carry over (a decoupled ARM0 thread).

Full execution (no skipping) costs 2.8 to 3.1 ms per tick (200,000 instructions, about 65 to 70 MIPS). Skipping removed 366 M of 380 M instructions (96 %).

**Equivalence.** After all six phases the state hash of the skipping run equals the full run, and the message lists are identical in content (15,337 / 3 / 126 / 98 / 1006 / 969 messages, every field). `WFI` count is 0 in this workload, so WFI is covered by a unit test (`wfi_has_one_semantic_in_the_full_and_the_skipping_path`), not by the benchmark.

## What the over-1-ms ticks are

43 ticks exceeded 1 ms. All 43 executed the full 200,000 instructions for real (nothing to skip). None is device-window polling (device-window reads in every one of them: 0 to 2, the poll breaker is not involved) and none grew the memory arena (the arena was reserved up front and did not grow during the run; heap_top did not move). They are real firmware work during program load (and a few key events), at about 3 to 5 ms per 200,000 instructions. The 8 slowest are all load ticks 99 to 176 (3.9 to 5.0 ms).

## What this means for real time

* **Not met:** "< 1 ms on every tick". Locked 1:1 to the audio callback, 4.8 % of load ticks and 0.5 to 1.0 % of key-event ticks miss.
* **Mean is fine:** 0.20 to 0.34 ms per tick, 66 to 80 % of a tick free on average.
* **The tail is unproven for real time.** The longest run of slow ticks is 32 in a row during program load, backlog 73 ms (the runs of the key phases are single ticks, backlog under 2 ms). A decoupled ARM0 thread with a message queue absorbs the key-event tail easily; whether the 73 ms load backlog is acceptable (program-load latency, a queue of a few thousand messages) is a product decision, not measured here. Message latency under the queue was not measured.
* **Go / no-go:** GO for continuing with this core as the ARM0 runtime (mean and steady state, key events). NO-GO for claiming real-time on every tick or for a lockstep design. Remedy when needed: a faster interpreter (predecode cache; about 5x needed for the slowest load ticks).

## Findings from the review of the first run

* A message split across a tick boundary lost its first words (the grouping state was rebuilt on each drain). Fixed: the grouper lives in the bus; test `a_message_split_across_two_drains_is_delivered_whole`. The first run's message counts were equal only because the full-execution comparison hit the same split points; content equality did not exist then.
* Stub returns now set the instruction set from `lr` bit 0 (test `stub_return_takes_the_instruction_set_from_lr`).
* `WFI` has one semantic in both run paths (the instruction completes, the rest of the budget is skipped); `Machine::wfi` counts it.
* Unbounded logs are capped (`null_calls`, `auto_mapped`, `I2cBus::log`, bus messages), `DspReturn::read` and the drain no longer allocate per call.

## Not verified

* `INSNS_PER_TICK = 200000` is ASSUMED. If the real ARM926 retires fewer instructions per ms, the heavy ticks shrink in proportion.
* Boot null-call count: Rust 747, Python (`tools/arm_steady.py`, re-run today) 762, with identical stop address; running the Rust boot to the next 200k boundary does not change it (747) and the Thumb-return fix did not change it. Cause not found.
* Load-phase message count 15,337 against 7,340 in the Python research (`tools/arm_org_stream.py`). Not investigated.
* DSP return layers beyond the SY idle mask and the shadow of the ARM's own messages (live chip, initial image) are not ported (`ns4-lle5`); no key phase reads the SPI return path often enough to measure it here (`DSP return reads` was 0 in the run).
* `inject_irq` runs the handler even if the CPSR I bit is set (as the Python harness does); this happened once (`irq_while_masked`). `Cpu::mode_changes` was 0 (no register banking needed in this workload).
* Only OS v1.66 `bank0` (ARM0) and one program were measured, on one machine.

## Implications

* A native interpreter is enough for the mean and for key events; decision [D245](../DECISIONS.md).
* Idle skipping needs no firmware knowledge, so it keeps working on other OS images; it is exact on the 6 benchmark phases (state and message content), not proven in general.
* Real-time use needs a decoupled ARM0 thread plus message queue, and a faster core if the load phase must also keep up.

## Follow-ups

* Predecode cache (or threaded interpreter) for about 5x the instruction rate; re-run `arm-lle-bench`.
* Measure message latency through the queue and size it from the 73 ms backlog.
* Measure the real instruction budget per ms to replace the ASSUMED 200,000.
* Find the 747 against 762 null-call difference and the load-phase message count difference.
* Port the live-chip and initial-image return layers (`ns4-lle5`).
