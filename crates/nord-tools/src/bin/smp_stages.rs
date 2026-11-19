//! Native engine level per stage for one program and note (D154, `docs/research/sample-gaps.md`).
//!
//! `smp_stages BACKUP.ns4b ENTRY [NOTE VELOCITY FRAMES]` loads the program like `ns4 render-patch`
//! (sounds from the backup, default firmware options), then prints one JSON line with the peak of
//! each Synth layer at four stages and of the final Main pair:
//!
//! Each stage is reported as the peak of the louder channel (`*_db`) and of the mono mean
//! (`*_mono_db`).
//!
//! 1. `voice`: `SynthLayer::render_frame` (resampler, filter, amp envelope x velocity x 0.25);
//! 2. `fx`: after the layer FX chain;
//! 3. `fader`: after the layer fader and the program level (what `mix_outputs` adds to the bus);
//! 4. `out`: the Main pair of `render_frame` on a second engine (master bus, trim).
//!
//! The mix is replayed here, not copied into the engine: `mix_outputs` stays the only mix.
use nord_core::cbin::Ns4BackupArchive;
use nord_core::engine::Stage4Engine;
use nord_core::midi::MidiEvent;
use std::fs::File;

const RATE: f32 = 48_000.0;
const SYNTH_FIRST: usize = 4;

/// Peak of a signal in dBFS (`-240` for silence).
fn peak_db(peak: f32) -> f32 {
    if peak > 1e-12 {
        20.0 * peak.log10()
    } else {
        -240.0
    }
}

/// Peak over both channels (`ch`) and of the mono mean `(l + r) / 2` (`mono`, what the oracle
/// comparison of `sample_stream_audio.py` measures).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Peak {
    ch: f32,
    mono: f32,
}

/// Running maximum of `|x|` over both channels and of the mono mean of a frame.
fn bump(peak: &mut Peak, f: [f32; 2]) {
    peak.ch = peak.ch.max(f[0].abs()).max(f[1].abs());
    peak.mono = peak.mono.max((0.5 * (f[0] + f[1])).abs());
}

fn load(backup: &str, entry: &str) -> Result<Stage4Engine, String> {
    let archive = Ns4BackupArchive::open(backup).map_err(|e| e.to_string())?;
    let mut file = File::open(backup).map_err(|e| e.to_string())?;
    let mut e = Stage4Engine::new(RATE);
    e.load_program_with_sounds_from_backup(&archive, &mut file, entry)
        .map_err(|x| x.to_string())?;
    nord_core::render_opts::FirmwareOptions::from_args(&[])?.apply_to(&mut e);
    Ok(e)
}

fn run(args: &[String]) -> Result<String, String> {
    let (backup, entry) = match args {
        [b, e, ..] => (b.as_str(), e.as_str()),
        _ => return Err("usage: smp_stages BACKUP.ns4b ENTRY [NOTE VELOCITY FRAMES]".into()),
    };
    let num = |i: usize, d: u32| args.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    let (note, velocity, frames) = (num(2, 60) as u8, num(3, 100) as u8, num(4, 72_000) as usize);
    let on = MidiEvent::NoteOn {
        channel: 0,
        note,
        velocity,
    };

    let mut tap = load(backup, entry)?;
    let mut main = load(backup, entry)?;
    tap.handle_midi(on);
    main.handle_midi(on);
    let (mut voice, mut fx, mut fader) = (
        [Peak::default(); 3],
        [Peak::default(); 3],
        [Peak::default(); 3],
    );
    let mut out = Peak::default();
    let level_db = tap.program_level_db();
    let program_gain = 10f32.powf(level_db / 20.0);
    let level: Vec<f32> = (0..3)
        .map(|k| {
            let l = tap.layer_levels[SYNTH_FIRST + k].evaluate(&tap.morph_controllers);
            nord_core::piano::downstream::layer_gain(l, tap.synth_layer_law)
        })
        .collect();
    for _ in 0..frames {
        let raw = [
            tap.synth_a.render_frame(),
            tap.synth_b.render_frame(),
            tap.synth_c.render_frame(),
        ];
        for (k, r) in raw.into_iter().enumerate() {
            bump(&mut voice[k], r);
            let f = tap.fx_chains[SYNTH_FIRST + k].process_frame_with_bpm(r, tap.master_clock.bpm);
            bump(&mut fx[k], f);
            let g = level[k] * program_gain;
            bump(&mut fader[k], [f[0] * g, f[1] * g]);
        }
        bump(&mut out, main.render_frame());
    }
    let row = |p: &[Peak; 3], mono: bool| {
        let v = |k: usize| peak_db(if mono { p[k].mono } else { p[k].ch });
        format!("[{:.2},{:.2},{:.2}]", v(0), v(1), v(2))
    };
    Ok(format!(
        "{{\"entry\":{entry:?},\"note\":{note},\"velocity\":{velocity},\"voice_db\":{},\"fx_db\":{},\"fader_db\":{},\"out_db\":{:.2},\"voice_mono_db\":{},\"fx_mono_db\":{},\"fader_mono_db\":{},\"out_mono_db\":{:.2},\"layer_gain\":[{:.4},{:.4},{:.4}],\"program_level_db\":{level_db:.2}}}",
        row(&voice, false), row(&fx, false), row(&fader, false), peak_db(out.ch),
        row(&voice, true), row(&fx, true), row(&fader, true), peak_db(out.mono),
        level[0], level[1], level[2]
    ))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(s) => println!("{s}"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_db_of_silence_and_unity() {
        assert_eq!(peak_db(0.0), -240.0);
        assert!(peak_db(1.0).abs() < 1e-6);
        assert!((peak_db(0.5) + 6.0206).abs() < 1e-3);
    }

    #[test]
    fn bump_keeps_the_largest_magnitude_of_both_channels_and_of_the_mono_mean() {
        let mut p = Peak::default();
        bump(&mut p, [0.25, -0.5]);
        bump(&mut p, [0.1, 0.2]);
        assert_eq!(p.ch, 0.5);
        assert_eq!(p.mono, 0.15); // (0.1 + 0.2) / 2 beats |0.25 - 0.5| / 2 = 0.125
    }

    #[test]
    fn run_rejects_missing_arguments() {
        assert!(run(&[]).is_err());
    }
}
