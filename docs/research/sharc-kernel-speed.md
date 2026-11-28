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

- `cargo run --release -p nord-tools --bin kernel-bench -- [RUNS] [reference] [pair] [FILTER]` times `fast_add`,
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
pair form (first judged to compile to the same instructions and not built; built and measured on 2026-11-27, see
"Paired SIMD kernels" below: no gain).

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

## Paired SIMD kernels (PEx and PEy together)

**Checked:** 2026-11-27, same host as above (Ryzen 9 5900XT, Windows 11), shared with other sessions.

**Question:** in SIMD mode every compute op runs on both elements with the same op and MODE1. Does a paired kernel
(`fast_add2`, `fast_sub2`, `fast_mul2`) on one 2 x 64-bit vector remove part of the 19 to 25% compute share of a whole
pass?

**Answer: no.** The pair kernels are bit-exact (tests below) but not faster per element, and wiring them into a whole
pass made it slower or left it within noise. They stay in the tree as API with tests; no call site is recommended.

### What was built

Files under `crates/nord-sharc/src/core/`; none of `aot_pe.rs`, `aot_mem.rs`, `aot_whole.rs`, `aot_hot.rs`,
`aot_glue.rs`, `aot2/` is changed.

| Item | What it does |
|---|---|
| `aot_simd.rs`: `fast_add2`, `fast_sub2`, `fast_mul2` | `[Out; 2]` for `[a0, a1] op [b0, b1]`, equal to the scalar kernels lane for lane. `add2_with` / `mul2_with` take the fallback as a closure (used by the mutation tests) |
| `aot_simd_lanes.rs` | SSE2 lane code (`std::arch`, x86-64 baseline, no runtime detection): `addpd` for the double sum, `pmuludq` for the 32 x 32 significand product, 64-bit shifts, masks and adds for rounding, `movemask_pd` for the lane bits |
| Zero form | a lane with a zero or denormal operand is decided in the vector path too (pass-through for a sum, signed zero for a product). Rejected to the scalar kernel: RND32 sums with a zero operand, infinity, NaN, overflow, flush, and a double sum on an undecided rounding boundary |
| Per-lane fallback | a lane with its bit set is recomputed by `fast_add` / `fast_mul` on the original operands; the other lane keeps its vector result |
| Other targets | `bad = 3`: both lanes use the scalar kernel. **No NEON path**: it cannot be built or tested on this x86-64 host, and an untested kernel would carry no exactness claim (the brief made NEON optional) |
| `aot_simd_core.rs`: `Core::aot_pair_exec` | the SIMD-mode compute form for `Fn = Fx + Fy`, `Fx - Fy`, `Fx * Fy`: one paired kernel, then registers, flags, MN..MI, AF and STKY of each element written in place (PEx first). Not called by any lowering |
| `aot_simd_sweep.rs` | the sweeps and mutation checks below |
| `kernel-bench ... pair` | `Kernel::Pair` in `aot_fast_bench.rs`: two consecutive operand pairs per call |

### Verification (VERIFIED, `cargo test -p nord-sharc --lib aot_simd`)

- Sweeps compare every `Out` field of both lanes with `float::add` / `float::mul`, in all four TRUNC x RND32 modes,
  add and sub: 300 000 random pairs with independent lanes; every exponent gap 0..=70 x 10 boundary fractions x all
  sign pairs x both orders; gaps 19..=23 around the exact-double limit; single-bit significands at every tie position;
  exponent edges 0..3 and 252..255 x fractions x extension bytes (overflow, flush); cancellations to every leading
  bit; all lane combinations of normal, denormal, zero, infinity and NaN. Directed cases are paired with a different
  case in the other lane, so lane isolation is checked.
- `pair_exec_equals_the_interpreter_on_both_elements`: `aot_pair_exec` equals `exec_compute_op` run on PEx then PEy
  (registers, extensions, ALU and multiplier flags, ASTAT, STKY), 6000 random SIMD states x 5 ops x 4 MODE1 values.
- `the_sweep_rejects_wrong_lanes_and_wrong_fallbacks`: the checker run on deliberately broken pair kernels must fail
  (swapped lanes, no fallback, a fallback that ignores the rounding mode, a fallback with the wrong subtraction).
- Mutation of the vector code (a `sed` edit of `aot_simd_lanes.rs`, `cargo test -p nord-sharc --lib aot_simd`, file
  restored after each): **12 of 12 caught**: zero pass-through takes the wrong operand; two-zero flag only for `a`;
  two-zero sign from `or`; RND32 zero sums not rejected; product zero sign dropped; product zero not flagged;
  movemask keeps lane 0 only; boundary check dropped; tie rounds up (`half - 1` to `half`); product carry dropped;
  product overflow bound 254; infinity not rejected. An earlier set on the code before the zero form was also caught
  (undecided never, exponent bound, odd bit ignored, product normalize shift, RND32 mask, lane swap on store).
