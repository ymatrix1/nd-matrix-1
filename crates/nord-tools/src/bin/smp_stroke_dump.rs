//! `smp_stroke_dump FILE.npno STROKE [OUT.f32]`: prints the descriptor of one stroke of a local
//! `.npno` and writes its decoded PCM (f32 little endian, `L R` per frame, mono repeated) for the
//! comparison with the live SMP tap (D200, `docs/research/smp-voice-pitch.md`). Inputs stay local.
use nord_core::cbin::piano_sample::{decode_npno_stroke, NpnoInstrument};

/// `L R` interleaved from the decoded channels (mono is repeated).
fn interleave(left: &[f32], right: Option<&[f32]>) -> Vec<f32> {
    left.iter()
        .enumerate()
        .flat_map(|(i, l)| [*l, right.map_or(*l, |r| r[i])])
        .collect()
}

fn describe(inst: &NpnoInstrument, idx: usize) -> Result<String, String> {
    let s = inst.strokes.get(idx).ok_or("stroke out of range")?;
    Ok(format!(
        "key 60: root {} tuning {} cents; stroke {idx}: root_key {} rate {} channels {} npages {} frames {} vel_low {} kind {} init_left {:?}",
        inst.key_map[60], inst.tuning_cents[60], s.root_key, s.sample_rate, s.channels, s.npages, s.total_frames, s.vel_low, s.stroke_kind,
        s.init_left
    ))
}

fn run(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .ok_or("usage: smp_stroke_dump FILE.npno STROKE [OUT.f32]")?;
    let idx: usize = args
        .get(1)
        .ok_or("missing STROKE")?
        .parse()
        .map_err(|_| "bad STROKE")?;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let inst = NpnoInstrument::parse(&bytes)?;
    println!("{}", describe(&inst, idx)?);
    if let Some(out) = args.get(2) {
        let (l, r) = decode_npno_stroke(&inst.strokes[idx], &bytes)?;
        let pcm = interleave(&l, r.as_deref());
        let raw: Vec<u8> = pcm.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(out, raw).map_err(|e| e.to_string())?;
        println!("wrote {} frames to {out}", pcm.len() / 2);
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interleave_repeats_mono_and_pairs_stereo() {
        assert_eq!(interleave(&[1.0, 2.0], None), vec![1.0, 1.0, 2.0, 2.0]);
        assert_eq!(
            interleave(&[1.0, 2.0], Some(&[3.0, 4.0])),
            vec![1.0, 3.0, 2.0, 4.0]
        );
    }

    #[test]
    fn test_run_reports_usage_and_bad_stroke() {
        assert!(run(&[]).is_err());
        assert!(run(&["x.npno".into(), "nan".into()]).is_err());
    }
}
