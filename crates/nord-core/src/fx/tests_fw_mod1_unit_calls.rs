//! `FirmwareFx::mod1_unit_calls` (D186): the RM / A-WAH / WAH / PUMP stage runs the frames per
//! call of the layer's unit (twice the ARM's block count: 2 for the organ and piano units, 4 for
//! the synth units, `docs/research/fx-mod1-call-count.md`) and `FirmwareFx::compressor_row` (D187):
//! the compressor carries the firmware's gain-law words.

use super::*;

fn chain(unit_calls: bool, unit_frames: usize, ty: u8) -> LayerFxChain {
    let mut fx = LayerFxChain::new(48_000.0);
    fx.firmware_fx = FirmwareFx {
        mod1: true,
        mod1_unit_calls: unit_calls,
        pump: true,
        ..FirmwareFx::OFF
    };
    fx.mod1_type = Mod1Type::Tremolo;
    fx.mod1_type_raw = Some(ty);
    fx.mod1_rate_raw = Some(100);
    fx.mod1_amount = 1.0;
    fx.mod1_unit_frames = unit_frames;
    fx
}

fn run(fx: &mut LayerFxChain, n: usize) -> Vec<[f32; 2]> {
    (0..n)
        .map(|k| fx.process_mod1(0.5 + 0.4 * (k as f32 * 0.011).sin(), 0.5, 1.0 / 48_000.0))
        .collect()
}

fn latency(y: &[[f32; 2]]) -> usize {
    y.iter().take_while(|f| **f == [0.0, 0.0]).count()
}

#[test]
fn the_switch_selects_the_unit_frames_and_off_keeps_the_two_frame_default() {
    for ty in [4u8, 6, 8] {
        let (mut synth, mut piano, mut off) =
            (chain(true, 4, ty), chain(true, 2, ty), chain(false, 4, ty));
        assert_eq!(latency(&run(&mut synth, 64)), 4, "type {ty}: synth unit");
        assert_eq!(latency(&run(&mut piano, 64)), 2, "type {ty}: piano unit");
        assert_eq!(latency(&run(&mut off, 64)), 2, "type {ty}: switch off");
    }
}

#[test]
fn a_synth_unit_call_of_four_frames_changes_the_envelope_and_smoother_rates() {
    // A-WAH follows the input envelope per call: 4 frames per call is not 2.
    let (mut a, mut b) = (chain(true, 2, 6), chain(true, 4, 6));
    let (ya, yb) = (run(&mut a, 8_000), run(&mut b, 8_000));
    let diff: f32 = ya.iter().zip(&yb).map(|(p, q)| (p[0] - q[0]).abs()).sum();
    assert!(
        diff > 1.0,
        "two- and four-frame calls give the same A-WAH: {diff}"
    );
}

#[test]
fn the_engine_gives_the_synth_chains_four_frames_and_the_others_two() {
    let e = crate::engine::Stage4Engine::new(48_000.0);
    let frames: Vec<usize> = e.fx_chains.iter().map(|c| c.mod1_unit_frames).collect();
    assert_eq!(
        frames,
        [2, 2, 2, 2, 4, 4, 4],
        "organ A B, piano A B, synth A B C"
    );
    assert!(
        e.fx_chains.iter().all(|c| !c.firmware_fx.mod1_unit_calls),
        "opt-in"
    );
}

#[test]
fn the_compressor_row_switch_changes_the_layer_compressor_and_is_off_by_default() {
    let (default, all) = (FirmwareFx::DEFAULT, FirmwareFx::ALL);
    assert!(!default.compressor_row && !all.compressor_row);
    let make = |row: bool| {
        let mut fx = LayerFxChain::new(48_000.0);
        fx.firmware_fx = FirmwareFx {
            compressor: true,
            compressor_row: row,
            ..FirmwareFx::OFF
        };
        fx.comp_enabled = true;
        fx.comp_amount = 1.0;
        fx
    };
    let peak = |row: bool| {
        let mut fx = make(row);
        let mut p = 0.0f32;
        for k in 0..24_000 {
            let x = 0.6 * (k as f32 * 0.07).sin();
            let [l, _] = fx.process_frame([x, x]);
            if k > 20_000 {
                p = p.max(l.abs());
            }
        }
        p
    };
    let (assumed, firmware) = (peak(false), peak(true));
    assert!(
        (assumed - firmware).abs() > 0.02 * assumed.max(firmware),
        "assumed {assumed}, firmware row {firmware}"
    );
}

#[test]
fn merge_on_carries_the_two_refinements() {
    let mut f = FirmwareFx::OFF;
    f.merge_on(&FirmwareFx {
        compressor_row: true,
        mod1_unit_calls: true,
        ..FirmwareFx::OFF
    });
    assert!(f.compressor_row && f.mod1_unit_calls);
    let mut g = FirmwareFx::OFF;
    g.merge_on(&FirmwareFx::ALL);
    assert!(
        !g.compressor_row && !g.mod1_unit_calls,
        "ALL leaves them off"
    );
}
