# SHARC exact float kernels: cost per op, before and after

**Checked:** 2026-11-25 (host: AMD Ryzen 9 5900XT, Zen 3, Windows 11, Rust release profile, no `target-cpu`).

**Sources:** `crates/nord-sharc/src/core/aot_fast.rs`, `aot_fadd.rs`, `aot_round.rs` (kernels); `float.rs`,
`float_mul.rs`, `float_conv.rs` (reference kernels, VERIFIED against the CCES simulator, see `sharc-float.md`);
`crates/nord-tools/src/bin/kernel_bench.rs` + `core/aot_fast_bench.rs` (benchmark); bean `ns4-lle3`;
design note on PR #672 (`sharc-lowering-v2.md` section 6: about 13 ns per exact element).

**Question:** the exact 40-bit add and multiply kernels cost about 58 cycles per element in the whole-pass
AOT, so about 3000 compute ops per pass in SIMD (6000 elements) are already x1.7 of the 46.08 us pass budget.
How much of that can the arithmetic itself give back while staying bit-exact?

## Method

- `cargo run --release -p nord-tools --bin kernel-bench -- [RUNS] [reference] [FILTER]` times `fast_add`,
  `fast_sub`, `fast_mul` over 4096 deterministic operand pairs per class. Every output field (value and nine
  flags) is folded into a checksum, so flag derivation is not optimized away. An identity kernel with
  data-dependent flags runs the same fold and its time is subtracted (net column).
- Clock: the thread CPU cycle counter (`QueryThreadCycleTime`), converted to ns with a TSC/wall calibration, so
  time the thread spends descheduled while other sessions build is not counted. Cycles below are ns x 4.5
  (assumed clock, not measured).
- Before = the kernels of `main` (u64 mantissa math, one `round_pack64`), built with the same harness. After =
  this change. Both executables ran **interleaved** (before, after, before, ...), 8 rounds each, each process
  doing 1 warm-up run plus 9 timed runs. Reported: median over rounds of the per-process median, min and max over
  all runs. A gain is called real only when the medians differ by more than the larger min-max band.
- Load check: the driver counts `cargo.exe` + `rustc.exe` processes before every run. The accepted runs had 0 to 1
  (one resident process of another session, constant), the "normal" filter run was repeated once because a first
  attempt overlapped other sessions' builds (3 to 32 processes) and was discarded.
- A first full-table run (all rows in one process) drifted: rows late in a 30 s process ran about 1.6x slower for
  both builds (frequency or SMT state). Short fresh processes per operand class (the `FILTER` argument) avoid it,
  so the table below uses those.

## Results (net ns per op, median [min-max])

| row | before | after | cycles at 4.5 GHz | speedup | verdict |
|---|---|---|---|---|---|
| add/normal/rne | 6.56 [6.41-7.27] | 3.30 [3.18-3.52] | 30 -> 15 | 1.99x | real |
| sub/normal/rne | 6.62 [6.43-9.46] | 3.28 [3.21-4.30] | 30 -> 15 | 2.02x | real |
| mul/normal/rne | 6.75 [6.59-10.22] | 2.74 [2.64-2.96] | 30 -> 12 | 2.46x | real |
| add/normal/trunc | 6.51 [6.32-6.98] | 3.29 [3.16-4.31] | 29 -> 15 | 1.98x | real |
| add/normal/r32 | 6.56 [6.38-7.04] | 3.29 [3.18-3.46] | 30 -> 15 | 1.99x | real |
| mul/normal/trunc | 4.24 [4.12-4.56] | 2.70 [2.64-3.97] | 19 -> 12 | 1.57x | real |
| mul/normal/r32 | 6.86 [6.71-11.30] | 2.74 [2.64-2.93] | 31 -> 12 | 2.50x | within noise (band set by one outlier run) |
| add/same_exp/rne | 6.63 [6.40-8.25] | 3.31 [3.22-5.15] | 30 -> 15 | 2.00x | real |
| add/gap_1_8/rne | 6.62 [6.48-7.31] | 3.29 [3.17-3.64] | 30 -> 15 | 2.01x | real |
| add/gap_24_60/rne | 7.84 [7.71-8.15] | 3.26 [3.21-3.49] | 35 -> 15 | 2.40x | real |
| sub/cancel/rne | 7.70 [7.60-8.04] | 3.22 [3.18-3.45] | 35 -> 14 | 2.39x | real |
| add/gap_24_60/trunc | 6.17 [5.92-6.62] | 4.22 [4.08-4.71] | 28 -> 19 | 1.46x | real (TRUNC with a gap above about 31 lands on a boundary and takes the exact integer kernel; RNE and RND32 stay at 3.3 ns) |
| add/near_overflow/rne | 5.91 [5.76-6.86] | 3.23 [3.18-3.79] | 27 -> 15 | 1.83x | real |
| add/near_underflow/rne | 6.00 [5.87-6.65] | 3.27 [3.20-3.37] | 27 -> 15 | 1.83x | real |
| add/mixed/rne (25% special operands) | 5.89 [5.77-6.34] | 3.75 [3.65-3.92] | 27 -> 17 | 1.57x | real |
| add/zero_operand/rne | 4.14 [4.04-4.23] | 4.00 [3.95-4.78] | 19 -> 18 | 1.03x | within noise |
| mul/zero_operand/rne | 3.80 [3.71-4.27] | 3.46 [3.37-4.91] | 17 -> 16 | 1.10x | within noise |
| mul/near_overflow/rne | 4.12 [4.08-6.38] | 4.64 [4.54-4.81] | 19 -> 21 | 0.89x | small consistent regression: +0.5 ns in every run, 0.89x of median (the cold exception path is slower; the min-max bands overlap only because of one outlier in the baseline) |
| mul/near_underflow/rne | 4.33 [4.18-6.71] | 4.00 [3.92-6.30] | 19 -> 18 | 1.08x | within noise |
| mul/mixed/rne | 4.58 [4.50-4.80] | 3.75 [3.68-4.61] | 21 -> 17 | 1.22x | within noise |

