//! Filter laws of the strip chain read from the firmware (opt-in, `--synth-filter-law`,
//! `--synth-fw-filter`).
//!
//! Two derived facts replace the fitted per-type cutoff factors of [`super::p_type`] and the
//! linear resonance to table position map of `TablePos::from_resonance`:
//!
//! * **Resonance word to table position (VERIFIED, run).** The ARM writes `kFltRes` as the float
//!   word `w = knob / 127 - 0.5` to the common block word 7 (`docs/research/synth-knob-laws.md`
//!   s3). The ORG core 2 pass smooths it (`w10 <- 0.995 w10 + 0.005 w` once per pass) and turns it
//!   into the stage position word `W28 = DM(0xC2926)` with `idx(n) = 64 + 128 w (1 - 0.995^(n+1))`
//!   at pass `n` (`tools/synth_filter_links.py res`, oracle = model to 0.001). The settled
//!   position is `128 knob / 127` (clipped at `127.999`): within one table index of the engine default
//!   `TablePos::from_resonance` (`127 r`). The earlier span `33.25` was the transient at pass 59
//!   (`128 (1 - 0.995^60)`), not the steady state (`docs/research/synth-filter-links.md` s2).
//! * **One nominal cutoff for every type (VERIFIED, derived).** The ARM cutoff word is type
//!   independent and every stage of the family takes the same coefficient `R` with the nominal
//!   cutoff `atan(R) / pi` cycles per sample (`docs/research/synth-filter-type-corners.md`), so
//!   there is no per-type cutoff factor; the shape differences between the types live in the
//!   stages that [`super::fw_filter`] already ports. The strip word 1 is the stage id selected by
//!   the file's filter type through an identity table (VERIFIED, 18 of 18 layers,
//!   `docs/research/synth-filter-links.md` s1).
//!
//! What stays ASSUMED is the unit: the display shows `1.971 x` the natural frequency
//! `F atan(R) / pi` ([`DISPLAY_OVER_NATURAL`]). The stage entry runs 4 samples per call and 16
//! calls per pass (VERIFIED, `tools/synth_filter_links.py calls`), so the stage rate is `2 F` when
//! a pass is two frames ([`UNIT_2F`], `docs/research/synth-filter-links.md` s3).

/// VERIFIED: the settled table position is `128 w` above the centre (`w = knob / 127 - 0.5`),
/// i.e. `128 knob / 127` (`tools/synth_filter_links.py res`, passes up to 6000, error < 0.01).
pub const RES_POSITION_SPAN: f32 = 128.0;
/// VERIFIED: table position at the neutral word `w = 0` (`W28 = 0x40000000`, idle).
pub const RES_POSITION_CENTRE: f32 = 64.0;

/// VERIFIED: display Hz over the natural frequency `F atan(R) / pi` of the stage (mean 1.971,
/// range 1.88 to 2.06 over knobs 8..116, `docs/research/data/synth-cutoff-voice-law.csv`).
pub const DISPLAY_OVER_NATURAL: f32 = 1.971;
/// ASSUMED: stage rate `F` (one stage sample per DSP frame): the engine cutoff is the natural
/// frequency, `1 / 1.971` of the display.
pub const UNIT_F: f32 = 1.0 / DISPLAY_OVER_NATURAL;
/// ASSUMED: stage rate `2 F`: the corner in Hz doubles, `2 / 1.971` of the display. Supported by
/// the 4-samples-per-call and 16-calls-per-pass counts and the pass = 2 frames envelope timing;
/// a direct capture of the hardware corner would confirm it.
pub const UNIT_2F: f32 = 2.0 / DISPLAY_OVER_NATURAL;

/// Table position (`0..=127.999`) of the firmware resonance word for a panel resonance `0..=1`
/// (`knob / 127`), the settled value of the pass smoother.
#[inline]
pub fn resonance_position(resonance: f32) -> f32 {
    let w = resonance.clamp(0.0, 1.0) - 0.5;
    (RES_POSITION_CENTRE + RES_POSITION_SPAN * w).min(127.999)
}

