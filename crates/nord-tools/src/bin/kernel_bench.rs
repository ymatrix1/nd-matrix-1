//! Micro-benchmark of the native SHARC float kernels (`ns4-lle3`): ns per `Fx + Fy`, `Fx - Fy` and
//! `Fx * Fy` over operand classes, for the optimized kernels, the `u128` reference kernels, and an
//! identity fold that measures loop and checksum overhead (subtracted from the net column).
//!
//!     cargo run --release -p nord-tools --bin kernel-bench -- [RUNS] [reference] [FILTER]
//!
//! Prints one TSV row per (operation, class, mode): the net median, min and max ns/op over RUNS
//! timed runs (default 7) after a warm-up run. `reference` adds the `u128` reference rows; any
//! other word is a substring filter on the row name (short runs of one class in a fresh process). To
//! compare two builds, run both executables alternately (see `docs/research/sharc-kernel-speed.md`).
//! Operand pairs are generated from a fixed seed, so every build times identical data.

use nord_sharc::core::aot_fast_bench::{fold_add, fold_mul, Kernel};
use nord_sharc::cpu_clock::{ns_per_tick, ticks};
use std::hint::black_box;
use std::time::Instant;

const PAIRS: usize = 4096;
/// Target time of one timed run (ns); repetitions are scaled to reach it.
const RUN_NS: f64 = 15e6;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// A float with the given biased exponent and a random mantissa and sign.
fn float(r: &mut Rng, exp: u64, signed: bool) -> u64 {
    let v = r.next();
    (u64::from(signed && v >> 63 == 1) << 39) | (exp << 31) | ((v >> 8) & 0x7FFF_FFFF)
}

