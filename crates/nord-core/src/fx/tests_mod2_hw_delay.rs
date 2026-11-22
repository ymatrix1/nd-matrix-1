//! Delay of the Mod 2 CHOR type 4 copy against the hardware clip (D173,
//! `docs/research/fx-chorus-hw-clip.md` s8).
//!
//! The firmware stage is driven with an impulse train and the offset of the delayed copy is read
//! per impulse, exactly as `tools/fx_chorus_oracle_delay.py` reads it for the oracle chain (the SHARC
//! oracle and the port give the same delays: min 0.737, max 3.433, mean 2.069 ms at rate word 30
//! and amount 64). The hardware reference is the comb filter fit of the audio of the clip
//! (`L.wav`, 22.5 to 37.5 s): range 0.9 to 1.3 ms at the minimum and 3.2 to 3.3 ms at the maximum, centre 2.1 to 2.3 ms, rate 0.36 Hz
//! (`tools/fx_chorus_comb.py`, seven runs over gain 0.6 to 1.0 and both channels). The
//! scope reading of the same clip (0.62 to 1.78 ms) rests on an ASSUMED scope tone of 87.31 Hz and
//! is not used here.

use super::fw_mod2::FwMod2Stage;

const FS: f32 = 48_000.0;
/// Spacing of the impulses, in frames: longer than the 256 frame delay ring.
const SPACING: usize = 600;

/// Delay (ms) of the copy for every impulse of a train, read from channel 0 of the stage.
fn copy_delays_ms(rate: u32, amount: u32, seconds: f32) -> Vec<f32> {
    let mut st = FwMod2Stage::new(false, rate, amount, FS);
    let n = (seconds * FS) as usize / SPACING * SPACING;
    let out: Vec<f32> = (0..n)
        .map(|i| {
            st.process_frame(if i % SPACING == 0 {
                [0.5, 0.5]
            } else {
                [0.0, 0.0]
            })[0]
        })
        .collect();
    // Skip the first period so the mix ramps and the delay lines have settled.
    out.chunks(SPACING)
        .skip(20)
        .map(|seg| {
            let direct = (0..8)
                .max_by(|&a, &b| seg[a].abs().total_cmp(&seg[b].abs()))
                .unwrap();
            let copy = (8..400)
                .max_by(|&a, &b| seg[a].abs().total_cmp(&seg[b].abs()))
                .unwrap();
            (copy - direct) as f32 / FS * 1000.0
        })
        .collect()
}

#[test]
fn chor4_delay_range_matches_the_hardware_comb_fit() {
    // EMPIRICAL: rate word 30 (0.343 Hz, the clip's 0.336 to 0.36 Hz), amount 64 (wet 0.82 of dry
    // against the clip's 0.86). Run over 3.2 s: one full LFO period is 2.9 s.
    let ms = copy_delays_ms(30, 64, 3.2);
    let (lo, hi) = ms
        .iter()
        .fold((f32::MAX, 0.0f32), |(l, h), &d| (l.min(d), h.max(d)));
    let mean = ms.iter().sum::<f32>() / ms.len() as f32;
    // Hardware comb fit 0.9..1.3 to 3.2..3.3 ms, centre 2.1..2.3; the oracle gives 0.74 .. 3.43, mean 2.07.
    assert!((0.6..=1.4).contains(&lo), "min {lo}");
    assert!((3.1..=3.7).contains(&hi), "max {hi}");
    assert!((1.9..=2.3).contains(&mean), "mean {mean}");
    // The scope reading (mean 1.17 ms) is 1.8 times shorter than every audio and oracle figure.
    assert!(mean / 1.17 > 1.5);
}

#[test]
fn chor4_delay_is_the_same_in_seconds_at_44100_hz() {
    // The delay words are DSP frames at 43,402.34 Hz; the stage rescales them (fw_mod2.rs).
    let at = |fs: f32| {
        let mut st = FwMod2Stage::new(false, 30, 64, fs);
        let n = (2.0 * fs) as usize / 600 * 600;
        let out: Vec<f32> = (0..n)
            .map(|i| st.process_frame(if i % 600 == 0 { [0.5, 0.5] } else { [0.0, 0.0] })[0])
            .collect();
        let m: Vec<f32> = out
            .chunks(600)
            .skip(20)
            .map(|s| {
                (8..400)
                    .max_by(|&a, &b| s[a].abs().total_cmp(&s[b].abs()))
                    .unwrap() as f32
                    / fs
            })
            .collect();
        m.iter().sum::<f32>() / m.len() as f32 * 1000.0
    };
    assert!((at(48_000.0) - at(44_100.0)).abs() < 0.15);
}