/// Resonance that `TablePos::from_resonance` maps to the firmware position.
#[inline]
pub fn resonance_for_table(resonance: f32) -> f32 {
    resonance_position(resonance) / 127.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::TablePos;

    #[test]
    fn position_is_the_settled_oracle_value() {
        // oracle settled: 128 knob / 127 (tools/synth_filter_links.py res, n -> inf)
        for knob in [0.0f32, 32.0, 64.0, 107.0] {
            let want = 128.0 * knob / 127.0;
            let got = resonance_position(knob / 127.0);
            assert!((got - want).abs() < 1e-3, "{knob}: {got} vs {want}");
        }
    }

    #[test]
    fn range_spans_the_table_and_clips_at_the_end() {
        assert!(resonance_position(0.0).abs() < 1e-4);
        assert!((resonance_position(0.5) - 64.0).abs() < 1e-6);
        assert!((resonance_position(1.0) - 127.999).abs() < 1e-4);
    }

    #[test]
    fn table_position_is_within_one_index_of_the_default_map() {
        for r in [0.0f32, 0.25, 0.5, 0.75] {
            let a = TablePos::from_resonance(resonance_for_table(r));
            let b = TablePos::from_resonance(r);
            // 128 r against 127 r: at most one table index apart
            let d = (a.idx as f32 + a.fr) - (b.idx as f32 + b.fr);
            assert!((0.0..=1.0).contains(&d), "{r}: {d}");
        }
        let top = TablePos::from_resonance(resonance_for_table(1.0));
        assert_eq!(top.idx, 127);
        assert!(top.fr > 0.99);
    }

    #[test]
    fn units_are_one_over_and_two_over_the_display_ratio() {
        assert!((UNIT_2F / UNIT_F - 2.0).abs() < 1e-6);
        assert!((UNIT_2F - 1.0147).abs() < 1e-3);
    }

    /// VERIFIED (oracle, all 128 resonance knobs): the native damping table lookup equals the one
    /// the ORG core 2 stage used. `docs/research/data/synth-res-settled.csv` holds, per knob, the
    /// settled position word `W28` and the damping `P = 2 R k / g` read from the LP24 and LP12
    /// stage prologues (`tools/synth_res_oracle.py settled`). The native position map equals the
    /// oracle position (the 1500 pass run leaves at most 0.035 of the smoother), the table at the
    /// oracle position equals the oracle `P` (float32 rounding), and the native map of the knob
    /// lands within the table slope times that residual.
    #[test]
    fn native_damping_equals_the_oracle_for_all_128_knobs() {
        use crate::synth::fw_filter::{FwCoefs, FwFilterType};
        const CSV: &str = include_str!("../../../../docs/research/data/synth-res-settled.csv");
        let damping = |kind: FwFilterType, pos: f32| {
            let c = FwCoefs::new(kind, 0.3, TablePos::from_value(pos));
            2.0 * 0.3 * c.k / c.g
        };
        let mut n = 0;
        for line in CSV.lines().skip(1) {
            let f: Vec<f32> = line.split(',').map(|x| x.parse().unwrap()).collect();
            let (knob, pos, p24, p12) = (f[0], f[1], f[3], f[4]);
            let ours = resonance_position(knob / 127.0);
            assert!((ours - pos).abs() < 0.04, "knob {knob}: {ours} vs {pos}");
            for (kind, want) in [(FwFilterType::Lp4, p24), (FwFilterType::Lp2, p12)] {
                let at_oracle = damping(kind, pos);
                // the CSV keeps 4 decimals of the position: 5e-5 index times the slope of the table
                assert!(
                    (at_oracle / want - 1.0).abs() < 1e-4,
                    "{kind:?} knob {knob}"
                );
                assert!(
                    (damping(kind, ours) - want).abs() < 1.5e-3,
                    "{kind:?} knob {knob}"
                );
            }
            n += 1;
        }
        assert_eq!(n, 128);
    }
}