type Class = (&'static str, fn(&mut Rng) -> (u64, u64));
type Pairs = [(u64, u64)];
type Batch = Box<dyn Fn(&Pairs, Kernel) -> u64>;

fn rand_exp(r: &mut Rng, lo: u64, hi: u64) -> u64 {
    lo + r.next() % (hi - lo + 1)
}

/// A float with a biased exponent drawn from `lo..=hi`.
fn float_in(r: &mut Rng, lo: u64, hi: u64) -> u64 {
    let e = rand_exp(r, lo, hi);
    float(r, e, true)
}

fn classes() -> Vec<Class> {
    vec![
        ("same_exp", |r| {
            let e = rand_exp(r, 100, 154);
            (float(r, e, true), float(r, e, true))
        }),
        ("gap_1_8", |r| {
            let e = rand_exp(r, 100, 154);
            let g = rand_exp(r, 1, 8);
            (float(r, e, true), float(r, e - g, true))
        }),
        ("gap_24_60", |r| {
            let e = rand_exp(r, 100, 154);
            let g = rand_exp(r, 24, 60);
            (float(r, e, true), float(r, e - g, true))
        }),
        ("cancel", |r| {
            let a = float_in(r, 100, 154);
            (a, a ^ (r.next() & 0xFFF)) // near-equal; use with sub
        }),
        ("normal", |r| (float_in(r, 100, 154), float_in(r, 100, 154))),
        ("zero_operand", |r| {
            (float(r, 0, true), float_in(r, 100, 154))
        }),
        ("near_overflow", |r| {
            (float_in(r, 248, 254), float_in(r, 120, 254))
        }),
        ("near_underflow", |r| {
            (float_in(r, 1, 6), float_in(r, 1, 134))
        }),
        ("mixed", |r| {
            let one = |r: &mut Rng| match r.next() % 16 {
                0 => float(r, 0, true),
                1 => float(r, 255, true),
                2 => float_in(r, 1, 3),
                3 => float_in(r, 252, 254),
                4..=7 => float_in(r, 120, 134),
                _ => float_in(r, 1, 254),
            };
            (one(r), one(r))
        }),
    ]
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

/// CPU ns per pair of `f` over `pairs`, one value per run.
fn time(pairs: &[(u64, u64)], runs: usize, f: &dyn Fn(&Pairs) -> u64) -> Vec<f64> {
    let npt = ns_per_tick();
    let t0 = Instant::now();
    let mut sink = black_box(f(black_box(pairs)));
    let one = t0.elapsed().as_nanos().max(1) as f64;
    let reps = ((RUN_NS / one) as usize).clamp(1, 20_000);
    let mut out = Vec::new();
    for run in 0..=runs {
        let t = ticks();
        for _ in 0..reps {
            sink ^= black_box(f(black_box(pairs)));
        }
        let ns = (ticks() - t) as f64 * npt / (reps * pairs.len()) as f64;
        if run > 0 {
            out.push(ns); // run 0 is the warm-up
        }
    }
    black_box(sink);
    out
}

struct Row<'a> {
    name: String,
    pairs: &'a [(u64, u64)],
}

fn report(name: &str, kernel: &str, net: &mut [f64]) {
    let (min, max) = (
        net.iter().cloned().fold(f64::MAX, f64::min),
        net.iter().cloned().fold(f64::MIN, f64::max),
    );
    println!("{name}\t{kernel}\t{:.2}\t{min:.2}\t{max:.2}", median(net));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let runs: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(7);
    let with_ref = args.iter().any(|a| a == "reference");
    let filter = args
        .iter()
        .skip(1)
        .find(|a| *a != "reference")
        .cloned()
        .unwrap_or_default();
    println!("op/class/mode\tkernel\tmedian_ns\tmin_ns\tmax_ns");
    let mut rng = Rng(0x1357_9BDF_2468_ACE1);
    let mut data = Vec::new();
    for (name, gen) in classes() {
        let pairs: Vec<(u64, u64)> = (0..PAIRS).map(|_| gen(&mut rng)).collect();
        data.push((name, pairs));
    }
    let mut rows: Vec<(Row, Batch)> = Vec::new();
    for (name, pairs) in &data {
        let add_like = !matches!(*name, "cancel");
        if add_like {
            rows.push((
                Row {
                    name: format!("add/{name}/rne"),
                    pairs,
                },
                Box::new(|p, k| fold_add(p, false, false, false, k)),
            ));
            rows.push((
                Row {
                    name: format!("sub/{name}/rne"),
                    pairs,
                },
                Box::new(|p, k| fold_add(p, true, false, false, k)),
            ));
        } else {
            rows.push((
                Row {
                    name: format!("sub/{name}/rne"),
                    pairs,
                },
                Box::new(|p, k| fold_add(p, true, false, false, k)),
            ));
        }
        if !matches!(*name, "cancel" | "same_exp" | "gap_1_8" | "gap_24_60") {
            rows.push((
                Row {
                    name: format!("mul/{name}/rne"),
                    pairs,
                },
                Box::new(|p, k| fold_mul(p, false, false, k)),
            ));
        }
    }
    // other rounding modes on the ordinary classes
    for (name, pairs) in data
        .iter()
        .filter(|(n, _)| matches!(*n, "normal" | "gap_1_8" | "gap_24_60"))
    {
        rows.push((
            Row {
                name: format!("add/{name}/trunc"),
                pairs,
            },
            Box::new(|p, k| fold_add(p, false, true, false, k)),
        ));
        rows.push((
            Row {
                name: format!("add/{name}/r32"),
                pairs,
            },
            Box::new(|p, k| fold_add(p, false, false, true, k)),
        ));
        if *name == "normal" {
            rows.push((
                Row {
                    name: format!("mul/{name}/trunc"),
                    pairs,
                },
                Box::new(|p, k| fold_mul(p, true, false, k)),
            ));
            rows.push((
                Row {
                    name: format!("mul/{name}/r32"),
                    pairs,
                },
                Box::new(|p, k| fold_mul(p, false, true, k)),
            ));
        }
    }
    for (row, f) in rows.iter().filter(|(r, _)| r.name.contains(&filter)) {
        let id = median(&mut time(row.pairs, runs, &|p| f(p, Kernel::Identity)));
        let mut fast: Vec<f64> = time(row.pairs, runs, &|p| f(p, Kernel::Fast))
            .iter()
            .map(|t| (t - id).max(0.0))
            .collect();
        report(&row.name, "fast", &mut fast);
        if with_ref {
            let mut r: Vec<f64> = time(row.pairs, runs, &|p| f(p, Kernel::Reference))
                .iter()
                .map(|t| (t - id).max(0.0))
                .collect();
            report(&row.name, "reference", &mut r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_generate_the_exponents_they_name() {
        let mut r = Rng(1);
        for (name, gen) in classes() {
            for _ in 0..200 {
                let (a, b) = gen(&mut r);
                let (ea, eb) = ((a >> 31) & 0xFF, (b >> 31) & 0xFF);
                match name {
                    "same_exp" => assert_eq!(ea, eb),
                    "gap_1_8" => assert!((1..=8).contains(&(ea - eb))),
                    "gap_24_60" => assert!((24..=60).contains(&(ea - eb))),
                    "zero_operand" => assert_eq!(ea, 0),
                    "near_overflow" => assert!(ea >= 248),
                    "near_underflow" => assert!(ea <= 6),
                    "cancel" => assert!((a ^ b) < 0x1000),
                    _ => {}
                }
            }
        }
    }
}
