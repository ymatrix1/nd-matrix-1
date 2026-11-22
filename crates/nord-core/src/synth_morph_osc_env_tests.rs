//! Osc Env Amount morph slot (D206): the firmware law against the captured ARM rows, the opt-in
//! gate, copy / paste and the engine path.

use crate::cbin::{MorphDeltas, SynthLayerDecoded};
use crate::morph::MorphControllers;
use crate::synth::SynthLayer;
use crate::synth_morph::{SynthMorphSet, OSC_ENV_SLOT};
use crate::synth_osc_env_word::osc_env_dsp_word;

/// ARM rows of `tools/morph_osc_env_report.py` over the 9 factory programs that use the target.
const ROWS: &str = include_str!("../../../docs/research/data/morph-fw-osc-env.csv");

/// Controller positions `v / 127` of a scenario text such as `wheel:127,at:64`.
fn controllers(scenario: &str) -> MorphControllers {
    let mut c = MorphControllers::default();
    for ev in scenario.split(',') {
        let (src, v) = ev.split_once(':').unwrap();
        let x = v.parse::<f32>().unwrap() / 127.0;
        match src {
            "wheel" => c.mod_wheel = x,
            "at" => c.aftertouch = x,
            _ => c.control_pedal = x,
        }
    }
    c
}

/// A decoded layer with the stored base (steps) and depths (steps) of one captured row.
fn layer(base: i32, depths: [i32; 3]) -> SynthLayerDecoded {
    let d = |s: i32| s as f32 / 120.0;
    SynthLayerDecoded {
        osc_env_amount: base as f32 / 60.0,
        osc_env_amount_morph: MorphDeltas {
            wheel: d(depths[0]),
            aftertouch: d(depths[1]),
            ctrl_pedal: d(depths[2]),
        },
        ..SynthLayerDecoded::default()
    }
}

/// Static ARM sweep (`tools/arm_osc_env_static.py`): program, Env To Pitch bit, amount, DSP steps.
const STATIC_ROWS: &str = include_str!("../../../docs/research/data/morph-fw-osc-env-dst.csv");

/// The three programs whose synth layer A has Env To Pitch on (`kOscEnvDst` 1, ARM cells).
const PITCH_PROGRAMS: [&str; 3] = ["Cassette Pad AT", "Favorite Video", "Time Warp Saw"];

fn parse(line: &str) -> (String, i32, [i32; 3], f32, Option<f32>) {
    // program,layer,scenario,base,wheel,at,pedal,law,dispatch,dsp_steps (scenario may be quoted)
    let line = line.replace("\"wheel:127,at:127,pedal:127\"", "all3");
    let f: Vec<&str> = line.split(',').collect();
    let scenario = if f[2] == "all3" {
        "wheel:127,at:127,pedal:127".to_string()
    } else {
        f[2].to_string()
    };
    let n = |i: usize| f[i].parse::<i32>().unwrap();
    (
        scenario,
        n(3),
        [n(4), n(5), n(6)],
        f[8].parse().unwrap(),
        f[9].parse().ok(),
    )
}

fn play(base: i32, depths: [i32; 3], scenario: &str) -> f32 {
    let mut set = SynthMorphSet::from_decoded(&layer(base, depths));
    let mut synth = SynthLayer::new(48_000.0);
    set.apply_with(&controllers(scenario).firmware_weighted(), &mut synth, true);
    synth.osc_env_amount * 60.0
}

#[test]
fn the_slot_reproduces_every_captured_dispatch_row() {
    let mut n = 0;
    for line in ROWS.lines().skip(1) {
        let (sc, base, d, dispatch, _) = parse(line);
        let got = play(base, d, &sc);
        assert!((got - dispatch).abs() < 1e-3, "{line}: {got} vs {dispatch}");
        n += 1;
    }
    assert_eq!(n, 27, "9 programs, 11 layer series");
}

