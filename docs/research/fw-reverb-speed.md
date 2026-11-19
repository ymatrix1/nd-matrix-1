# Firmware reverb port: where the time goes and what a bit-identical speedup reaches

- **Checked:** 2026-11-19 (branch `perf/fw-reverb-speed` on `main` @ `6a8ba992`)
- **Sources:** `crates/nord-core/src/fx/{fw_ext40,fw_leslie_core,fw_leslie_ops,fw_rvbm,fw_rvbm_steps_*}.rs`, the new bench `cargo run --release -p nord-tools --bin fx_reverb_speed`, `ns4 bench --gui`, a sampling profile of the bench (a Python helper that suspends the bench thread, reads `RIP` and symbolises through `dbghelp` against the PDB; `samply` hung on this host, see Follow-ups), [gui-audio-glitches](gui-audio-glitches.md), TD-14.
- **Question:** The GUI Reverb checkbox (firmware reverb port, `--firmware-fx-ports reverb`) makes the crackle clearly worse. How much cheaper can the port get **without changing one output bit**, and what costs the time?

## 1. Method

`fx_reverb_speed` runs each `FwReverb` / `FwReverbMulti` type (0, 1, 2..5, 7..11; 6 has no captured block) on a fixed xorshift noise stimulus (1 s noise, 1 s silence, repeating), reports **ms per 480 frame block** (min over `--repeat` runs) and an FNV-1a hash of every output bit. The host runs other jobs (python at 100 percent for hours, builds): one binary varies by 30 to 60 percent between runs, so every number below is the **min of 28 runs, binaries interleaved**, same stimulus. Treat ratios, not absolute ms.

## 2. Profile (before)

Sampling profile of the multi-tap path, type cycle 2..11 (about 1300 to 2300 samples):

| Share | Where |
|---|---|
| 26 percent | `rvbm_core` (generated steps: loads, stores, DAG moves, register copies) |
| 24 percent | `rvbm_wrapper` (six fs/4 biquads on the 4 frame block, mix) |
| 19 percent | `E40::add` (outlined call, 128 bit fallback behind an f64 two-sum test) |
| 18 percent | `E40::mul` (outlined call, unpack, `u64` mantissa product, repack) |
| 3 percent | `E40::sub` |
| 12 percent | small helpers (`slot` window search, `fpack`) |

The same machine on `f32` registers (`FwReverbMultiF32`, different output) runs in 0.44 ms against 0.85 ms per block, so about half of the cost is the 40 bit arithmetic and half is the register machine. About 2 ns per SHARC instruction (two lanes) is already close to what this interpretation style allows.

## 3. Changes (all bit-identical)

| Change | Why it is exact | Effect |
|---|---|---|
| `E40::mul`: f64 product, then `round_exact` (32 bit round, ties to even, flush); the integer path only when the f64 product sits exactly on a 32 bit tie (low 21 bits equal `1 << 20`, about 2^-21 of products) | the f64 product has one 53 bit rounding; a 32 bit boundary is representable in 53 bits, so a monotone rounding cannot cross it except by landing exactly on it, and that case takes the old integer path (`fast_tests`: 100 000 tie and non-tie products equal to the integer `pack`) | mul no longer unpacks both operands |
| `E40::add`: operands within 20 binades (exponent field difference test) skip the error-term test | two 32 bit mantissas at distance `d` span `d + 33` bits, so the f64 sum is exact for `d <= 20`; `d = 21` already needs 54 bits (`fast_tests` fails when the limit is 21) | replaces a six flop dependent chain by three integer ops |
| `E40::add` / `mul` / `sub`, `round_exact`: `#[inline(always)]`; slow paths (`add_far`, `mul_tie`) outlined and cold | inlining only | removes the call per arithmetic op |
| `Core::slot` returns the words left in the window; `ld` / `wr2` use one window search per pair when both words are in it (else the old per-word path) | same addresses, same windows, same search order; `pair_access_at_a_window_edge_splits_like_two_single_accesses` covers adjacent windows and window ends | one lookup instead of two per pair access |

Tried and dropped: `#[inline(always)]` on the `Core` accessors and the `lanes` wrappers (the generated `rvbm_core` of about 1,800 lines becomes one huge function: 15 to 20 percent slower and a 2 minute compile); the same on only `fadd` / `fsub` / `fmul`: slower too.

