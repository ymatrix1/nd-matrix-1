//! Voice-level tests of the `.npno` / SFZ sample loop and the tail fade (moved out of `piano.rs`).

use super::tests::build_test_wav_i16;
use super::*;

fn looped_sound(loop_decay: i16) -> Arc<NpnoSound> {
    let sfz = "<region> sample=a.wav lokey=0 hikey=127 pitch_keycenter=60
";
    let wav = build_test_wav_i16(48_000, 261.63, 4096);
    let mut sound = NpnoSound::from_sfz_str("Loop", sfz, |_| Ok(wav.clone())).unwrap();
    sound.strokes[0].loop_start = Some(2048);
    sound.strokes[0].loop_end = Some(4096);
    sound.strokes[0].loop_decay = loop_decay;
    Arc::new(sound)
}

/// Issue #640: the firmware wraps a held stroke at the loop end, so the note outlasts its
/// sample instead of ending (hardware: SMP `F-8` / `F-7`, `docs/research/piano-sample-end.md`).
#[test]
fn held_looped_note_outlasts_its_sample() {
    let mut piano = PianoLayer::new(48_000.0);
    piano.set_sound(Some(looped_sound(0)));
    piano.note_on(60, 110);
    let out: Vec<f32> = (0..30_000).map(|_| piano.render_frame()[0].abs()).collect();
    let peak = |a: usize, b: usize| out[a..b].iter().cloned().fold(0.0f32, f32::max);
    assert!(
        piano.voices.iter().any(|v| v.active),
        "voice must survive the stroke end"
    );
    assert!(
        peak(29_000, 30_000) > 0.8 * peak(1000, 2000),
        "looped tail must keep sounding"
    );
}

/// Inside the loop the firmware rate word `F+6` makes every repetition quieter.
#[test]
fn looped_note_decays_in_the_loop_and_is_freed_when_silent() {
    let mut piano = PianoLayer::new(48_000.0);
    piano.set_sound(Some(looped_sound(-20_000)));
    piano.note_on(60, 110);
    let out: Vec<f32> = (0..60_000).map(|_| piano.render_frame()[0].abs()).collect();
    let peak = |a: usize, b: usize| out[a..b].iter().cloned().fold(0.0f32, f32::max);
    let early = peak(4_200, 5_200);
    let late = peak(20_000, 21_000);
    assert!(late < 0.5 * early, "loop must decay: {late} vs {early}");
    assert!(late > 0.0);
    assert!(
        piano.voices.iter().all(|v| !v.active),
        "voice freed when decayed"
    );
}

/// Issue #640: a held note that outlasts its sample must fade out, not stop at full level.
#[test]
fn held_note_fades_out_at_sample_end() {
    let sfz = "<region> sample=a.wav lokey=0 hikey=127 pitch_keycenter=60\n";
    let wav = build_test_wav_i16(48_000, 261.63, 4096);
    let sound =
        Arc::new(NpnoSound::from_sfz_str("Tail", sfz, |_| Ok(wav.clone())).expect("SFZ parses"));
    let mut piano = PianoLayer::new(48_000.0);
    piano.set_sound(Some(sound));
    piano.note_on(60, 110);
    let out: Vec<f32> = (0..6000).map(|_| piano.render_frame()[0].abs()).collect();
    let peak = |a: usize, b: usize| out[a..b].iter().cloned().fold(0.0f32, f32::max);
    let body = peak(1000, 1200);
    assert!(body > 1e-3, "note must sound: {body}");
    // The last 64 frames before the sample ends sit at the bottom of the fade.
    assert!(
        peak(4096 - 64, 4096) < 0.05 * body,
        "tail not faded: {} vs {body}",
        peak(4096 - 64, 4096)
    );
    assert!(
        piano.voices.iter().all(|v| !v.active),
        "voice freed after the sample"
    );
}