`sub` rows equal their `add` rows (a subtraction only flips one sign bit before the kernel). The `u128`
reference kernels (`float::add`, `float::mul`) cost 13 to 26 ns on the same operands (normal-range rows, one run,
`kernel-bench 9 reference normal`), so `main`'s kernels were already about 3x better than the reference.

Target of at least 2x on normal-range operands: **met for add and sub** (1.98x to 2.02x on `rne`, `trunc`, `r32`
normal operands, 2.0x to 2.4x by gap class), **met for mul** in round-to-nearest (2.5x), **not met for mul in
TRUNC** (1.57x: the old kernel skips the rounding branch there and is already cheap). Operands that hit the cold
paths (zero, denormal, infinity, NaN, overflow or flush results) gain nothing: unchanged within noise, except
`mul/near_overflow`, which is about 0.5 ns slower. "Cycles" in this note are ns x 4.5 (assumed clock, not
measured cycle counts).

## What changed

1. **Add on the double unit** (`aot_fadd.rs`). A register float `s|e8|f31` shifted left by 21 is a double
   scaled by `2^-896`; one `addsd` of two such doubles is the register add with a single rounding at 53 bits. The
   result is rounded to 32 (RND32: 24) bits by integer arithmetic on the double's bits: add `half - 1 + t`
   (`t` = last kept bit), mask the fraction field, and a carry out of the fraction increments the exponent field by
   itself. No lz-count, no variable shift, no 128-bit math.
2. **Exactness argument** (also in the file header). `s` is the double nearest to the exact sum `v`, and every
   rounding boundary of the register format is a double, so `s` and `v` round alike unless `s` is exactly on a
   boundary: the midpoint (`low == half`) for round to nearest, the value itself (`low == 0`) for TRUNC. Then
   the case is decided by the exact integer kernel, except when the exponent gap is at most 20, where the sum of two
   32-bit significands fits 53 bits and `s = v`. A result exponent field outside 1..=254 (zero, flush, overflow)
   also takes the integer kernel.
3. **Integer kernel** (`add_exact`, out of line): swap by one compare of the 39 magnitude bits, clamped alignment
   shift (gap >= 62 is "all sticky"), sticky from a trailing-zero count, branchless add/subtract, normalize with a
   leading-zero count, one rounding.
4. **Multiply** (`fast_mul`): the 32x32 product has its top bit at 63 or 62, so normalization is `p + p` or
   nothing (no lz-count), and the rounding is the same single add as above (`round_norm` in `aot_round.rs`). Round
   to nearest even is `mn + (2^(sh-1) - 1 + t)` with `t` = sticky or last kept bit; the carry out of bit 63 is the
   significand overflow.
5. **Cold results return in registers** (`Packed`, `pack`/`unpack`). A cold function returning the 24-byte `Out`
   makes the compiler keep the hot path's `Out` in the same memory slot (stores, then byte reloads of every
   field). Returning `(u64, u64)` keeps both paths in registers; this alone was worth about 2 ns per add.