- `ordinary_operands_rarely_reach_the_fallback`: the fallback runs for fewer than a quarter of the lanes on ordinary
  add operands and never on mul, so the vector path is a path and not a wrapper.

### Per-element micro-benchmark (EMPIRICAL)

`kernel-bench 9 pair`: 9 fresh processes, net ns per element, median of the per-process medians [min of mins - max of
maxes]. Host load 66 to 85% from other sessions, 0 compiler processes at each round. Gain called real only outside the
larger min-max band.

| row | scalar `fast` | `pair` | scalar / pair | verdict |
|---|---|---|---|---|
| add/normal/rne | 3.49 [3.33-4.25] | 4.10 [3.89-4.76] | 0.85 | within noise by the band rule (bands overlap 3.89-4.25); pair slower in 9 of 9 rounds |
| add/normal/trunc | 3.49 [3.34-3.98] | 4.08 [3.92-5.77] | 0.86 | within noise by the band rule; pair slower in 9 of 9 rounds |
| add/normal/r32 | 3.51 [3.34-3.78] | 4.08 [3.92-4.62] | 0.86 | **pair slower, bands disjoint** |
| mul/normal/rne | 2.83 [2.63-3.39] | 2.80 [2.62-4.18] | 1.01 | within noise |
| add/zero_operand/rne | 3.88 [3.27-4.65] | 4.10 [3.44-5.21] | 0.95 | within noise |
| mul/zero_operand/rne | 3.52 [3.32-4.83] | 2.77 [2.60-3.41] | 1.27 | within noise by the band rule (min-of-N 3.32 vs 2.60 ns); pair faster in 9 of 9 rounds |
| add/mixed/rne | 3.86 [3.66-5.15] | 5.06 [4.89-6.66] | 0.76 | within noise by the band rule (overlap 4.89-5.15); pair slower in 9 of 9 rounds |
| mul/mixed/rne | 3.92 [3.74-4.86] | 4.17 [3.93-6.54] | 0.94 | within noise |

The pair add is about 55 vector instructions for two lanes against about 40 integer instructions per lane in the
scalar add, and the out-of-order core already overlaps independent scalar elements, so two lanes in one register buy
nothing. Only the product with a zero operand is faster (the scalar kernel takes its cold out-of-line path). A first
version without the vector zero form sent every zero lane to the scalar fallback and was slower still.

### Lane census of real passes

Throw-away relaxed-atomic counters in `add2_with` / `mul2_with` (not committed; `aot_pair_exec` wired temporarily as
below), `lle_sharc_bench --engine aot`, 316 passes. Lanes by operand class (exponent field 0 = zero or denormal):

| Core | op | lanes | both normal | one zero | both zero | special |
|---|---|---|---|---|---|---|
| ORG Core2 | add/sub | 520 992 | 11.4% | 18.8% | 69.8% | 0 |
| ORG Core2 | mul | 391 200 | 16.3% | 68.1% | 15.6% | 0 |
| PNO Core2 | add/sub | 267 968 | 12.2% | 33.5% | 54.2% | 0 |
| PNO Core2 | mul | 247 112 | 15.3% | 49.9% | 34.8% | 0 |

ORG Core2 runs about 824 add and 619 mul pair calls per pass (1 445 both-element ops). **Only 11 to 16% of the lanes
have two normal operands.** The speed-ups of the scalar kernels in "Results" (2.0x to 2.5x) apply to that share of the
compute ops of these silent passes; a loaded audio pass has another mix. The 59 to 66 cycles per both-element op in
the profile are therefore mostly not arithmetic.

### Whole-pass A/B (EMPIRICAL; the wiring is not committed)

