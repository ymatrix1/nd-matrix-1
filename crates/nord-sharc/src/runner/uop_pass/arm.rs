//! Arming of a booted core for the uop pass loops: which code to translate ([`code_ranges`]), the
//! stop pc ([`arm_uop`]) and the engine names and configurations the runners and tools share
//! ([`engine_named`], [`uop_config`], [`arm_for`]).
//!
//! OWNER: W1-RUN. The engine translates what it is told to: [`code_ranges`] finds the code of a
//! booted core (the non-zero parcels of the VISA window, split at wide zero gaps so that a zeroed
//! data buffer is not marked as code and its stores do not count as `smc`), or the caller passes
//! the ranges (`probe::probe_ranges`). [`arm_uop_ranges`] sets the pass idle pc as the stop pc
//! (design P7: it must be a block start) before `enable_uop`.
use super::super::pass_sched::Engine;
use super::probe::pc_ranges;
use crate::core::{Core, UopBuildReport, UopConfig, UopError};
use std::ops::Range;

/// Short-word window the engine accepts code from: the SRAM short-word alias of `core::memory`
/// (`SHORT_WORDS`; VISA code lives at `0x12_0000..` in the firmware images). VERIFIED against
/// `Core::enable_uop`, which rejects a range outside it: test
/// `the_code_window_is_the_one_enable_uop_accepts`.
pub const CODE_WINDOW: Range<u32> = 0x10_0000..0x20_0000;
/// A run of this many zero parcels or more ends a [`code_ranges`] range. ASSUMED 256 = one code
/// page (`PAGE_SHIFT` 8): a zeroed delay line or buffer is then at least a page wide and stays out
/// of the translated pages. Confirm on firmware with the `pages` of `UopBuildReport` and the G2
/// `smc` counter (a data store on a code page).
pub const RANGE_GAP: u32 = 256;
/// Parcels added after the last non-zero parcel of a range, so the instruction that starts there
/// is read whole. VERIFIED: no VISA instruction is longer than 3 parcels = `visa::classify` (the
/// length the interpreter fetch of `core/exec.rs` uses; ISA-mode words are a fixed 3), checked by
/// `no_instruction_is_longer_than_three_parcels` over all 65,536 first parcels and the three
/// follow-up cases (none, a parcel without the 32-bit marker, a parcel with it): the longest is 3,
/// so the tail is 3 - 1 = 2, max err 0. `a_three_parcel_instruction_with_zero_tail_parcels_is_covered`
/// shows a 3-parcel instruction whose last two parcels are zero is inside the range.
pub const RANGE_TAIL: u32 = 2;

/// The short-word ranges of the non-zero code in the VISA window of `core`, in address order,
/// ready for `Core::enable_uop` ([`pc_ranges`] over the non-zero parcels): each range runs from
/// its first non-zero parcel to [`RANGE_TAIL`] parcels past its last one (clamped to the window);
/// zero runs of [`RANGE_GAP`] parcels or more separate ranges. Empty when the window holds no
/// code. Code whose parcels are all zero is not found: the scan is for booted firmware images.
/// Non-zero data counts as code here, which is what `probe::probe_ranges` avoids.
pub fn code_ranges(core: &Core) -> Vec<(u32, u32)> {
    pc_ranges(CODE_WINDOW.filter(|&pc| core.parcel_at(pc) != 0))
}

/// Arms `core` for the pass loop: `idle_pc` becomes the stop pc (a block start, P7) and the short
/// word `ranges` are translated with `cfg`. Call it once on a booted core, before the first pass;
/// the pass loops keep the stop pc current afterwards. The stop pc is set BEFORE `enable_uop`
/// because `enable_uop` keeps the value it finds and the translator reads it
/// (`lower/translate.rs`): without it the pass idle pc is an interior pc of a block, and a pass
/// could run past it (test `arming_makes_the_stop_pc_a_block_start_which_a_bare_enable_does_not`
/// shows the retranslation after a code write cutting at it only when it was set here).
pub fn arm_uop_ranges(
    core: &mut Core,
    cfg: UopConfig,
    idle_pc: u32,
    ranges: &[(u32, u32)],
) -> Result<UopBuildReport, UopError> {
    core.uop_set_stop_pc(Some(idle_pc));
    core.enable_uop(cfg, ranges)
}

/// [`arm_uop_ranges`] over the [`code_ranges`] of the core.
pub fn arm_uop(core: &mut Core, cfg: UopConfig, idle_pc: u32) -> Result<UopBuildReport, UopError> {
    let ranges = code_ranges(core);
    arm_uop_ranges(core, cfg, idle_pc, &ranges)
}

