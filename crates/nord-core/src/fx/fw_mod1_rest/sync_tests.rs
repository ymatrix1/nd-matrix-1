//! Tests of the PUMP Master Clock drive of [`FwMod1Rest`] (`fw_pump_clock`, D149).

use super::*;

#[test]
fn a_counter_change_resets_the_free_running_pump_phase() {
    const N_FRAMES: usize = 7_000;
    let gain_after = |tick: bool| {
        let mut st = FwMod1Rest::new(10, 73, 127, 48_000.0).unwrap();
        let mut g = 0.0;
        for i in 0..N_FRAMES {
            if tick && i == N_FRAMES - 1_000 {
                st.st.ctx[index(-0x15)] = 1; // the next block sees a changed counter
            }
            g = st.process_frame([1.0, 1.0])[0];
        }
        g
    };
    // 7,000 frames into the 7,862 frame cycle the gain is 0.896 without a tick; a tick 1,000
    // frames earlier restarts the cycle (0.438, measured).
    let (free, ticked) = (gain_after(false), gain_after(true));
    assert!(free > 0.85 && ticked < 0.55, "{free} {ticked}");
}

fn sync_gains(frames: usize, mutate: impl Fn(&mut FwMod1Rest, usize)) -> Vec<f32> {
    let mut st = FwMod1Rest::new(10, 73, 127, 48_000.0).unwrap();
    st.set_clock_period(Some((22_857.142_857, 1.0))); // 126 BPM, one beat
    (0..frames)
        .map(|i| {
            mutate(&mut st, i);
            st.process_frame([1.0, 1.0])[0]
        })
        .collect()
}

#[test]
fn under_the_clock_the_counter_and_the_own_phase_do_not_reach_the_output() {
    // Hardware: `c[-29]` points at the clock word, so `c[-31]`, `c[-32]` and `c[-0x15]` only
    // feed the stored phase (ARM run, `fw_pump_clock` module note).
    let base = sync_gains(60_000, |_, _| {});
    let poked = sync_gains(60_000, |st, i| {
        st.st.ctx[index(-0x15)] = (i / 2) as u32 + 1; // counter changes on every block
        st.st.ctx[index(-32)] = 0x0100_0000 + i as u32;
    });
    assert_eq!(base, poked);
}

#[test]
fn the_engine_clock_word_with_zero_at_the_start_puts_the_duck_on_the_beat() {
    let g = sync_gains(60_000, |_, _| {});
    // Falling edges (gain drops by more than 0.5 within 600 frames), first in each beat.
    let mut ducks = vec![];
    let mut i = 0;
    while i + 600 < g.len() {
        if g[i] - g[i + 600] > 0.5 {
            ducks.push(i);
            i += 2_000;
        } else {
            i += 1;
        }
    }
    let beat = 22_857.142_857;
    assert!(ducks.len() >= 2, "{ducks:?}");
    for w in ducks.windows(2) {
        assert!(((w[1] - w[0]) as f64 - beat).abs() < 4.0, "{ducks:?}");
    }
    // The first duck of the clock start is complete within 600 frames of the beat.
    assert!((ducks[0] as f64 + 600.0 - beat).abs() < 700.0, "{ducks:?}");
}

#[test]
fn no_pump_word_uses_the_spare_context_word_for_the_clock() {
    // `c[0x2A]` holds the engine's clock word. The instance row and the type words of both PUMP
    // types have no entry there, and in free run the output does not depend on it.
    for words in [d::ROW_PUMP, d::WORDS_10, d::WORDS_11] {
        assert!(words.iter().all(|&(o, _)| o != CLOCK_WORD));
    }
    let run = |poke: u32| {
        let mut st = FwMod1Rest::new(10, 73, 127, 48_000.0).unwrap();
        st.st.ctx[index(CLOCK_WORD)] = poke;
        (0..20_000)
            .map(|_| st.process_frame([1.0, 1.0])[0])
            .collect::<Vec<_>>()
    };
    assert_eq!(run(0), run(0xDEAD_BEEF));
}

#[test]
fn free_run_that_was_never_synced_keeps_its_boot_words() {
    let mut st = FwMod1Rest::new(10, 73, 127, 48_000.0).unwrap();
    let before = st.st.ctx;
    st.set_clock_period(None);
    assert_eq!(st.st.ctx, before);
    st.set_clock_period(Some((22_857.0, 1.0)));
    assert_eq!(st.st.ctx[index(PUMP_POINTER)], CLOCK_ADDRESS);
    assert_eq!(st.st.ctx[index(PUMP_MULTIPLIER)], 32);
    st.set_clock_period(None);
    assert_eq!(st.st.ctx[index(PUMP_POINTER)], CTX_I1 - 31);
    assert_eq!(st.st.ctx[index(PUMP_MULTIPLIER)], 1);
}
