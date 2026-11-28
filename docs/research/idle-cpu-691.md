# Idle CPU floor: where it goes and the first bit-identical skip (issue #691)

- **Checked:** 2026-11-28 (`main` @ `9199e66`, 4 vCPU x86_64 container, release build)
- **Sources:** `ns4 bench --scenario worst|idle --breakdown`, `valgrind --tool=callgrind` on `ns4 bench --scenario idle` (debuginfo build), `crates/nord-core/src/fx/{amp_eq,fw_eq,fw_eq_tables}.rs`, `engine/mix.rs`, `organ/tonewheel.rs`.
- **Question:** With no key held and zero output, 7 racks and layers cost ~6.7 ms per 128-frame block on a Ryzen 3 PRO 2300U (deadline 2.667 ms). What is the cost, and which part can be skipped without changing one output bit?

## 1. Baseline on `main`

`ns4 bench --scenario worst --blocks 300`: hash `0xe1ec8bb293c35c33` (the issue's `0x10b5ed24...` is from commit `0bfe89a3`, older). Breakdown (mean ms per block, this machine, noisy):

| Row | ms |
|---|---|
| full worst case | 2.100 |
| all FX racks and master comp off | 1.706 |
| no keys held, all FX racks on (idle) | 1.053 |
| Organ A / B only, 88 keys, FX off | 1.014 / 0.704 |
| Piano A / B only | 0.627 / 0.604 |
| Synth A / B / C only | 0.596 / 0.581 / 0.640 |

## 2. Idle profile (callgrind, instructions, steady state)

Setup (wavetable init, layer construction) is ~25% of a short run; the table is the render loop only.

| Part | Share of render (before) | Note |
|---|---|---|
| `LayerFxChain::process_frame_stages` (7 racks) | ~91% | everything below is inside it |
| `logf` / `exp`, `FwEqStage::set`, `freq_index_of_hz` | ~40% of racks | pure functions of the EQ knobs, re-derived **every frame** |
| firmware amp + cabinet (`RateMatch`) | ~25% | stateful IIR, runs on zeros |
| LFO `sinf` / `powf` (modulation) | ~8% | free-running phases |
| layers (`organ`, `piano`, `synth` `render_frame`) | ~9% | tonewheel `need_signal` is a small part of this |

The layers the issue names are not the floor; the racks are.

## 3. Skip implemented: memoized EQ words

`process_fw_eq` called `freq_index_of_hz` (128 `ln`) and `FwEqStage::set` (`ln`/`exp`, table scans) per frame, 7 racks x 48 kHz. Both are pure in `(bass, mid, treble, freq, Hz)`. `FwEqStage` now keeps the last input bits and result. A changed bit refreshes; the words written into the tone stack are the identical `f32` values, so output cannot change.

Proof (`fx/tests_fw_eq_memo.rs`, memo on vs the `memo = false` reference path, bit compare of every frame): moving knobs and Hz, LP24 switch, 4 s of silence then a note-on.

## 4. Result

| | before | after |
|---|---|---|
| idle, instructions per 128-frame block (callgrind) | 13.78 M | 7.87 M (-43%) |
| `--scenario idle` mean ms/block, median of 5 interleaved runs, this machine | 1.364 | 0.763 |
| `--scenario worst --blocks 300` hash | `0xe1ec8bb293c35c33` | `0xe1ec8bb293c35c33` |

## Implications

- The remaining idle floor is the firmware amp + cabinet and the modulation LFOs of each rack. Skipping them needs a proof that the filter state has decayed to exact zero (denormal and rounding tails make this non-trivial), and the LFO phases must be advanced analytically to keep phase continuity.
- FTZ/DAZ would remove denormal cost but changes numeric results and is not equivalent on aarch64; not adopted.

## Follow-ups

- Skip racks with exactly-zero state and input (amp/cabinet), with the same on/off bit-identical test.
- Tonewheel `B3Wheels` / `FarfisaWheels` / `Firmware` sources: analytic phase advance when no key is held.
- SIMD with runtime dispatch and block processing (after the floor is re-measured).