## 4. Results

`fx_reverb_speed`, ms per 480 frame block, min of 28, both columns from one interleaved run (host loaded; the quieter earlier run gave sum 9.28 before, 6.98 after, 5.87 after with LTO):

| kRvbType | before | after | ratio | hash equal |
|---|---|---|---|---|
| 0 Spring | 1.553 | 1.034 | 1.50 | yes |
| 1 Spring 2 | 1.403 | 1.119 | 1.25 | yes |
| 2 Booth | 0.652 | 0.608 | 1.07 | yes |
| 3 Booth Predelay | 1.025 | 0.686 | 1.49 | yes |
| 4 Room | 1.040 | 0.834 | 1.25 | yes |
| 5 Room Predelay | 1.021 | 0.812 | 1.26 | yes |
| 7 Stage Chorale | 1.032 | 0.845 | 1.22 | yes |
| 8 Hall | 1.016 | 0.833 | 1.22 | yes |
| 9 Hall Chorale | 1.014 | 0.798 | 1.27 | yes |
| 10 Cathedral | 0.987 | 0.824 | 1.20 | yes |
| 11 Cathedral Chorale | 1.010 | 0.819 | 1.23 | yes |
| sum | 11.754 | 9.214 | **1.28** | all |

The quieter run: sum 9.28 to 6.98, **1.33x**. Full engine (`ns4 bench --gui`, Horizon Whl): `render_hash` `0x9494f3d2516e334e` with `--firmware-fx-ports reverb` and `0x47576812a4ed6e44` with `--firmware-fx`, each equal before and after. The GUI bench wall times were not usable for a before/after: the same binary measured 6.1 to 10.9 ms across runs of one session (other jobs on the host), so no figure is claimed for it.

**Target missed:** the goal was 2x; this branch reaches about 1.3x on the reverb port. What remains is the register machine itself (a 16 byte `V` enum per lane, 32 byte pairs copied between the register file and the ops, a linear window search, two lanes of 40 bit arithmetic per op).

### Release profile (not changed here)

Building with `lto = "fat"` and `codegen-units = 1` (`CARGO_PROFILE_RELEASE_LTO=fat CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1`, hashes equal) is another 1.15 to 1.2x: sum 9.28 / 6.98 / 5.87 (before / this branch / this branch + LTO), 1.58x against before. `codegen-units = 1` alone gave nothing. It lengthens every release build (about 1 min 10 s for `nord-tools` here) and changes the GUI binary, so it is left to the owner.

## Pinned output

`tests_fw_reverb_pinned.rs` pins an FNV-1a hash of every output bit for types 0 and 1 and for types 2..5 and 7..11 in each of the three colours (29 runs of 30 000 frames, noise burst then silence at amount 127), taken from the implementation before this change. They are regression pins, not oracle data; the oracle checks are `tests_fw_rvb_golden.rs` and `tests_fw_rvbm_golden.rs`, which pass unchanged.

## Implications

- The firmware reverb is about 25 percent cheaper at no change in sound. A scene with seven firmware reverbs stays near 0.8 ms per reverb and block on this host (about 5.7 ms of a 10 ms block), so the crackle margin improves but is not removed.
- LTO would give the next 15 to 20 percent without code changes; a 2x on top of the base needs a different executor, not more tuning of this one.

## Follow-ups

- Owner decision: `lto = "fat"` and `codegen-units = 1` in `[profile.release]` (about 1.15 to 1.2x more on the reverb, longer builds).
- A leaner executor: keep the registers of a straight-line step in locals (a change to `tools/fx_rvbmulti_rust_gen.py`), a two-lane SIMD `E40` mul and add (SSE2 f64 pairs plus an integer round), a direct-mapped address table instead of the window search. Each must keep the pins of `tests_fw_reverb_pinned.rs`.
- Sharing one reverb between racks with identical settings and the opt-in idle skip ([gui-audio-glitches](gui-audio-glitches.md), D161) cut more than any tuning here.
- `samply record` hung on this host (UAC / xperf stage) on its second and later runs; a 60 line Python sampler (suspend thread, read `RIP`, `dbghelp` symbols) works without admin rights for a process of the same user.
