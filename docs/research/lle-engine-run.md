# LLE engine run: ARM0 plus six DSP cores on one virtual clock (`ns4-lle5`)

- **Checked:** 2026-11-27
- **Sources (local, never committed):** OS v1.66 `target/fw_unpacked/bank0_CODE.bin`, `bank0_SRAM.bin`; EEPROM `target/oled/eeprom.bin` (factory restore, no program of the earlier research); flat SRAM images `target/PNO/core1_sram.bin`, `tools/ns4_dsp_ldr.py ... --sram` of `target/DSP/dsp_image_{0_FX,2_ORG}.bin`, `target/oracle/{sy,smp}.bin` (laid out as in `crates/nord-lle/src/load.rs`). Code: `crates/nord-lle`, `lle_engine_run`. Background: [org-system-run](org-system-run.md), [arm0-rust-runtime](arm0-rust-runtime.md), [sharc-lle-realtime](sharc-lle-realtime.md), [smp-page-refill](smp-page-refill.md), [dsp-to-arm-return](dsp-to-arm-return.md).
- **Question:** Does the composed engine (ARM0 boot, messages routed to PNO core 1, ORG core 2, SY, SMP, FX Core1, FX Core2, passes through the audio bus to a host buffer) run without a fault, deterministically, and what does it cost per subsystem?

Command: `target/release/lle_engine_run.exe --dir IMG --ticks 4000 --runs 2 --note-on 1500:60:100 --note-off 3000:60`.

## Result

| Item | Result | Tag |
|---|---|---|
| ARM0 boot | 232,822,686 virtual instructions to the main loop, 1.3 to 2.1 s wall | EMPIRICAL (busy host, 2 runs) |
| DSP boots | all six cores boot under their presets in 0.15 to 0.23 s | EMPIRICAL |
| Faults | none in 4,000 ticks (0.167 s virtual, 3,616 passes), 2 runs; with the old defaults ORG core 2 faulted at pass 19 (`DM(0x3E021)`), fixed by the `epdma` model for the tone chips | VERIFIED (run) |
| Messages | 15,335 in 4,000 ticks; routed per `(comm, core)`: FX1 1,844, FX2 1,905, PNO1 7,334 (7,292 without the key), ORG2 912, SMP 1,516; counted without a chip: 1,824 (ORG core 1 and PNO core 2, 912 each) | VERIFIED (run); agrees with the per-chip counts of [org-system-run](org-system-run.md) s2 and [arm-dsp-live-link](arm-dsp-live-link.md) |
| Key event | `NoteOn` through the `lle_io` ring reaches UART2; the ARM answered with 42 extra PNO core 1 messages | VERIFIED (run) |
| Determinism | two full runs (boot included): host buffer, ARM state hash and counters give the same digest `9b58996fa9ed4583` | VERIFIED (run) |
| Audio | **silent**: all six nodes have output peak 0.0, host 7,232 frames all zero (key held 1,500 to 3,000) | VERIFIED (run); cause **not diagnosed** |
| Wall time | 4,000 ticks in 11.9 and 13.3 s = 71x and 80x slower than real time; DSP bus 11.3 / 12.7 s, ARM 0.5 / 0.6 s, routing 0.6 / 0.9 ms | EMPIRICAL |
| Per node (run 1) | PNO1 3.86 s, ORG2 2.77 s, SY 2.16 s, SMP 1.15 s, FX1 0.72 s, FX2 0.65 s | EMPIRICAL |
| FX input | 4 frames short at the first FX pass of each core (one hop of latency) | VERIFIED (run) |

The host was at 85 to 100 % CPU from other sessions during the runs; each figure is one run or two, no median, so read wall times as an order of magnitude.

## Why silent (not established)

Candidates, none tested: (1) the factory-restore EEPROM holds no program recalled into the tone chips and 0.17 s of virtual time is short; (2) the taps (`OutTap::Lane` set 0 pair 0 for tone chips) may not be where the firmware writes this chip's audio; (3) the tone chips were never given the full program messages the Python harness sends (`tools/arm_dsp_live.py` also sets probes and windows); (4) ORG core 2 is silent in every state tried ([org-system-run](org-system-run.md) s1). Needs a per-node tap census over a longer run.

## Implications

* The engine composes and is deterministic; it is the base for a null test once a chip produces audio.
* At 71 to 80x over real time the engine is an oracle, as Gate 1 said; the DSP bus is 96 % of the wall time, the ARM 4 %.

## Follow-ups

1. Find out why every tap is zero (program in the EEPROM, longer run, per-node lane census).
2. Live-chip and initial-image return layers.
3. Panel key map for `PanelEvent`, OLED/LED telemetry, 48 kHz resampling.
4. `.npno` refill against a real SMP run (only synthetic here).
