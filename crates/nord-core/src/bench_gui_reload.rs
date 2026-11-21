//! `ns4 bench --gui --reload N`: the cost of a program change on the audio thread.
//!
//! The GUI applies `AudioCommand::LoadProgram` (`Stage4Engine::apply_program`) inside the audio
//! callback, so the block that carries it pays for the load and for every lazy stage the new
//! program builds on first use (firmware Mod 2 stages, reverbs; TD-18). This module plays a chord,
//! then alternates the programs `--program` and `--program2` (or re-applies one) and times the
//! `apply_program` call and the blocks that follow it against the block deadline.

use crate::bench::{SAMPLE_RATE, STANDALONE_MASTER_VOL};
use crate::bench_gui::{engine, GuiBenchConfig};
use crate::bench_gui_prog::ProgramSource;
use crate::cbin::ProgramDecoded;
use crate::engine::Stage4Engine;
use crate::midi::MidiEvent;
use std::time::Instant;

/// Blocks timed after every load (the first one carries the lazy builds).
pub const AFTER_BLOCKS: usize = 3;

/// Timing of one load and the blocks after it, milliseconds.
#[derive(Debug, Clone, PartialEq)]
pub struct ReloadEvent {
    pub apply_ms: f64,
    pub after_ms: [f64; AFTER_BLOCKS],
}

/// Summary of a reload run.
#[derive(Debug, Clone, PartialEq)]
pub struct ReloadReport {
    pub deadline_ms: f64,
    /// Mean time of the steady blocks between loads.
    pub steady_mean_ms: f64,
    pub events: Vec<ReloadEvent>,
}

impl ReloadReport {
    /// Slowest `apply_program` call.
    pub fn worst_apply_ms(&self) -> f64 {
        self.events.iter().map(|e| e.apply_ms).fold(0.0, f64::max)
    }

    /// Slowest block after a load (the block that follows the apply is included in the callback
    /// that carried it, so the worst "load callback" is `apply_ms + after_ms[0]`).
    pub fn worst_load_callback_ms(&self) -> f64 {
        self.events
            .iter()
            .map(|e| e.apply_ms + e.after_ms[0])
            .fold(0.0, f64::max)
    }

    /// Loads whose callback (apply plus its first block) exceeded the deadline.
    pub fn late_loads(&self) -> usize {
        self.events
            .iter()
            .filter(|e| e.apply_ms + e.after_ms[0] > self.deadline_ms)
            .count()
    }

    /// Mean extra time of the first block after a load over the two blocks that follow it, and the
    /// worst such excess: what the load itself and the lazily built stages add to one callback.
    pub fn first_block_excess_ms(&self) -> (f64, f64) {
        let ex: Vec<f64> = self
            .events
            .iter()
            .map(|e| e.apply_ms + e.after_ms[0] - (e.after_ms[1] + e.after_ms[2]) / 2.0)
            .collect();
        (
            ex.iter().sum::<f64>() / ex.len().max(1) as f64,
            ex.iter().copied().fold(f64::MIN, f64::max),
        )
    }

    /// Report text.
    pub fn format(&self) -> String {
        let worst_after = self
            .events
            .iter()
            .flat_map(|e| e.after_ms.iter().skip(1))
            .copied()
            .fold(0.0, f64::max);
        let (mean_excess, worst_excess) = self.first_block_excess_ms();
        format!(
            "program reload: {} loads, deadline {:.3} ms, steady mean {:.3} ms\n  apply_program worst {:.3} ms, mean {:.3} ms\n  load callback (apply + first block) worst {:.3} ms, late {}/{}\n  blocks 2..{} after a load: worst {:.3} ms
  load callback excess over the next two blocks: mean {:.3} ms, worst {:.3} ms",
            self.events.len(),
            self.deadline_ms,
            self.steady_mean_ms,
            self.worst_apply_ms(),
            self.events.iter().map(|e| e.apply_ms).sum::<f64>() / self.events.len().max(1) as f64,
            self.worst_load_callback_ms(),
            self.late_loads(),
            self.events.len(),
            AFTER_BLOCKS,
            worst_after,
            mean_excess,
            worst_excess,
        )
    }
}

fn render_block(e: &mut Stage4Engine, frames: usize) -> f64 {
    let t0 = Instant::now();
    let mut s = 0.0f32;
    for _ in 0..frames {
        let [l, r] = e.render_frame();
        s += (l + r) * STANDALONE_MASTER_VOL;
    }
    std::hint::black_box(s);
    t0.elapsed().as_secs_f64() * 1000.0
}