#[test]
fn the_dsp_word_law_matches_all_27_rows_by_destination() {
    // 21 wheel rows are Osc Ctrl (`x / 60`), the 6 aftertouch rows are Env To Pitch programs (OQ43
    // answered, D213): the taper word equals the ARM value on every row.
    let (mut ctrl, mut pitch) = (0, 0);
    for line in ROWS.lines().skip(1) {
        let (sc, base, d, dispatch, dsp) = parse(line);
        let to_pitch = PITCH_PROGRAMS.contains(&line.split(',').next().unwrap());
        let Some(dsp) = dsp else { continue };
        let steps = play(base, d, &sc);
        assert!((steps - dispatch).abs() < 1e-3, "{line}");
        let got = osc_env_dsp_word(steps, to_pitch) * 60.0;
        assert!((got - dsp).abs() < 2e-3, "{line}: {got} vs {dsp}");
        if to_pitch {
            pitch += 1;
        } else {
            ctrl += 1;
        }
    }
    assert_eq!((ctrl, pitch), (21, 6));
}

#[test]
fn the_dsp_word_law_matches_every_static_row_and_the_dst_flip() {
    // Time Warp Saw and Sine-Saw Plk Whl, each with Env To Pitch as recalled and patched.
    let mut n = [0; 2];
    for line in STATIC_ROWS.lines().skip(1) {
        let f: Vec<&str> = line.split(',').collect();
        let to_pitch = f[1] == "1";
        let (x, dsp): (f32, f32) = (f[2].parse().unwrap(), f[4].parse().unwrap());
        let got = osc_env_dsp_word(x, to_pitch) * 60.0;
        assert!((got - dsp).abs() < 1e-3, "{line}: {got}");
        n[usize::from(to_pitch)] += 1;
    }
    assert_eq!(n, [242, 242]);
}

#[test]
fn rich_fm_pad_shows_a_fractional_step_and_the_clamp() {
    // Rich FM Pad Whl layer B: base 60, wheel -45: 37.5 at wheel 64, 15 at 127 (ARM run).
    assert!((play(60, [-45, 0, 0], "wheel:64") - 37.5).abs() < 1e-4);
    assert!((play(60, [-45, 0, 0], "wheel:127") - 15.0).abs() < 1e-4);
    assert!(
        (play(50, [40, 0, 0], "wheel:127") - 60.0).abs() < 1e-4,
        "clamp +60"
    );
    assert!(
        (play(-50, [-40, 0, 0], "wheel:127") + 60.0).abs() < 1e-4,
        "clamp -60"
    );
    // a negative base with no source stays the stored amount
    assert!((play(-21, [-15, 0, 0], "wheel:0") + 21.0).abs() < 1e-4);
}

#[test]
fn the_slot_is_off_unless_asked_and_apply_keeps_the_old_signature() {
    let mut set = SynthMorphSet::from_decoded(&layer(21, [2, 0, 0]));
    let mut synth = SynthLayer::new(48_000.0);
    let ctrl = controllers("wheel:127").firmware_weighted();
    set.apply(&ctrl, &mut synth);
    set.apply_with(&ctrl, &mut synth, false);
    assert_eq!(synth.osc_env_amount, 0.0, "default render: field untouched");
    set.apply_with(&ctrl, &mut synth, true);
    assert!((synth.osc_env_amount * 60.0 - 23.0).abs() < 1e-4);
    assert_eq!(OSC_ENV_SLOT, 5);
    assert!(set.slot(OSC_ENV_SLOT).delta_wheel > 0.0);
}

#[test]
fn a_live_edit_becomes_the_new_base_and_clearing_restores_it() {
    let mut set = SynthMorphSet::from_decoded(&layer(21, [2, 0, 0]));
    let mut synth = SynthLayer::new(48_000.0);
    let wheel = controllers("wheel:127").firmware_weighted();
    set.apply_with(&wheel, &mut synth, true);
    synth.osc_env_amount = 30.0 / 60.0; // GUI edit while the wheel is up
    set.apply_with(&wheel, &mut synth, true);
    assert!((synth.osc_env_amount * 60.0 - 32.0).abs() < 1e-3, "30 + 2");
    set.slot_mut(OSC_ENV_SLOT).delta_wheel = 0.0;
    set.apply_with(&wheel, &mut synth, true);
    assert!(
        (synth.osc_env_amount * 60.0 - 30.0).abs() < 1e-3,
        "base back"
    );
}
