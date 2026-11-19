//! `cargo run --release -p nord-tools --bin fx_reverb_speed [-- --blocks 400 --repeat 3]`
//!
//! Times the bit-exact (`E40`) firmware reverb ports per `kRvbType` (0, 1 Spring; 2..11 multi-tap,
//! 6 has no captured block) on a deterministic stimulus (an xorshift noise burst train), as ms per
//! 480 frame block (the GUI block), min over `--repeat` runs, and prints an FNV-1a hash of every
//! output bit so a change that alters the output is visible. Pinned in
//! `docs/research/fw-reverb-speed.md`. Nothing here reads firmware or audio files.
use nord_core::fx::{FwReverb, FwReverbMulti, FwReverbMultiF32};
use std::time::Instant;

const BLOCK: usize = 480;

fn arg(name: &str, default: usize) -> usize {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|k| a.get(k + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn stimulus(n: usize) -> Vec<[f32; 2]> {
    let mut s = 0x2545_f491_4f6c_dd1d_u64;
    (0..n)
        .map(|k| {
            let mut next = || {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                ((s >> 40) as f32 / 8_388_608.0 - 1.0) * 0.25
            };
            // 1 s of noise, then 1 s of silence, repeating: exercises the tail and the idle path.
            if (k / 48_000) % 2 == 0 {
                [next(), next()]
            } else {
                [0.0, 0.0]
            }
        })
        .collect()
}

fn run(rvb_type: usize, input: &[[f32; 2]]) -> (f64, u64) {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut out = |y: [f32; 2]| {
        for v in y {
            hash = (hash ^ u64::from(v.to_bits())).wrapping_mul(0x100_0000_01b3);
        }
    };
    let t = Instant::now();
    if rvb_type < 2 {
        let mut r = FwReverb::new(rvb_type);
        r.set_amount(40.0, true);
        for &x in input {
            out(r.process_frame(x));
        }
    } else if std::env::var("F32").is_ok() {
        let mut r = FwReverbMultiF32::new(rvb_type).expect("covered type");
        r.set_amount(40.0, true);
        for &x in input {
            out(r.process_frame(x));
        }
    } else {
        let mut r = FwReverbMulti::new(rvb_type).expect("covered type");
        r.set_amount(40.0, true);
        for &x in input {
            out(r.process_frame(x));
        }
    }
    let ms = t.elapsed().as_secs_f64() * 1e3 / (input.len() / BLOCK) as f64;
    (ms, hash)
}

fn main() {
    let blocks = arg("--blocks", 400);
    let repeat = arg("--repeat", 3);
    let input = stimulus(blocks * BLOCK);
    println!("type  ms/480 (min of {repeat})  hash");
    let mut total = 0.0;
    for t in [0, 1, 2, 3, 4, 5, 7, 8, 9, 10, 11] {
        let (mut best, mut h) = (f64::MAX, 0);
        for _ in 0..repeat {
            let (ms, hh) = run(t, &input);
            best = best.min(ms);
            h = hh;
        }
        total += best;
        println!("{t:>4}  {best:>8.4}  {h:#018x}");
    }
    println!("sum   {total:>8.4}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stimulus_is_deterministic_and_the_hash_is_stable_for_a_short_run() {
        let a = stimulus(2 * BLOCK);
        assert_eq!(a, stimulus(2 * BLOCK));
        assert_eq!(run(10, &a).1, run(10, &a).1);
        assert_ne!(run(10, &a).1, run(10, &a[..BLOCK]).1);
    }
}