/// Plays a chord on the engine `cfg` and `source` describe, then applies `programs` in turn
/// `loads` times (cycling), timing each `apply_program` and the blocks after it.
pub fn run(
    cfg: &GuiBenchConfig,
    source: &ProgramSource,
    programs: &[ProgramDecoded],
    loads: usize,
) -> Result<ReloadReport, String> {
    let mut e = engine(cfg);
    source.apply(cfg, &mut e)?;
    for k in 0..cfg.keys {
        e.handle_midi(MidiEvent::NoteOn {
            channel: 0,
            note: 36 + k,
            velocity: 100,
        });
    }
    for _ in 0..cfg.warmup_blocks.max(4) {
        render_block(&mut e, cfg.frames);
    }
    let mut steady = Vec::new();
    let mut events = Vec::with_capacity(loads);
    for i in 0..loads {
        let prog = &programs[i % programs.len()];
        let t0 = Instant::now();
        e.apply_program(prog);
        cfg.options.apply_exact(&mut e);
        let apply_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let mut after_ms = [0.0; AFTER_BLOCKS];
        for a in after_ms.iter_mut() {
            *a = render_block(&mut e, cfg.frames);
        }
        events.push(ReloadEvent { apply_ms, after_ms });
        for _ in 0..4 {
            steady.push(render_block(&mut e, cfg.frames));
        }
    }
    Ok(ReloadReport {
        deadline_ms: cfg.frames as f64 / f64::from(SAMPLE_RATE) * 1000.0,
        steady_mean_ms: steady.iter().sum::<f64>() / steady.len().max(1) as f64,
        events,
    })
}

/// Decodes the programs the run cycles through: `--program`, then `--program2` when given, then
/// `pool` more programs spread evenly over the archive (`--reload-pool`), so a run covers many
/// different FX settings (Mod 2 types, reverbs) instead of one.
pub fn decode_programs(
    source: &ProgramSource,
    second: Option<&str>,
    pool: usize,
) -> Result<Vec<ProgramDecoded>, String> {
    let archive = crate::cbin::Ns4BackupArchive::open(&source.backup).map_err(|x| x.to_string())?;
    let mut file = std::fs::File::open(&source.backup).map_err(|x| x.to_string())?;
    let mut names: Vec<String> = std::iter::once(source.entry.as_str())
        .chain(second)
        .map(str::to_string)
        .collect();
    let all: Vec<String> = archive
        .entries_with_ext("ns4p")
        .iter()
        .map(|e| e.name.clone())
        .collect();
    if pool > 0 && !all.is_empty() {
        let step = (all.len() / pool).max(1);
        names.extend(all.iter().step_by(step).take(pool).cloned());
    }
    let mut out = Vec::new();
    for name in &names {
        let bytes = archive
            .read_entry(&mut file, name)
            .map_err(|x| x.to_string())?;
        let cbin = crate::cbin::CbinFile::parse(&bytes).map_err(|x| x.to_string())?;
        out.push(cbin.decode_program().map_err(|x| x.to_string())?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_times_every_load_and_the_blocks_after_it() {
        let cfg = GuiBenchConfig {
            layers: 3,
            keys: 3,
            frames: 128,
            ..GuiBenchConfig::default()
        };
        let programs = [ProgramDecoded::default(), ProgramDecoded::default()];
        let r = run(&cfg, &ProgramSource::default(), &programs, 5).unwrap();
        assert_eq!(r.events.len(), 5);
        assert!(r.steady_mean_ms > 0.0 && r.worst_apply_ms() > 0.0);
        assert!(r.worst_load_callback_ms() >= r.worst_apply_ms());
        let text = r.format();
        assert!(
            text.contains("5 loads") && text.contains("apply_program worst"),
            "{text}"
        );
    }

    #[test]
    fn late_loads_compare_apply_plus_first_block_with_the_deadline() {
        let r = ReloadReport {
            deadline_ms: 10.0,
            steady_mean_ms: 4.0,
            events: vec![
                ReloadEvent {
                    apply_ms: 1.0,
                    after_ms: [4.0, 4.0, 4.0],
                },
                ReloadEvent {
                    apply_ms: 8.0,
                    after_ms: [4.0, 4.0, 4.0],
                },
            ],
        };
        assert_eq!(r.late_loads(), 1);
        assert_eq!(r.worst_load_callback_ms(), 12.0);
    }
}
