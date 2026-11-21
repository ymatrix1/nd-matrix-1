//! Option groups of the program ranking (`program_rank --ablate`, `docs/research/program-ranking.md`):
//! the fields by which `--firmware-all --firmware-routing` differs from the engine default, split in
//! eight groups that can be put back to the default one at a time or in combinations.
//!
//! The six FX groups are the ones that explain the ranked distance of the worst programs, so
//! [`fx_subsets`] lists every non-empty combination of them (63) for the combined ablation.

use nord_core::render_opts::FirmwareOptions;

/// Puts the fields of one group of the first options back to the values of the second.
pub type Revert = fn(&mut FirmwareOptions, &FirmwareOptions);

/// Number of groups.
pub const GROUP_COUNT: usize = 8;

/// Index of the first FX group in [`GROUPS`] (`fx_delay`); the six FX groups follow it.
pub const FIRST_FX: usize = 2;

/// Number of FX groups.
pub const FX_COUNT: usize = 6;

/// Name and revert function of every group. Organ tone wheels and the synth filter engine come
/// first, then the FX groups.
pub const GROUPS: [(&str, Revert); GROUP_COUNT] = [
    ("organ_tone", |o, d| o.tone_source = d.tone_source),
    ("synth_filter", |o, d| o.filter_engine = d.filter_engine),
    ("fx_delay", |o, d| {
        o.fx.delay_laws = d.fx.delay_laws;
        o.fx.delay_time = d.fx.delay_time;
    }),
    ("fx_compressor", |o, d| o.fx.compressor = d.fx.compressor),
    ("fx_reverb", |o, d| {
        o.fx.reverb_mix = d.fx.reverb_mix;
        o.fx.reverb = d.fx.reverb;
    }),
    ("fx_eq", |o, d| o.fx.eq = d.fx.eq),
    ("fx_mod1", |o, d| o.fx.mod1 = d.fx.mod1),
    ("fx_mod2", |o, d| o.fx.mod2 = d.fx.mod2),
];

/// Flip-only sub-groups of `fx_delay` (indices `GROUP_COUNT` and `GROUP_COUNT + 1`): they split the
/// delay group into its two ports so a default flip of one port can be measured alone
/// (`program_rank --flip fx_delay_time`). They are not part of the ablation or the 63 subsets.
pub const SUBGROUPS: [(&str, Revert); 5] = [
    ("fx_delay_time", |o, d| o.fx.delay_time = d.fx.delay_time),
    ("fx_delay_laws", |o, d| o.fx.delay_laws = d.fx.delay_laws),
    // The rate match (D137, default on since D176) is a converter, not a port: the flip switches
    // it on whatever the firmware options say, so it is only a change against a `--legacy-fx-rate`
    // (or older) baseline.
    ("fx_rate_match", |o, _| o.fx_rate_match = true),
    // Opt-in refinements of two ports (D186, D187): the all-firmware reference carries them, the
    // default does not, so a flip is a change against the current default.
    ("fx_compressor_row", |o, d| {
        o.fx.compressor_row = d.fx.compressor_row
    }),
    ("fx_mod1_unit_calls", |o, d| {
        o.fx.mod1_unit_calls = d.fx.mod1_unit_calls
    }),
];

/// Name and copy function of group index `g` (a [`GROUPS`] entry, then a [`SUBGROUPS`] entry).
fn group(g: usize) -> (&'static str, Revert) {
    if g < GROUP_COUNT {
        GROUPS[g]
    } else {
        SUBGROUPS[g - GROUP_COUNT]
    }
}

/// The groups of `FirmwareFx::PARITY` (every FX group but `fx_reverb`): delay, compressor, EQ,
/// Mod 1, Mod 2.
pub const PARITY_GROUPS: [usize; 5] = [2, 3, 5, 6, 7];

/// The firmware-derived options the ranking compares against the default (the oracle-verified
/// model): `--firmware-all` plus `--firmware-routing`.
pub fn firmware_options() -> FirmwareOptions {
    let mut o = FirmwareOptions {
        firmware_routing: true,
        ..FirmwareOptions::firmware_all()
    };
    // The firmware's own compressor words and the per-unit Mod 1 call length (D186, D187).
    o.fx.compressor_row = true;
    o.fx.mod1_unit_calls = true;
    o
}

