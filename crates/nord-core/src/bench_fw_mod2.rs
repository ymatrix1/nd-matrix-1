//! Bench workloads with the firmware Mod 2 running (D163).
//!
//! The firmware Mod 2 is the default but only runs for a chain that carries the raw `kFx2Type` /
//! `kFx2RateClkDiv` words of a loaded program; the plain worst-case engine of `bench` sets
//! `mod2_type` by type, stays on the engine path and never exercises it. These engines are the
//! worst case with the raw words set on every chain, so `ns4 bench --scenario worst-fw-chorus`
//! and `worst-fw-spin` measure what a loaded Mod 2 program costs against `worst` (engine Chorus).

use crate::bench::{press_all_keys, worst_case_engine};
use crate::engine::Stage4Engine;
use crate::fx::Mod2Type;

/// Raw `kFx2Type` of the chorus (`CHOR`, 4) and of the rotary (`SPIN`, 10).
pub const RAW_CHORUS: u8 = 4;
pub const RAW_SPIN: u8 = 10;
/// Raw 7 bit `kFx2RateClkDiv` of the bench: mid scale.
/// ASSUMED: any rate gives the same cost per frame; mid scale keeps the stage in its normal range.
pub const RAW_RATE: u8 = 64;

/// The worst-case engine with the firmware Mod 2 of raw type `ty` on every layer chain and every
/// key held. `kind` is the engine type kept next to it (what the GUI shows).
pub fn fw_mod2_engine(sample_rate: f32, ty: u8, kind: Mod2Type) -> Stage4Engine {
    let mut e = worst_case_engine(sample_rate);
    for chain in e.fx_chains.iter_mut() {
        chain.mod2_type = kind;
        chain.mod2_type_raw = Some(ty);
        chain.mod2_rate_raw = Some(RAW_RATE);
        chain.mod2_amount = 0.8;
    }
    press_all_keys(&mut e);
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::{run, run_engine, BenchConfig, Scenario};

    #[test]
    fn the_raw_words_reach_every_chain_and_the_hash_differs_from_the_engine_path() {
        let e = fw_mod2_engine(48_000.0, RAW_CHORUS, Mod2Type::Chorus);
        assert!(e
            .fx_chains
            .iter()
            .all(|c| c.firmware_fx.mod2 && c.mod2_type_raw == Some(RAW_CHORUS)));
        let cfg = BenchConfig {
            scenario: Scenario::Worst,
            warmup_blocks: 10,
            blocks: 20,
            multi_out: true,
        };
        let engine_path = run(cfg);
        let firmware = run_engine(cfg, e);
        assert!(firmware.all_finite && firmware.peak > 0.0);
        assert_ne!(engine_path.hash, firmware.hash, "the stage under test runs");
    }
}
