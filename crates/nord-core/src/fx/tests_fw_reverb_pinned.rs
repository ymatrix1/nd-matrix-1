//! Bit-identity pin of the firmware reverb ports (`FwReverb` types 0 and 1, `FwReverbMulti` types
//! 2..=5 and 7..=11, every colour): an FNV-1a hash of every output bit on a fixed synthetic
//! stimulus (noise burst, then silence so the tails and the delay lines are read back).
//!
//! The pins were taken from the implementation before the speed work of
//! `docs/research/fw-reverb-speed.md` (the `E40` fast paths, the paired memory access), which
//! changes the cost, not one output bit. They are not oracle data; the oracle checks are
//! `tests_fw_rvb_golden.rs` and `tests_fw_rvbm_golden.rs`. A change that moves a pin changes the
//! output of the reverb and needs the oracle null tests and a decision (`docs/DECISIONS.md`).

use super::{FwReverb, FwReverbMulti};

const FRAMES: usize = 30_000;
const NOISE: usize = 1_500;

/// Deterministic stimulus: xorshift noise at 0.25 peak for `NOISE` frames, then silence.
fn stimulus() -> Vec<[f32; 2]> {
    let mut s = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        ((s >> 40) as f32 / 8_388_608.0 - 1.0) * 0.25
    };
    (0..FRAMES)
        .map(|k| {
            if k < NOISE {
                [next(), next()]
            } else {
                [0.0; 2]
            }
        })
        .collect()
}

fn fnv(h: &mut u64, v: f32) {
    *h = (*h ^ u64::from(v.to_bits())).wrapping_mul(0x100_0000_01b3);
}

/// Hash of the output of reverb `rvb_type` with colour `color` (multi-tap types only).
fn run(rvb_type: usize, color: usize, input: &[[f32; 2]]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    if rvb_type < 2 {
        let mut r = FwReverb::new(rvb_type);
        r.set_amount(127.0, true);
        for &x in input {
            for v in r.process_frame(x) {
                fnv(&mut h, v);
            }
        }
    } else {
        let mut r = FwReverbMulti::new(rvb_type).expect("covered type");
        assert!(r.set_color(color));
        r.set_amount(127.0, true);
        for &x in input {
            for v in r.process_frame(x) {
                fnv(&mut h, v);
            }
        }
    }
    h
}

/// `(kRvbType, kRvbColor, hash)`.
const PINS: [(usize, usize, u64); 29] = [
    (0, 0, 0xa1243c2ff83e7f44),
    (1, 0, 0xa263f88a8868a583),
    (2, 0, 0x1319b36650754114),
    (2, 1, 0x873b225f3e851e11),
    (2, 2, 0xd68f4a6a540c7289),
    (3, 0, 0x2f5800834f1c5f78),
    (3, 1, 0x455e9a2e5697490d),
    (3, 2, 0x979c87cb06ff9eab),
    (4, 0, 0xbb908d8e6d3976a3),
    (4, 1, 0x32eec6210170ba15),
    (4, 2, 0x764a82efc7190d4a),
    (5, 0, 0x1307558a7360781c),
    (5, 1, 0x6ce955d825a61410),
    (5, 2, 0x7b5f271d51045042),
    (7, 0, 0x48abe0aab882fa3e),
    (7, 1, 0xa69982845730c2b8),
    (7, 2, 0x18d34a7cee0bf377),
    (8, 0, 0x7932c8a196b51ad2),
    (8, 1, 0x2688ec93bbd1ab0d),
    (8, 2, 0xb0ab6b3c52a69d85),
    (9, 0, 0x358454dd169f25f3),
    (9, 1, 0x0afedaa2efd310e1),
    (9, 2, 0xc2a018b6afe829ec),
    (10, 0, 0x2a2272c8a335b297),
    (10, 1, 0xdd3702167a3611a8),
    (10, 2, 0xcb042bafea44746b),
    (11, 0, 0xc8f71d4e0594b1d2),
    (11, 1, 0x9760f278a750eeb4),
    (11, 2, 0x0f8008dd25f367f2),
];

#[test]
fn every_reverb_type_and_colour_keeps_its_pinned_output() {
    let input = stimulus();
    let bad: Vec<String> = PINS
        .iter()
        .filter_map(|&(t, c, want)| {
            let got = run(t, c, &input);
            (got != want).then(|| format!("    ({t}, {c}, {got:#018x}),"))
        })
        .collect();
    assert!(bad.is_empty(), "output changed:\n{}", bad.join("\n"));
}

#[test]
fn the_stimulus_is_deterministic_and_the_tail_is_not_silent() {
    let input = stimulus();
    assert_eq!(input, stimulus());
    assert!(input[..NOISE].iter().all(|f| f[0].abs() <= 0.25));
    assert!(input[NOISE..].iter().all(|f| *f == [0.0; 2]));
    let mut r = FwReverbMulti::new(10).unwrap();
    r.set_amount(127.0, true);
    let tail_energy: f32 = input
        .iter()
        .enumerate()
        .map(|(k, &x)| {
            let y = r.process_frame(x);
            if k >= NOISE + 100 {
                y[0] * y[0]
            } else {
                0.0
            }
        })
        .sum();
    assert!(tail_energy > 0.0, "the pin covers the reverb tail");
}
