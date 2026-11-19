//! Sample stream tool for the SAMPLE oracle runs (D150).
//!
//! * `smp_stream resolve BACKUP.ns4b ENTRY` prints one JSON line with the `.nsmp4` member each
//!   SAMPLE synth layer of the program entry resolves to (the native name hash, `null` if none).
//! * `smp_stream stream BACKUP.ns4b MEMBER NOTE VELOCITY OUT [PCM_OUT]` writes the packed stream of the
//!   stroke that serves the note (little endian words for `sharc-ref stream`) and prints the
//!   stroke facts as one JSON line; `PCM_OUT` also gets the decoded stroke as interleaved `f32`
//!   frames (`nord-core` decoder). The stream stays in a local scratch file, never in git.
//! * `smp_stream zones BACKUP.ns4b MEMBER` prints the zone table (key / velocity range, root key,
//!   stroke rate, modelled pair block word) as a JSON array (D154).
//! * `smp_stream entry KEY ROOT RATE_HZ` prints the modelled pitch entry `e` of the key, the zone
//!   word of the stroke and the resulting SY step (`step_join`, ASSUMED writer of `e`, D65).
use nord_core::cbin::{CbinFile, Ns4BackupArchive};
use nord_core::sinc::pitch_law::{dsp_step, zone_word};
use nord_core::sinc::step_join::{table_entry_for_key, unpack_ratio};
use nord_tools::smp_stream::{stream_and_pcm, zone_table};
use std::fs::File;

fn open(path: &str) -> Result<(Ns4BackupArchive, File), String> {
    let archive = Ns4BackupArchive::open(path).map_err(|e| e.to_string())?;
    let file = File::open(path).map_err(|e| e.to_string())?;
    Ok((archive, file))
}

fn json_opt(v: &Option<String>) -> String {
    v.as_ref().map_or("null".to_string(), |s| format!("{s:?}"))
}

/// JSON of the modelled entry, zone word bits and SY step (source frames per SY frame).
fn entry_json(key: i32, root: i32, rate: f64) -> String {
    let e = table_entry_for_key(key);
    let word = zone_word(root, rate);
    let step = dsp_step(unpack_ratio(e), word);
    format!(
        "{{\"entry\":{e},\"zone_word_bits\":{},\"step\":{step:.9}}}",
        word.to_bits()
    )
}

fn run(args: &[String]) -> Result<String, String> {
    match args {
        [cmd, backup, entry] if cmd == "resolve" => {
            let (archive, mut file) = open(backup)?;
            let bytes = archive
                .read_entry(&mut file, entry)
                .map_err(|e| e.to_string())?;
            let prog = CbinFile::parse(&bytes)
                .and_then(|c| c.decode_program())
                .map_err(|e| e.to_string())?;
            let r = archive.resolve_program_sounds(&prog);
            Ok(format!(
                "{{\"synth_a\":{},\"synth_b\":{},\"synth_c\":{}}}",
                json_opt(&r.synth_a),
                json_opt(&r.synth_b),
                json_opt(&r.synth_c)
            ))
        }
        [cmd, backup, member, note, vel, out, rest @ ..] if cmd == "stream" && rest.len() <= 1 => {
            let (archive, mut file) = open(backup)?;
            let bytes = archive
                .read_entry(&mut file, member)
                .map_err(|e| e.to_string())?;
            let note: u8 = note.parse().map_err(|_| "bad NOTE".to_string())?;
            let vel: u8 = vel.parse().map_err(|_| "bad VELOCITY".to_string())?;
            let (info, stream, pcm) = stream_and_pcm(&bytes, note, vel)?;
            std::fs::write(out, &stream).map_err(|e| format!("{out}: {e}"))?;
            if let Some(pcm_out) = rest.first() {
                std::fs::write(pcm_out, &pcm).map_err(|e| format!("{pcm_out}: {e}"))?;
            }
            Ok(info.to_json())
        }
        [cmd, backup, member] if cmd == "zones" => {
            let (archive, mut file) = open(backup)?;
            let bytes = archive
                .read_entry(&mut file, member)
                .map_err(|e| e.to_string())?;
            let rows: Vec<String> = zone_table(&bytes)?.iter().map(|z| z.to_json()).collect();
            Ok(format!("[{}]", rows.join(",")))
        }
        [cmd, key, root, rate] if cmd == "entry" => {
            let key: i32 = key.parse().map_err(|_| "bad KEY".to_string())?;
            let root: i32 = root.parse().map_err(|_| "bad ROOT".to_string())?;
            let rate: f64 = rate.parse().map_err(|_| "bad RATE_HZ".to_string())?;
            Ok(entry_json(key, root, rate))
        }
        _ => Err(
            "usage: smp_stream resolve BACKUP ENTRY | zones BACKUP MEMBER | stream BACKUP MEMBER NOTE VEL OUT [PCM_OUT] | entry KEY ROOT RATE"
                .into(),
        ),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(text) => println!("{text}"),
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
    fn usage_and_missing_files_are_errors() {
        assert!(run(&[]).is_err());
        assert!(run(&["resolve".into(), "/nonexistent.ns4b".into(), "x".into()]).is_err());
        assert!(run(&["entry".into(), "x".into(), "60".into(), "1".into()]).is_err());
        assert!(run(&["zones".into(), "/nonexistent.ns4b".into(), "m".into()]).is_err());
        assert!(json_opt(&None) == "null" && json_opt(&Some("a".into())) == "\"a\"");
    }

    #[test]
    fn modelled_entry_of_the_root_key_steps_at_rate_over_sy_frame() {
        // ASSUMED model of `e` (step_join, D65): the root key plays the native rate.
        let j = entry_json(60, 60, 35012.0);
        let step: f64 = j
            .split("\"step\":")
            .nth(1)
            .unwrap()
            .trim_end_matches('}')
            .parse()
            .unwrap();
        assert!((step - 35012.0 / 43402.34375).abs() < 1e-3, "{j}");
        assert!(run(&["entry".into(), "60".into(), "60".into(), "35012".into()]).is_ok());
    }
}