/// The engine default the ranking compares against: `FirmwareOptions::default()` (the PARITY FX
/// ports since D173, rate match since D176), with `legacy_fx_rate` the default of D173 to D175
/// (rate match off), with `legacy_fx_ports` the default of D163 to D170 (amp and Mod 2), with
/// `legacy_mod2` the default before D163 (firmware Mod 2 off). The two port flags leave the rate
/// match on (it is no port); add `legacy_fx_rate` to reproduce the data of those eras. The committed ranking data
/// (`docs/research/data/program-ranking*.csv`) is the `legacy_mod2` baseline. Against the current
/// default every FX group of [`PARITY_GROUPS`] is a no-op flip.
pub fn baseline(legacy_mod2: bool, legacy_fx_ports: bool, legacy_fx_rate: bool) -> FirmwareOptions {
    let mut o = FirmwareOptions::default();
    if legacy_fx_ports {
        o.fx = nord_core::fx::FirmwareFx::PRE_PARITY;
    }
    if legacy_mod2 {
        o.fx = nord_core::fx::FirmwareFx::LEGACY;
    }
    if legacy_fx_rate {
        o.fx_rate_match = false;
    }
    o
}

/// `firmware` with the groups in `set` (indices into [`GROUPS`]) put back to `base`.
pub fn reverted_set(
    firmware: &FirmwareOptions,
    base: &FirmwareOptions,
    set: &[usize],
) -> FirmwareOptions {
    let mut o = *firmware;
    for &g in set {
        GROUPS[g].1(&mut o, base);
    }
    o
}

/// `base` with the groups in `set` switched to the firmware value (what a default flip of those
/// groups would render); the other groups keep `base`. `set` may name a [`SUBGROUPS`] index.
pub fn flipped_set(
    firmware: &FirmwareOptions,
    base: &FirmwareOptions,
    set: &[usize],
) -> FirmwareOptions {
    let mut o = *base;
    for &g in set {
        group(g).1(&mut o, firmware);
    }
    o
}

/// Parses `fx_mod1+fx_mod2,fx_eq` (sets separated by `,`, groups by `+`) into group indices;
/// `all-fx` is the six FX groups.
pub fn parse_sets(list: &str) -> Result<Vec<Vec<usize>>, String> {
    list.split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|item| {
            if item.trim() == "all-fx" {
                return Ok((FIRST_FX..GROUP_COUNT).collect());
            }
            item.split('+')
                .map(|n| {
                    (0..GROUP_COUNT + SUBGROUPS.len())
                        .find(|&g| group(g).0 == n.trim())
                        .ok_or_else(|| format!("unknown group '{n}'"))
                })
                .collect()
        })
        .collect()
}

/// `firmware` with one group put back to `base`.
pub fn reverted(
    firmware: &FirmwareOptions,
    base: &FirmwareOptions,
    group: usize,
) -> FirmwareOptions {
    reverted_set(firmware, base, &[group])
}

/// Key of a combination: the group names joined by `+` in [`GROUPS`] order.
pub fn subset_key(set: &[usize]) -> String {
    let mut s = set.to_vec();
    s.sort_unstable();
    s.iter().map(|&g| group(g).0).collect::<Vec<_>>().join("+")
}