`aot_pe_both` in `aot_mem.rs` (the lowering session's file) was patched locally, for this measurement only, then
restored with `git checkout`:

```rust
if simd && kern == Kern::Exact && self.aot_pair_exec(cop) {
    return;
}
```

Two `lle_sharc_bench` binaries (without and with the patch, same generated modules `ORG2`, `PNO2`), one core, 300
passes + 16 warm-up, 2 warm-up runs, then 9 rounds alternating which binary runs first. Mean us per pass, median
[min-max]. Three series, the first two on the code before the vector zero form was added, the third on the final code.

| Core | series | before | after | host load, compilers | verdict |
|---|---|---|---|---|---|
| ORG Core2 | 1 | 177.5 [168.4-384.0] | 201.2 [190.6-388.9] | 61 to 87%, 0 to 3 | bands overlap (one 384 us outlier round); after slower in 9 of 9 rounds |
| ORG Core2 | 2 | 174.2 [162.5-182.6] | 195.3 [186.9-223.9] | 59 to 84%, 1 to 3 | **after slower, +12%, bands disjoint**; 9 of 9 rounds |
| ORG Core2 | 3 (final) | 199.4 [178.8-244.0] | 218.1 [196.5-279.0] | 63 to 94%, 1 to 4 | within noise, after not better (slower in 5 of 9 rounds for ORG, 8 of 9 for PNO) |
| PNO Core2 | 1 | 133.2 [126.7-286.9] | 142.7 [137.0-288.1] | 61 to 87%, 0 to 3 | within noise; after slower in 9 of 9 rounds |
| PNO Core2 | 2 | 128.9 [124.6-138.1] | 145.2 [137.3-155.5] | 59 to 84%, 1 to 3 | **after slower, +13%, bands disjoint**; 9 of 9 rounds |
| PNO Core2 | 3 (final) | 157.8 [136.3-167.4] | 166.5 [151.1-198.4] | 63 to 94%, 1 to 4 | within noise, after not better |

**The paired call site removes none of the compute share: it adds time or is within noise.** Do not hand the call site
to the lowering session as an improvement. If its owner wants to try it anyway, the change is the three lines above at
the top of `Core::aot_pe_both`, followed by the 1:1 diff against the interpreter (not run on the patched build:
`aot_pair_exec` is verified by the unit test only).

The API is only useful in a lowering that runs both elements of one op together over a block (today `b{bi}_{ii}` runs
the whole block for PEx and then for PEy, `aot_emit.rs`). Even then 54 to 70% of the add lanes are two zeros, which the
scalar kernel already handles in about 4 ns, so the upper bound of the gain is small.

## Fast-vs-Exact error on real passes

**Checked:** 2026-11-27. **Tool:** `fast_vs_exact` (`crates/nord-tools/src/bin/fast_vs_exact.rs`, 4 unit tests):

```
fast_vs_exact --image IMG --preset org|pno [--msgs FILE] --exact TAG --fast TAG
              [--passes 300] [--warmup 16] [--csv FILE] [--scan LO:HI]
```

Two cores boot from the same image and preset, receive the same ARM messages before the same passes, and run an exact
whole-pass module and the module of the fast tier (host `f32`, not bit-exact: `ORG2` / `ORG2F`, `PNO2` / `PNO2F`).
After every pass it compares, as `f32` words (non-finite as 0): the output lanes (`DM(0xB0159 + set)`, 8 sets x 12
words), the pass buffers `DM(0xB0000..0xB2FFF)`, and the module state (`DM(0x80000..0xFFFFF)` outside the buffers, 512
000 words). Only words that read as ordinary floats (zero or 1e-12..1e9) enter the error energies; other differing
words (integers, pointers) are counted separately. It reports per pass: max abs error, max relative error, error
energy, SNR relative to the exact output; the first diverging pass; control-flow (step count) mismatches; mean error
energy per quartile of passes and a drift verdict (GROWING when quartile 4 exceeds 2x quartile 3, else bounded).

Runs (one run each; deterministic, the result does not depend on timing):

- ORG Core2: `target/sram/ORG/core2_sram.bin`, preset `org`, the 912 flattened load messages
  (`org2.msgtokens`), `ORG2` vs `ORG2F`, 316 passes (16 warm-up + 300).
- PNO Core2: `target/sram/PNO/core2_sram.bin`, preset `org`, no messages (no PNO Core2 message script exists),
  `PNO2` vs `PNO2F`, 316 passes.

| | ORG Core2 | PNO Core2 |
|---|---|---|
| passes with a different step count (control flow) | 0 of 316 | 0 of 316 |
| output lanes: words differing | 0 of 30 336 (exact lanes are all 0, SNR undefined) | 0 of 30 336 (all 0) |
| pass buffers: words differing | 0 of 3 883 008 (exact energy 8.7e3 per pass, identical) | 0 of 3 883 008 (same) |
| failed data-memory reads, non-finite words in the fast core (added after review, re-run) | 0 / 0 | 0 / 0 |
| state: first diverging pass | 1 | 0 |
| state: words differing per pass, min / median / max | 58 / 190 / 222 of 512 000 | 12 / 96 / 128 of 512 000 |
| state: max abs error, any pass | 1.40e-5 | 1.28e-5 |
| state: max relative error, words 1e-12 or larger | 1.97e-5 (pass 254) | 1.93e-5 (pass 216) |
| state: largest relative error of any word (tiny values) | 1.22e-4 (about 1e-20) | 1.27e-5 (0.205) |
| mean error energy, quartiles 1 / 2 / 3 / 4 | 2.0e-11 / 2.1e-10 / 1.1e-9 / 1.1e-9 | 2.5e-11 / 3.0e-10 / 1.3e-9 / 4.4e-10 |
| drift | bounded: quartile 4 is 1.0x quartile 3 (57x quartile 1: a start-up rise, then flat) | bounded: quartile 4 is 0.34x quartile 3 (17.7x quartile 1) |

Samples, state words differing and max abs error. ORG Core2: pass 0 identical; pass 1: 64 words, 1.3e-8; pass 5: 100
words, 6.0e-8; pass 100: 190 words, 2.0e-6; pass 200: 222 words, 5.5e-6; last pass: 178 words, 3.4e-6. PNO Core2:
pass 16: 128 words, 2.4e-7; last pass: 84 words, 3.5e-6. The differing words are repeated copies of a few values
(for example `0xc1e36`, `0xc1e37`, `0xc1e40`, ... hold the same pair in ORG; `0x3e5214f8` vs `0x3e5215a7`, 0.205158 vs
0.205161, in PNO), so the count is a handful of distinct state variables.

**Silent and loaded pass.** Both streams are silent: the exact output lanes are exactly 0 in every pass (the pass
buffers hold nonzero data, identical on both cores; known for ORG Core2: `org-system-run.md`, "ORG Core2 produces no audio in any state tried"). **The SNR relative to
the exact output is therefore not defined** (signal energy 0, error 0): the audio-side comparison is trivially
identical and says nothing about audible error. The measurable difference is the module state. The idle pass (PNO
Core2, pass 16) and the loaded pass (ORG Core2 after the 912 messages, pass 16 and later) look alike: tens to a few
hundred state words differ by 1e-8 to 1e-5 absolute and 2e-5 relative, rising over the first 100 passes and then
flat. The state-energy SNR (about 280 to 300 dB) is dominated by large-magnitude words and is not a useful figure.

**Against the proposed tolerance (D256, proposed):**

| # | Criterion | ORG Core2 | PNO Core2 |
|---|---|---|---|
| 1 | step count equal to the exact module on every pass | met (0 of 316) | met (0 of 316) |
| 2 | output lanes: SNR at least 100 dB whole run, 80 dB worst pass, spectrum within 0.1 dB (`aot_audio::TOLERANCE`) | not testable: exact output is 0 | not testable |
| 3 | module float state: relative error at most 1e-3 on words of 1e-12 or larger | met, 2.0e-5 | met, 1.9e-5 |
| 4 | bounded drift: last-quartile mean error energy at most 2x the third | met, 1.0x | met, 0.34x |

**Not shown, so not claimed:** that the fast tier is inaudible. These streams never reach the output lanes; a state
error of 2e-5 relative in a filter or oscillator state could be audible once the voice sounds, and 316 passes (14.6 ms
of audio) cannot show a drift that needs seconds (a pole near the unit circle, a slow phase). Still needed:

1. An audio-producing stream (a PNO key stream with the model install, or an ORG voice that reaches the lanes; neither
   exists yet, `org-system-run.md` follow-up 2), at least 2 s of audio (about 43 400 passes).
2. A null test of the fast against the exact output of that stream: SNR over the lanes per 4 096-sample window against
   the D256 limits.
3. A blind A/B listening test of the same stream on both modules.
4. This table repeated for every module build, since the generated modules change with the lowering.

`lle_sharc_bench` shows the fast tier saves 12 to 17% of a pass on these streams (`sharc-lle-realtime.md`, "Compute
share"), so the listening cost is only worth paying if compute later dominates a pass.

The `max rel` column of the tool's pass table is the value of that pass only; the whole-run figures above are the maxima
over all passes (from the CSV). After the first review the tool also counts failed `dm_read`s and non-finite fast words
(both 0 in the two runs above, re-run 2026-11-28 with identical numbers); a silent exact output prints its SNR as
"undefined".

Reproduce (Git Bash; modules from `sharc-lle-realtime.md`, "Reproduce"):

```bash
W=$(cygpath -m $PWD)
export NORD_SHARC_AOT_HOT="ORG2=$W/target/lle/org2_whole.rs;ORG2F=$W/target/lle/org2_whole_approx.rs;PNO2=$W/target/lle/pno2_whole.rs;PNO2F=$W/target/lle/pno2_whole_approx.rs"
CARGO_TARGET_DIR=target/aot cargo build --release -p nord-tools --bin fast_vs_exact
target/aot/release/fast_vs_exact --image target/sram/ORG/core2_sram.bin --preset org --msgs target/lle/org2.msgtokens --exact ORG2 --fast ORG2F --csv target/lle/fve_org.csv
target/aot/release/fast_vs_exact --image target/sram/PNO/core2_sram.bin --preset org --exact PNO2 --fast PNO2F --csv target/lle/fve_pno.csv
```
