//! Unit tests of [`FwMod1Rest`] (types, rate, frames per call).

use super::*;

fn run(ty: u8, rate: u32, amount: u32, fs: f32, n: usize) -> Vec<[f32; 2]> {
    let mut st = FwMod1Rest::new(ty, rate, amount, fs).unwrap();
    (0..n)
        .map(|i| st.process_frame([((i as f32) * 0.05).sin(), ((i as f32) * 0.031).cos()]))
        .collect()
}

#[test]
fn every_type_runs_finite_with_a_two_frame_latency() {
    for ty in 4..=11 {
        for (rate, amount) in [(0, 0), (64, 64), (127, 127), (30, 100)] {
            let out = run(ty, rate, amount, 48_000.0, 6000);
            assert!(
                out.iter()
                    .all(|f| f.iter().all(|x| x.is_finite() && x.abs() < 16.0)),
                "type {ty} rate {rate} amount {amount}"
            );
            assert!(out[..2].iter().all(|f| *f == [0.0, 0.0]), "latency");
            assert!(
                out.iter().skip(2).any(|f| f[0] != 0.0),
                "type {ty} rate {rate} amount {amount} is silent"
            );
        }
    }
    for ty in [0, 1, 2, 3, 12] {
        assert!(FwMod1Rest::new(ty, 0, 0, 48_000.0).is_none(), "type {ty}");
    }
}

#[test]
fn the_laws_hit_the_captured_samples() {
    for ty in 4..=11 {
        let sp = spec(ty).unwrap();
        for law in sp.amt.iter().chain(sp.rate) {
            for (k, &v) in SAMPLES.iter().enumerate() {
                assert_eq!(to_word(law, eval(law, v)), law.words[k], "type {ty} v {v}");
            }
        }
    }
}

#[test]
fn the_dense_rate_words_follow_the_arm_between_the_samples() {
    // ARM words at v = 120 (`fx-mod1-dense.csv`): RM phase increment 0xaa84a00, A-WAH 0x3c...
    // The 11-sample law was 6.4 % of the range off there.
    let rm = FwMod1Rest::new(4, 120, 64, crate::organ::FIRMWARE_FRAME_RATE_HZ as f32).unwrap();
    assert_eq!(rm.st.ctx[index(-32)], 0x0aa8_4a00);
    let sample_only = eval(&spec(4).unwrap().rate[0], 120);
    assert!((sample_only - 0x0aa8_4a00_u32 as f32).abs() > 1.0e6);
    for ty in [4, 5] {
        let at = |v| {
            FwMod1Rest::new(ty, v, 64, crate::organ::FIRMWARE_FRAME_RATE_HZ as f32)
                .unwrap()
                .st
                .ctx[index(-32)]
        };
        assert_eq!(at(127), 0x0fb6_3b00, "type {ty}");
        assert_eq!(at(2), 0x0003_ed00, "type {ty}");
    }
}

#[test]
fn the_boot_words_give_the_self_pointers() {
    // RM, WAH and PUMP read the phase through `c[-29]`; the boot word is the address of
    // `c[-31]` in the row, relocated to the context of the port.
    for ty in [4, 8, 10] {
        let st = FwMod1Rest::new(ty, 64, 64, 48_000.0).unwrap();
        assert_eq!(st.st.ctx[index(-29)], CTX_I1 - 31, "type {ty}");
    }
}

#[test]
fn phase_increments_scale_with_the_engine_rate() {
    for ty in [4, 8, 10] {
        let at = |fs: f32| FwMod1Rest::new(ty, 90, 64, fs).unwrap().st.ctx[index(-32)] as f64;
        let ratio = at(86_804.69) / at(43_402.34);
        assert!((ratio - 0.5).abs() < 1.0e-3, "type {ty}: {ratio}");
    }
}

#[test]
fn four_frames_per_call_gives_a_four_frame_latency_and_the_default_stays_two() {
    let first_live = |frames: usize| {
        let mut st = FwMod1Rest::new(4, 64, 64, 48_000.0).unwrap();
        st.set_frames_per_call(frames);
        (0..16)
            .map(|i| st.process_frame([0.1 + 0.01 * i as f32, 0.2]))
            .position(|f| f != [0.0, 0.0])
            .unwrap()
    };
    assert_eq!(first_live(2), 2);
    assert_eq!(first_live(4), 4);
    assert_eq!(first_live(3), 2, "anything but 4 is the default");
}

#[test]
fn rate_and_amount_change_the_output() {
    for ty in 4..=11 {
        assert_ne!(
            run(ty, 20, 90, 48_000.0, 3000),
            run(ty, 100, 90, 48_000.0, 3000),
            "rate {ty}"
        );
        assert_ne!(
            run(ty, 60, 10, 48_000.0, 3000),
            run(ty, 60, 120, 48_000.0, 3000),
            "amount {ty}"
        );
    }
}
