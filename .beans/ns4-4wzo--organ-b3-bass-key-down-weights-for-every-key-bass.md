---
# ns4-4wzo
title: 'Organ: B3 Bass key-down weights for every key; Bass stage blockers'
status: in-progress
type: task
created_at: 2026-11-14T13:04:37Z
updated_at: 2026-11-14T15:00:00Z
---

## Context
B3 Bass (model 5) table and wiring (ns4-b511 follow-up, organ-farfisa-bass s9).

## Higher Goal
1:1 audio parity of the organ models.

## Acceptance Criteria
- [x] Key-down weights of model 5 decoded for every key from the ARM (organ_bass_keys.py, b3_bass_keys.rs, bass-keydown.csv)
- [x] Oracle chain tool for the Bass processor (organ_bass_proc.py); window 1 inaudible, window 2 carries the bass
- [x] State decoded (s11): windows additive per key, `G = phi(S)` accumulator, output `1 - 0.84 G`, line-gain table at G = 0; the per-key lattice words are not the cause
- [x] Processor part with oracle chord goldens (1, 3, 10 keys, 3 orders): max 1.45 dB, rms <= 0.65 dB (`b3_bass_state.rs`, not wired)
- [x] Generator weight rule decoded: `T[note][n]` over held writes, key-up included (`b3_bass_gen.rs`, `organ_bass_gen.py`, `bass-gen-weights.csv`)
- [x] Opt-in Bass stage wired (`--b3-bass-processor on`, `b3_bass_stage.rs`), chord 60 64 67 within +1.1 to +2.5 dB per line
- [x] Instruction-level Python reference of `0x122296` (one call, 868 of 868 trace rows and 0 of 16,128 memory words, `tools/sharc_listing_vm.py`, s13)
- [x] Generator `0x121BBE` in the VM and the multi-call replay of generator + processor bit-exact over whole chord scripts (4696 + 3 x 1200 calls, `tools/sharc_listing_ext.py`, `tools/organ_replay.py`, s14)
- [x] Within 1 dB on the full chain: decoded generator + processor run in Rust (`organ/b3_bass_fw/`, bit exact over 48 recorded calls) and the stage reads at most 0.0036 dB off the 46 oracle chord lines (s15, D71); the FIR is replaced
- [x] Farfisa processor `0x122B85` on the machine (jumps, ALU flags; bit exact 24 + 12 calls; stage within 0.0039 dB on 48 chord lines; s16, D74)
- [ ] Drawbar / percussion dependent key tables (capture non-888888888 drawbars and percussion), key-up capture and the `G` resync, a real time path (TD-15)
- [ ] B3 Bass hardware clip comparison and default decision (the B3 clip plays the B3 model: delta 0.00 dB by construction)

- [ ] Real time (TD-15): compiled in place executor, zero and `float` fast paths, branch free rounding (s18, D192): Bass 1.8 to 0.65, Farfisa 1.35 to 0.45 s CPU per second of audio, bit identical; not within the 0.8 ms block budget with a full program (next: straight line Rust with registers in locals, SIMD over the PEs, a closed form of the Farfisa processor)

## Out of Scope
Bit-exact 0x122296 model (still open).

## Summary of Changes
Weights table, then the state model (`b3_bass_state.rs`, `tools/organ_bass_state.py`, data CSVs) and the correction that the section words are not read; then the generator weight rule (s12) and the opt-in Bass stage; default render unchanged, `emu_vs_hw.py` bass band delta 0.00 dB by construction (B3 clip is the B3 model).

s13: `tools/sharc_listing_vm.py` + `tools/organ_listing_check.py` run the ADI listing of `0x122296` bit-exact against `sharc-chain` (SIMD pair rule, I1 broadcast loads, 40-bit float). Still open: generator listing for multi-call replay, the Rust recursion (Bass stage unchanged, +1.1 to +2.5 dB), branches for the Farfisa `0x122B85`.

s14: the generator listing runs in the VM and the multi-call replay is bit-exact (4696 calls of 60 64 67, 1200 calls of a 10 key chord and of two drawbar settings). The processor is linear but time varying (scanner delay taps), so the EMPIRICAL FIR stays until the Rust recursion with the scanner phase exists; drawbar / percussion key tables, `G` resync after key-up, `0x122B85` branches, Twin amp types 7 to 9 and the cabinet bank are not done.

s15: `tools/sharc_listing_rust.py` decodes the listings into rows, `organ/b3_bass_fw/` runs them (40 bit float, SIMD pairs, circular buffers), the Bass stage feeds it the key-down state and resamples from the DSP rate. Oracle chord lines within 0.0036 dB (the FIR read +1.1 to +2.5 dB); offline only (3.6 s of CPU per second of audio). Open: drawbar / percussion tables, key-up, speed, Farfisa `0x122B85` (branches, flags, model 2 install).

Farfisa processor on the firmware machine (jumps and AZ / AN flags, `organ/farfisa_fw/`, `farfisa_keys.rs`, `tools/organ_farfisa_fw.py`), 64 bit float fast path, CPU benchmark example (s16, s17, D74).
