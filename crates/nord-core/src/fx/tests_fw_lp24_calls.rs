//! Multi-call golden of the firmware Amp LP24 / HP24 stage: 6 scenarios of 24 calls (4 stereo
//! frames each) from the **template state** of a booted unit (cutoff and resonance smoothers 0,
//! pre-gain smoother 1.0, filter states 0) with the real `kAmpMidFreq` / `kAmpMid` words, state
//! carried across calls, against the `sharc-fx` oracle (`tools/fx_lp24_capture.py golden`). The
//! single-call cases of `tests_fw_lp24_golden.rs` start from random states; this one checks the
//! smoothing and the state write-back over time. Input: seeded LCG, amplitude 0.4.

use super::fw_lp24::{FwLp24, Lp24Mode};
use super::fw_lp24_tables as tab;
use super::tests_fw_lp24_calls_data::{OUT, SCENARIOS, SEEDS};

fn lcg_frames(seed: u32, n: usize) -> Vec<[f32; 2]> {
    let mut state = seed;
    let mut next = || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (((f64::from(state >> 8) / f64::from(1u32 << 24)) - 0.5) * 2.0 * 0.4) as f32
    };
    (0..n).map(|_| [next(), next()]).collect()
}

/// Stage in the state of a booted unit that has just received the words of `(freq, mid)`.
fn booted(mode: Lp24Mode, freq: f32, mid: f32) -> FwLp24 {
    let mut st = FwLp24::new(mode, freq, mid);
    st.load(
        [
            [tab::cutoff_word(freq), 0.0],
            [tab::res_word(mid), 0.0],
            [tab::gain_word(mid), 1.0],
        ],
        [[0.0; 6]; 2],
    );
    st
}

fn run(mode: Lp24Mode, freq: f32, mid: f32, seed: u32) -> Vec<f32> {
    let mut st = booted(mode, freq, mid);
    lcg_frames(seed, 96)
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|b| st.block(*b).into_iter().flatten())
        .collect()
}

fn mode_of(m: u8) -> Lp24Mode {
    if m == 0 {
        Lp24Mode::Low
    } else {
        Lp24Mode::High
    }
}

#[test]
fn every_scenario_matches_the_oracle_over_24_calls() {
    let mut worst = 0.0f32;
    for (n, &(m, freq, mid)) in SCENARIOS.iter().enumerate() {
        let got = run(mode_of(m), f32::from(freq), f32::from(mid), SEEDS[n]);
        let peak = OUT[n]
            .iter()
            .map(|w| f32::from_bits(*w).abs())
            .fold(0.0, f32::max);
        assert!(peak > 0.2, "scenario {n} is not quiet: {peak}");
        for (g, w) in got.iter().zip(&OUT[n]) {
            worst = worst.max((g - f32::from_bits(*w)).abs());
        }
    }
    assert!(worst < 1.0e-5, "worst difference {worst:e}");
}

#[test]
fn the_glide_from_the_template_matters() {
    // The same scenario started settled differs in the first calls: the test fails for a port
    // that ignores the smoothers' template start.
    let (m, freq, mid) = SCENARIOS[1];
    let (mode, f, md) = (mode_of(m), f32::from(freq), f32::from(mid));
    let template = run(mode, f, md, SEEDS[1]);
    let mut settled = FwLp24::new(mode, f, md);
    let frames = lcg_frames(SEEDS[1], 96);
    let settled: Vec<f32> = frames
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|b| {
            settled
                .block([b[0], b[1], b[2], b[3]])
                .into_iter()
                .flatten()
        })
        .collect();
    let diff = template
        .iter()
        .zip(&settled)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(diff > 1.0e-3, "start state is visible: {diff}");
}
