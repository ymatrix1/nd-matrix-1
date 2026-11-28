//! Proof for the firmware EQ memo (`fw_eq.rs`, issue #691): a chain whose EQ stage memoizes its
//! target words and mid frequency index renders bit for bit what a chain that re-derives them on
//! every frame renders, across knob moves, a long silent stretch and a note-on after it.

use super::*;

const EQ_ONLY: FirmwareFx = FirmwareFx {
    eq: true,
    ..FirmwareFx::OFF
};

fn chain(memo: bool) -> LayerFxChain {
    let mut fx = LayerFxChain::new(48_000.0);
    fx.reverb_model = ReverbModel::Off;
    fx.amp_cabinet = AmpCabinet::Off;
    fx.firmware_fx = EQ_ONLY;
    fx.eq_bass_db = 3.0;
    fx.eq_mid_db = -5.0;
    fx.eq_treble_db = 6.0;
    fx.eq_mid_freq_hz = 1000.0;
    // the stage is built on the first frame; build it now so the switch is in place
    let _ = fx.process_frame([0.0, 0.0]);
    fx.fw_eq.as_mut().expect("eq stage").memo = memo;
    fx
}

/// Frames of a note that starts at `on` and keeps ringing, silence before it.
fn input(i: usize, on: usize) -> [f32; 2] {
    if i < on {
        return [0.0, 0.0];
    }
    let x = ((i - on) as f32 * 0.043).sin() * 0.3;
    [x, -0.8 * x]
}

#[test]
fn memo_matches_the_unmemoized_path_with_a_moving_knob_set() {
    let (mut a, mut b) = (chain(true), chain(false));
    for i in 0..30_000usize {
        if i % 1500 == 0 {
            // knob moves: every input of the memo changes at some point, also back to a repeat
            let k = (i / 1500) as f32;
            for fx in [&mut a, &mut b] {
                fx.eq_bass_db = (k * 1.5) % 9.0 - 4.0;
                fx.eq_mid_db = if i % 3000 == 0 { 0.0 } else { 4.0 };
                fx.eq_treble_db = 6.0 - k;
                fx.eq_mid_freq_hz = [250.0, 1000.0, 1000.0, 4000.0][(i / 1500) % 4];
                fx.amp_cabinet = if i == 21_000 {
                    AmpCabinet::LpFilter
                } else {
                    AmpCabinet::Off
                };
            }
        }
        let x = input(i, 0);
        let (ya, yb) = (a.process_frame(x), b.process_frame(x));
        assert_eq!(
            [ya[0].to_bits(), ya[1].to_bits()],
            [yb[0].to_bits(), yb[1].to_bits()],
            "frame {i}"
        );
    }
}

#[test]
fn memo_matches_after_a_long_idle_stretch_and_a_note_on() {
    let (mut a, mut b) = (chain(true), chain(false));
    // 4 s of exact silence, then a note
    let on = 4 * 48_000;
    for i in 0..on + 5_000 {
        let x = input(i, on);
        let (ya, yb) = (a.process_frame(x), b.process_frame(x));
        assert_eq!(
            [ya[0].to_bits(), ya[1].to_bits()],
            [yb[0].to_bits(), yb[1].to_bits()],
            "frame {i}"
        );
    }
}

#[test]
fn memo_reuses_words_while_the_inputs_repeat_and_refreshes_on_a_change() {
    let mut fx = chain(true);
    let st = fx.fw_eq.as_mut().expect("eq stage");
    let i1 = st.freq_index_of_hz(1000.0);
    assert_eq!(i1, super::fw_eq::freq_index_of_hz(1000.0));
    assert_eq!(st.freq_index_of_hz(1000.0), i1, "repeat hits the memo");
    let i2 = st.freq_index_of_hz(4000.0);
    assert_eq!(i2, super::fw_eq::freq_index_of_hz(4000.0));
    assert_ne!(i1, i2, "a new Hz refreshes the memo");
    // the memo of the words holds the last key
    let key = st.targets.expect("words memoized").0;
    st.set(1.0, 2.0, 3.0, 40.0, false);
    assert_ne!(st.targets.unwrap().0, key);
}
