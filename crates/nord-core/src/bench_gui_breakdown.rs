//! The `--breakdown` rows of `ns4 bench --gui`: the same run with parts of the engine switched off.

use crate::bench::SAMPLE_RATE;
use crate::bench_gui::{run_with, GuiBenchConfig, GuiBenchReport};
use crate::engine::Stage4Engine;

/// Tail figures with parts of the engine switched off, to find what the spikes come from. The
/// rows are the full engine, FX racks off, no keys, and each layer alone (FX off).
pub fn breakdown(cfg: GuiBenchConfig) -> Vec<(String, GuiBenchReport)> {
    breakdown_with(cfg, &|_| {})
}

/// [`breakdown`] on the engine `base` prepares (a loaded program, an arpeggiator).
pub fn breakdown_with(
    cfg: GuiBenchConfig,
    base: &dyn Fn(&mut Stage4Engine),
) -> Vec<(String, GuiBenchReport)> {
    use crate::fx::{LayerFxChain, ReverbModel};
    let fx_off = |e: &mut Stage4Engine| {
        base(e);
        for chain in e.fx_chains.iter_mut() {
            *chain = LayerFxChain::new(SAMPLE_RATE);
            chain.reverb_model = ReverbModel::Off;
        }
        e.master_bus.compressor_amount = 0.0;
    };
    let mut rows = vec![("full".to_string(), run_with(cfg, base))];
    rows.push(("FX racks + master comp off".into(), run_with(cfg, fx_off)));
    rows.push((
        "no keys held".into(),
        run_with(GuiBenchConfig { keys: 0, ..cfg }, base),
    ));
    let names = [
        "Organ A", "Organ B", "Piano A", "Piano B", "Synth A", "Synth B", "Synth C",
    ];
    let count = usize::from(cfg.layers.min(7));
    let all: [usize; 7] = [0, 1, 2, 3, 4, 5, 6];
    // Scene I plays Organ A, Piano A, Synth A; Scene II all seven.
    let in_scene: Vec<usize> = if count > 3 {
        all.to_vec()
    } else {
        vec![0, 2, 4]
    };
    for i in in_scene {
        let only = move |e: &mut Stage4Engine| {
            fx_off(e);
            e.scene.scene_2_mask = 1 << i;
            e.scene.scene_1_mask = 1 << i;
        };
        rows.push((format!("{} only, FX off", names[i]), run_with(cfg, only)));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench_gui::format_row;

    #[test]
    fn breakdown_names_the_rows_for_both_scenes() {
        let rows = breakdown(GuiBenchConfig {
            blocks: 8,
            keys: 4,
            ..GuiBenchConfig::default()
        });
        assert_eq!(rows.len(), 3 + 7);
        let rows3 = breakdown(GuiBenchConfig {
            layers: 3,
            blocks: 8,
            keys: 4,
            ..GuiBenchConfig::default()
        });
        assert_eq!(rows3.len(), 3 + 3);
        assert!(format_row(&rows3[0].0, &rows3[0].1).contains("full"));
    }
}