6. `fast_from_int` uses the same `round_norm`; semantics unchanged.

Tried and dropped: variable-shift-free selects for the width (LLVM turned them back into `shl cl`, 3 micro-ops
on x86 without BMI2); `bsf`-free sticky variants and a float-exponent leading-zero trick (not faster; `bsr`
and `tzcnt` cost micro-ops on Zen 3 but the double path avoids them anyway); mul on the double unit (the 64-bit
product does not fit 53 bits; the integer kernel is already at 12 cycles); a batched `fast_add2` / `fast_mul2`
pair form (the loop already overlaps independent elements through out-of-order execution, and the scalar kernels
have no cross-lane work, so a pair call compiles to the same instructions; not built).

## Verification

- `aot_fast.rs` tests unchanged and passing (random operand classes, cancellation and gaps, overflow and flush,
  `FLOAT`, state effects of the native ops).
- `aot_fast_sweep.rs` (new): 600 000 random pairs over operand classes, every exponent gap 0..=70 x 10
  boundary fractions x all sign pairs x both operand orders, single-bit significands at every tie position
  (add gaps 0..=40, mul `2^i x 2^j`), a full grid of exponent edges (1, 2, 3, ..., 253, 254) x boundary fractions x
  extension bytes (overflow, flush), and 200 000 cancellations to every leading-bit position. Each case runs
  all four TRUNC x RND32 modes and compares every `Out` field with `float::add` / `float::mul`.
- `gaps_around_the_exact_double_limit_...`: gaps 19..=23 x sparse fractions, add and sub, both orders, all modes
  (a gap limit of 21 now fails 2 tests, 22 fails 4).
- Mutation check: raising the exact-sum gap from 20 to 28 makes five tests fail, so the sweeps reach the
  inexact-double branch; setting it to 200 fails ten.
- `aot_round.rs` and `aot_fadd.rs` carry their own unit tests (rounding ties, carry, flush/overflow, the
  double encoding).

Assumption (ASSUMED, flagged in `aot_fadd.rs`): `f64` addition is IEEE-754 round-to-nearest, as on SSE2 and AArch64.
A 32-bit x86 target using x87 extended precision would double-round and fail the sweeps.

## Implications

- Kernel cost per element, normal range: add 30 -> 15 cycles, mul 30 -> 12 cycles (about -3.3 ns and -4.0 ns).
- Against the x1.7 floor of PR #672 (6000 elements x 13 ns = 78 us per pass against 46.08 us): the 13 ns figure
  is the whole element, of which this benchmark isolates 6.6 ns (30 cycles) of kernel. Taking the mean saving of
  an add/mul mix (3.65 ns), the exact path drops to 6000 x 9.35 ns = 56 us, **x1.22 of budget**. That removes
  28% of the exact cost and 69% of the 0.7 budget excess. This is a projection, not a measurement: it assumes
  the 13 ns per element of PR #672, a 50/50 add/mul mix, all-normal operands (no zero, denormal, special or
  overflow results) and that the kernels are the only difference; there is no whole-pass measurement. Real-time exact execution of this workload is still
  not reached by arithmetic alone: the remaining 6.4 ns per element is the glue around the kernel (register
  read of the 40-bit value with its extension tag, `set_f40`, flag and STKY updates), which this change does
  not touch, and the figure assumes the kernels are the only difference between the 13 ns and the benchmark.
- Normal-range add is now at the cost of an `addsd` plus about 40 integer ops; further gains need fewer ops per
  element around it (batching the flag updates, keeping 40-bit registers as doubles across a block) rather than a
  faster rounding.

## Follow-ups

- Measure the whole-pass AOT (`lle_sharc_bench`) with these kernels once `aot2` calls them; the x1.22 above is a
  projection from per-kernel numbers.
- The glue around the kernel (6.4 ns per element by subtraction): flags from `Out` are 10 booleans folded into
  ASTAT and STKY every op; an `Out` packed in two registers (`Packed`) that the lowering consumes directly would
  avoid the unpack on the hot path.
- Cold paths (`mul` overflow, zero operands) are about 0.5 ns slower or unchanged; a short inline fast path for
  one-zero-operand `add` (the other operand passes through) is possible if profiles show zero operands are common.
- A 128-bit-free `fast_from_int` micro-benchmark row is missing (the `FLOAT` op is rarer).