/// Every non-empty combination of the FX groups (indices into [`GROUPS`]), by mask 1..2^6.
pub fn fx_subsets() -> Vec<Vec<usize>> {
    (1u32..(1 << FX_COUNT))
        .map(|mask| {
            (0..FX_COUNT)
                .filter(|b| mask & (1 << b) != 0)
                .map(|b| FIRST_FX + b)
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nord_core::fx::FirmwareFx;

    /// The default before D163: every group differs from the firmware options.
    fn pre() -> FirmwareOptions {
        baseline(true, false, false)
    }

    /// `o` with the two opt-in refinements (D186, D187) as the firmware options have them: they
    /// are [`SUBGROUPS`], not [`GROUPS`], so a group revert leaves them on.
    fn with_refinements(mut o: FirmwareOptions) -> FirmwareOptions {
        o.fx.compressor_row = true;
        o.fx.mod1_unit_calls = true;
        o
    }

    #[test]
    fn groups_cover_every_difference() {
        let (firmware, base) = (firmware_options(), pre());
        let all: Vec<usize> = (0..GROUP_COUNT).collect();
        assert_eq!(
            reverted_set(&firmware, &base, &all),
            with_refinements(base),
            "a group is missing"
        );
        assert_ne!(firmware, base);
        for g in 0..GROUP_COUNT {
            assert_ne!(
                reverted(&firmware, &base, g),
                firmware,
                "group {g} changes nothing"
            );
        }
    }

    #[test]
    fn mod2_group_is_a_no_op_against_the_current_default() {
        let (firmware, now) = (firmware_options(), baseline(false, true, false));
        assert_eq!(GROUPS[7].0, "fx_mod2");
        assert_eq!(
            reverted(&firmware, &now, 7),
            firmware,
            "Mod 2 is the default (D163)"
        );
        for g in (0..GROUP_COUNT).filter(|g| *g != 7) {
            assert_ne!(reverted(&firmware, &now, g), firmware, "group {g}");
        }
        assert!(baseline(false, true, false).fx.mod2 && !baseline(true, false, false).fx.mod2);
    }

    #[test]
    fn fx_groups_are_the_six_after_the_engine_groups() {
        assert!(GROUPS[..FIRST_FX]
            .iter()
            .all(|(n, _)| !n.starts_with("fx_")));
        assert!(GROUPS[FIRST_FX..].iter().all(|(n, _)| n.starts_with("fx_")));
        assert_eq!(GROUP_COUNT - FIRST_FX, FX_COUNT);
    }

    #[test]
    fn fx_subsets_are_all_63_distinct_combinations() {
        let s = fx_subsets();
        assert_eq!(s.len(), 63);
        let keys: std::collections::HashSet<String> = s.iter().map(|x| subset_key(x)).collect();
        assert_eq!(keys.len(), 63);
        assert_eq!(s.iter().filter(|x| x.len() == 1).count(), 6);
        assert_eq!(s.iter().filter(|x| x.len() == 2).count(), 15);
        assert_eq!(s.last().map(Vec::len), Some(6));
    }

    #[test]
    fn reverting_all_fx_groups_leaves_only_engine_groups_different() {
        let (firmware, base) = (firmware_options(), pre());
        let all_fx: Vec<usize> = (FIRST_FX..GROUP_COUNT).collect();
        let o = reverted_set(&firmware, &base, &all_fx);
        assert_eq!(o.fx, with_refinements(base).fx);
        assert_ne!(o.tone_source, base.tone_source);
    }

    #[test]
    fn flipping_every_group_is_the_firmware_and_none_is_the_base() {
        let (firmware, base) = (firmware_options(), pre());
        let all: Vec<usize> = (0..GROUP_COUNT).collect();
        assert_eq!(
            flipped_set(&firmware, &base, &all),
            {
                let mut f = firmware;
                f.fx.compressor_row = false;
                f.fx.mod1_unit_calls = false;
                f
            },
            "the refinements are subgroups, not part of the group flip"
        );
        let none = flipped_set(&firmware, &base, &[]);
        assert_eq!(none.fx, base.fx);
        assert_eq!(none.tone_source, base.tone_source);
    }

    #[test]
    fn parity_groups_are_the_fx_ports_of_the_parity_preset() {
        let o = flipped_set(&firmware_options(), &pre(), &PARITY_GROUPS);
        // `compressor_row` is a subgroup (D187), not a group: the flip from `pre()` leaves it off
        assert_eq!(
            o.fx,
            FirmwareFx {
                compressor_row: false,
                ..FirmwareFx::PARITY
            }
        );
        assert_eq!(GROUPS[4].0, "fx_reverb", "the one FX group left out");
        assert!(!PARITY_GROUPS.contains(&4));
    }

    #[test]
    fn flipping_one_group_changes_only_that_group() {
        let (firmware, def) = (firmware_options(), pre());
        let o = flipped_set(&firmware, &def, &[7]);
        assert_eq!(o.fx.mod2, firmware.fx.mod2);
        assert_ne!(o.fx.mod2, def.fx.mod2);
        assert_eq!(o.fx.eq, def.fx.eq);
        assert_eq!(o.fx.mod1, def.fx.mod1);
    }

    #[test]
    fn parse_sets_reads_names_and_all_fx() {
        assert_eq!(
            parse_sets("fx_mod2+fx_eq,fx_delay").unwrap(),
            [vec![7, 5], vec![2]]
        );
        assert_eq!(parse_sets("all-fx").unwrap()[0].len(), 6);
        assert!(parse_sets("fx_bogus").is_err());
        assert!(parse_sets("").unwrap().is_empty());
    }

    #[test]
    fn delay_subgroups_flip_one_port_alone() {
        let (firmware, base) = (firmware_options(), baseline(false, true, false));
        let time = flipped_set(&firmware, &base, &[GROUP_COUNT]);
        assert!(time.fx.delay_time && !time.fx.delay_laws);
        let laws = flipped_set(&firmware, &base, &[GROUP_COUNT + 1]);
        assert!(laws.fx.delay_laws && !laws.fx.delay_time);
        assert_eq!(
            parse_sets("fx_delay_time+fx_delay_laws").unwrap(),
            [vec![GROUP_COUNT, GROUP_COUNT + 1]]
        );
        assert_eq!(subset_key(&[GROUP_COUNT + 1, 7]), "fx_mod2+fx_delay_laws");
        let both = flipped_set(&firmware, &base, &[2]);
        assert!(both.fx.delay_time && both.fx.delay_laws);
    }

    #[test]
    fn baseline_has_the_rate_match_on_unless_a_legacy_flag_turns_it_off() {
        assert!(baseline(false, false, false).fx_rate_match);
        assert!(!baseline(false, false, true).fx_rate_match);
        assert!(baseline(false, true, false).fx_rate_match);
        assert!(baseline(true, false, false).fx_rate_match);
        assert!(!baseline(true, false, true).fx_rate_match);
        assert_eq!(
            baseline(false, false, true).fx,
            nord_core::fx::FirmwareFx::PARITY
        );
        assert!(firmware_options().fx_rate_match);
    }

    #[test]
    fn compressor_row_and_unit_call_subgroups_flip_one_refinement_alone() {
        let (firmware, base) = (firmware_options(), baseline(false, true, false));
        assert!(firmware.fx.compressor_row && firmware.fx.mod1_unit_calls);
        assert!(
            baseline(false, false, false).fx.compressor_row,
            "D187 default"
        );
        assert!(
            !base.fx.compressor_row && !base.fx.mod1_unit_calls,
            "off before D173"
        );
        let row = flipped_set(&firmware, &base, &[GROUP_COUNT + 3]);
        assert!(row.fx.compressor_row && !row.fx.mod1_unit_calls);
        let calls = flipped_set(&firmware, &base, &[GROUP_COUNT + 4]);
        assert!(calls.fx.mod1_unit_calls && !calls.fx.compressor_row);
        assert_eq!(
            parse_sets("fx_compressor_row+fx_mod1_unit_calls").unwrap(),
            [vec![GROUP_COUNT + 3, GROUP_COUNT + 4]]
        );
    }

    #[test]
    fn rate_match_subgroup_switches_the_converter_on_alone() {
        let (firmware, base) = (firmware_options(), baseline(false, true, true));
        assert!(firmware.fx_rate_match && !base.fx_rate_match);
        let on = flipped_set(&firmware, &base, &[GROUP_COUNT + 2]);
        assert!(on.fx_rate_match);
        assert_eq!(
            FirmwareOptions {
                fx_rate_match: false,
                ..on
            },
            base
        );
        let sets = parse_sets("fx_delay+fx_rate_match").unwrap();
        assert_eq!(sets, [vec![2, GROUP_COUNT + 2]]);
        assert_eq!(subset_key(&sets[0]), "fx_delay+fx_rate_match");
    }

    #[test]
    fn subset_key_sorts_and_joins_names() {
        assert_eq!(subset_key(&[7, 2]), "fx_delay+fx_mod2");
        assert_eq!(subset_key(&[4]), "fx_reverb");
    }
}