/// The `UopConfig` an engine runs with, `None` for the engines that do not use the uop path.
/// `families` is the `native::FAM_*` bit set of the inline families (0 = call-threaded only) and
/// only matters for [`Engine::UopNative`].
pub fn uop_config(engine: Engine, families: u32) -> Option<UopConfig> {
    match engine {
        Engine::Interp | Engine::Aot => None,
        Engine::Uop | Engine::UopChecked => Some(UopConfig::default()),
        Engine::UopNative => Some(UopConfig {
            native: true,
            families,
            ..UopConfig::default()
        }),
    }
}

/// [`arm_uop`] for the engine a runner was asked for: `Ok(None)` when it needs no arming. With
/// `ranges` the engine translates those short-word ranges instead of the [`code_ranges`] of the
/// core. The error is a message ready to print; a native request on a build without the feature
/// says how to enable it instead of silently measuring Tier 1.
pub fn arm_for(
    core: &mut Core,
    engine: Engine,
    families: u32,
    idle_pc: u32,
    ranges: Option<&[(u32, u32)]>,
) -> Result<Option<UopBuildReport>, String> {
    let Some(cfg) = uop_config(engine, families) else {
        return Ok(None);
    };
    let armed = match ranges {
        Some(r) => arm_uop_ranges(core, cfg, idle_pc, r),
        None => arm_uop(core, cfg, idle_pc),
    };
    match armed {
        Ok(report) => Ok(Some(report)),
        Err(UopError::NativeUnavailable) => {
            Err("the uop-native engine needs a build with feature \
             nord-sharc/uop-native (cargo run ... --features nord-sharc/uop-native)"
                .into())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// The engine a command-line name selects: `interp`, `aot`, `uop`, `uop-checked`, `uop-native`.
pub fn engine_named(name: &str) -> Option<Engine> {
    Some(match name {
        "interp" => Engine::Interp,
        "aot" => Engine::Aot,
        "uop" => Engine::Uop,
        "uop-checked" => Engine::UopChecked,
        "uop-native" => Engine::UopNative,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::support::{poll_core, CODE, POLL_PC};
    use super::*;
    use crate::core::support::core_with;

    /// Writes parcels through the short-word alias window, like a program load.
    fn put(c: &mut Core, at: u32, parcels: &[u16]) {
        for (k, p) in parcels.iter().enumerate() {
            c.dm_write(at + k as u32, u32::from(*p)).unwrap();
        }
    }

    #[test]
    fn code_ranges_of_a_core_without_code_is_empty() {
        assert_eq!(code_ranges(&Core::new(&[])), Vec::new());
    }

    #[test]
    fn code_ranges_cover_the_non_zero_parcels_plus_the_tail() {
        let mut c = Core::new(&[]);
        put(&mut c, CODE + 5, &[0x0001, 0x0001, 0x0001]);
        assert_eq!(code_ranges(&c), vec![(CODE + 5, CODE + 8 + RANGE_TAIL)]);
        // a close island joins the range: the zero parcels between are inside the extent
        put(&mut c, CODE + 5 + 3 + RANGE_GAP - 1, &[0x0001]);
        assert_eq!(
            code_ranges(&c),
            vec![(CODE + 5, CODE + 5 + 3 + RANGE_GAP + RANGE_TAIL)]
        );
    }

    #[test]
    fn a_zero_gap_of_the_gap_size_splits_the_extent_in_two() {
        let mut c = Core::new(&[]);
        put(&mut c, CODE, &[0x0001]);
        put(&mut c, CODE + 1 + RANGE_GAP, &[0x0001]); // exactly RANGE_GAP zero parcels between
        put(&mut c, CODE + 0x4_0000, &[0x0001, 0x0001]);
        let r = code_ranges(&c);
        assert_eq!(
            r,
            vec![
                (CODE, CODE + 1 + RANGE_TAIL),
                (CODE + 1 + RANGE_GAP, CODE + 2 + RANGE_GAP + RANGE_TAIL),
                (CODE + 0x4_0000, CODE + 0x4_0002 + RANGE_TAIL),
            ]
        );
        assert!(
            r.windows(2).all(|w| w[0].1 <= w[1].0),
            "sorted, disjoint: {r:?}"
        );
        // one parcel fewer between them and they are one range
        let mut c = Core::new(&[]);
        put(&mut c, CODE, &[0x0001]);
        put(&mut c, CODE + RANGE_GAP, &[0x0001]);
        assert_eq!(
            code_ranges(&c),
            vec![(CODE, CODE + RANGE_GAP + 1 + RANGE_TAIL)]
        );
    }

    #[test]
    fn ranges_are_clamped_to_the_window_and_the_window_edges_are_scanned() {
        let mut c = Core::new(&[]);
        put(&mut c, CODE_WINDOW.start, &[0x0001]);
        put(&mut c, CODE_WINDOW.end - 1, &[0x0001]);
        assert_eq!(
            code_ranges(&c),
            vec![
                (CODE_WINDOW.start, CODE_WINDOW.start + 1 + RANGE_TAIL),
                (CODE_WINDOW.end - 1, CODE_WINDOW.end),
            ]
        );
    }

    #[test]
    fn a_three_parcel_instruction_with_zero_tail_parcels_is_covered() {
        // IF EQ JUMP 0x0: 0600 0000 0000, the longest instruction with two zero parcels
        let (_, len) = crate::visa::classify(0x0600, Some(0));
        assert_eq!(len as u32 - 1, RANGE_TAIL);
        let mut c = Core::new(&[]);
        put(&mut c, CODE, &[0x0600]);
        assert_eq!(code_ranges(&c), vec![(CODE, CODE + 3)]);
    }

    #[test]
    fn no_instruction_is_longer_than_three_parcels() {
        // the whole first-parcel space; the second parcel matters to `classify` only through the
        // 32-bit marker (low 7 bits 0x3F), so none, an unmarked and a marked one cover it
        let mut longest = 0;
        for p0 in 0..=u16::MAX {
            for p1 in [None, Some(0x0000), Some(0x003F)] {
                longest = longest.max(crate::visa::classify(p0, p1).1);
            }
        }
        assert_eq!(longest, 3, "the longest instruction");
        assert_eq!(longest as u32 - 1, RANGE_TAIL, "the tail is that minus one");
    }

    #[test]
    fn the_code_window_is_the_one_enable_uop_accepts() {
        let mut c = Core::new(&[]);
        let ok = |c: &mut Core, lo, hi| c.enable_uop(UopConfig::default(), &[(lo, hi)]).is_ok();
        assert!(ok(&mut c, CODE_WINDOW.start, CODE_WINDOW.start + 1));
        assert!(ok(&mut c, CODE_WINDOW.end - 1, CODE_WINDOW.end));
        assert!(!ok(&mut c, CODE_WINDOW.start - 1, CODE_WINDOW.start + 1));
        assert!(!ok(&mut c, CODE_WINDOW.end - 1, CODE_WINDOW.end + 1));
        // and every range the scan can return is acceptable to it
        let mut c = Core::new(&[]);
        put(&mut c, CODE_WINDOW.start, &[0x0001]);
        put(&mut c, CODE_WINDOW.end - 1, &[0x0001]);
        assert!(c.enable_uop(UopConfig::default(), &code_ranges(&c)).is_ok());
    }

    #[test]
    fn arm_uop_translates_the_code_of_the_core() {
        let mut c = core_with(&[0x0001; 10]);
        let report = arm_uop(&mut c, UopConfig::default(), CODE + 5).unwrap();
        assert!(c.uop_enabled());
        assert_eq!(c.uop_stats().translations, u64::from(report.blocks));
        // no code at all: armed and empty, not an error
        let mut empty = Core::new(&[]);
        assert_eq!(
            arm_uop(&mut empty, UopConfig::default(), CODE + 5),
            Ok(UopBuildReport::default())
        );
        assert!(empty.uop_enabled());
    }

    #[test]
    fn engines_map_to_their_configuration_and_names() {
        assert_eq!(uop_config(Engine::Interp, 0), None);
        assert_eq!(uop_config(Engine::Aot, 0), None);
        assert_eq!(uop_config(Engine::Uop, 7), Some(UopConfig::default()));
        assert_eq!(
            uop_config(Engine::UopChecked, 0),
            Some(UopConfig::default())
        );
        let native = uop_config(Engine::UopNative, 5).unwrap();
        assert_eq!((native.native, native.families), (true, 5));
        for (name, engine) in [
            ("interp", Engine::Interp),
            ("aot", Engine::Aot),
            ("uop", Engine::Uop),
            ("uop-checked", Engine::UopChecked),
            ("uop-native", Engine::UopNative),
        ] {
            assert_eq!(engine_named(name), Some(engine));
        }
        assert_eq!(engine_named("Uop"), None);
        assert_eq!(engine_named(""), None);
    }

    #[test]
    fn arm_for_arms_the_uop_engines_only_and_explains_a_missing_native_tier() {
        let mut c = core_with(&[0x0001; 10]);
        assert_eq!(arm_for(&mut c, Engine::Interp, 0, CODE + 5, None), Ok(None));
        assert_eq!(arm_for(&mut c, Engine::Aot, 0, CODE + 5, None), Ok(None));
        assert!(!c.uop_enabled());
        let uop = arm_for(&mut c, Engine::Uop, 0, CODE + 5, None);
        assert!(uop.unwrap().is_some());
        assert!(c.uop_enabled());
        let mut n = core_with(&[0x0001; 10]);
        let native = arm_for(&mut n, Engine::UopNative, 0, CODE + 5, None);
        if cfg!(feature = "uop-native") {
            assert!(native.is_ok());
        } else {
            let e = native.unwrap_err();
            assert!(e.contains("uop-native") && e.contains("--features"), "{e}");
            assert!(!n.uop_enabled(), "a failed arming leaves the engine off");
        }
    }

    #[test]
    fn a_range_list_arms_the_engine_instead_of_the_extent_scan() {
        // ten NOPs, but only the first four are asked for
        let mut c = core_with(&[0x0001; 10]);
        let four = [(CODE, CODE + 4)];
        let narrow = arm_for(&mut c, Engine::Uop, 0, CODE + 4, Some(&four)).unwrap();
        let mut all = core_with(&[0x0001; 10]);
        let wide = arm_for(&mut all, Engine::Uop, 0, CODE + 4, None).unwrap();
        let (narrow, wide) = (narrow.unwrap(), wide.unwrap());
        assert!(c.uop_enabled() && all.uop_enabled());
        // the translator is real: four of the ten NOPs are asked for
        assert!(wide.insns > 0, "{wide:?}");
        assert!(narrow.insns < wide.insns, "{narrow:?} against {wide:?}");
        // a range outside the window is the engine's error, as text, and leaves the engine off
        let mut bad = core_with(&[0x0001; 10]);
        let e = arm_for(&mut bad, Engine::Uop, 0, CODE, Some(&[(0, 4)])).unwrap_err();
        assert!(e.contains("outside the code window"), "{e}");
        assert!(!bad.uop_enabled());
    }

    /// A code write that leaves the code as it is, so the page is dirty and `uop_maintain` drops
    /// and retranslates its blocks; returns the blocks that retranslation added.
    fn retranslated(c: &mut Core) -> usize {
        let word = c.dm_read(CODE / 2).unwrap();
        c.dm_write(CODE / 2, word).unwrap();
        assert!(c.uop_needs_maintenance(), "the write made the page dirty");
        c.uop_maintain()
    }

    #[test]
    fn arming_makes_the_stop_pc_a_block_start_which_a_bare_enable_does_not() {
        // ten NOPs and the poll jump at POLL_PC. The translator reads `uop_run.stop_pc`
        // (`lower/translate.rs`): with the pass idle pc known it cuts the NOP run in front of the
        // poll instruction, so the poll core has more blocks than the same core enabled without
        // it. `enable_uop` keeps the stop pc set before it (it saves and restores it), and the
        // retranslation after a code write (`uop_maintain`, a program change or overlay load)
        // reads it again: the retranslated poll program has the extra blocks only when
        // `arm_uop_ranges` set the stop pc.
        let (mut armed, mut bare) = (poll_core(), poll_core());
        let ranges = code_ranges(&bare);
        let built = arm_uop(&mut armed, UopConfig::default(), POLL_PC).unwrap();
        let plain = bare.enable_uop(UopConfig::default(), &ranges).unwrap();
        assert!(plain.blocks > 0, "the translator is real: {plain:?}");
        assert_eq!(built.insns, plain.insns, "the same code is covered");
        // not asserted strictly on the first build: `Core::enable_uop` clears the stop pc with
        // `disable_uop` before it translates and restores it afterwards (drive.rs), so the first
        // translation does not see it yet; `uop_begin_pass` repairs that (P7, W2-DRIVE)
        assert!(built.blocks >= plain.blocks, "{built:?} against {plain:?}");
        let (with_stop, without) = (retranslated(&mut armed), retranslated(&mut bare));
        assert_eq!(
            without as u32, plain.blocks,
            "the bare engine retranslates exactly the blocks it had: one per PEy mode"
        );
        assert!(
            with_stop > without,
            "armed retranslated {with_stop} blocks, bare {without}: the stop pc is a block start"
        );
    }
}
