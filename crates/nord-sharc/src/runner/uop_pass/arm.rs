//! Arming of a booted core for the uop pass loops: which code to translate ([`code_ranges`]) and
//! the stop pc ([`arm_uop`], [`arm_uop_ranges`]). The engine names and the configuration an
//! engine runs with are in `engines.rs`.
//!
//! OWNER: W1-RUN. The engine translates what it is told to: [`code_ranges`] finds the code of a
//! booted core (the non-zero parcels of the VISA window, split at wide zero gaps so that a zeroed
//! data buffer is not marked as code and its stores do not count as `smc`), or the caller passes
//! the ranges (`probe::probe_ranges`). [`arm_uop_ranges`] makes the pass idle pc a block start
//! (design P7) and sets it as the stop pc for every later retranslation.
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

/// `ranges` with the one that holds `pc` in its interior cut in two at `pc`: the translator
/// starts a block at the first pc of every range, so `pc` is a block start whatever the stop pc
/// says. A range that starts or ends at `pc`, or does not hold it, is kept whole, and so is a
/// range outside [`CODE_WINDOW`]: `enable_uop` rejects it and names it in the error, not a half.
fn split_at(ranges: &[(u32, u32)], pc: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::with_capacity(ranges.len() + 1);
    for &(lo, hi) in ranges {
        let in_window = CODE_WINDOW.contains(&lo) && hi <= CODE_WINDOW.end;
        if in_window && lo < pc && pc < hi {
            out.extend([(lo, pc), (pc, hi)]);
        } else {
            out.push((lo, hi));
        }
    }
    out
}

/// Arms `core` for the pass loop: `idle_pc` becomes a block start (design P7) and the stop pc, and
/// the short word `ranges` are translated with `cfg`. Call it once on a booted core, before the
/// first pass; the pass loops keep the stop pc current afterwards.
///
/// Two things make `idle_pc` a block start. The stop pc is set BEFORE `enable_uop`, which keeps
/// it while it translates (`core/uop/drive/mod.rs`: it saves it across `disable_uop` and restores
/// it before the first translation) and which the retranslation after a code write
/// (`uop_maintain`: a program change or an overlay load) reads again. The range that holds
/// `idle_pc` is also cut there (`split_at`): the translator starts a block at the first pc of a
/// range, so the first build does not depend on the stop pc being honoured (belt and braces; the
/// stop pc alone gives the same blocks, `arming_cuts_the_first_translation_...`). Tests: `arming_cuts_the_first_translation_at_the_idle_pc_which_a_bare_enable_does_not` (the
/// first build) and `the_stop_pc_set_at_arming_cuts_the_retranslation_too` (deleting the
/// `uop_set_stop_pc` call fails it).
pub fn arm_uop_ranges(
    core: &mut Core,
    cfg: UopConfig,
    idle_pc: u32,
    ranges: &[(u32, u32)],
) -> Result<UopBuildReport, UopError> {
    core.uop_set_stop_pc(Some(idle_pc));
    core.enable_uop(cfg, &split_at(ranges, idle_pc))
}

/// [`arm_uop_ranges`] over the [`code_ranges`] of the core.
pub fn arm_uop(core: &mut Core, cfg: UopConfig, idle_pc: u32) -> Result<UopBuildReport, UopError> {
    let ranges = code_ranges(core);
    arm_uop_ranges(core, cfg, idle_pc, &ranges)
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

    /// A code write that changes the first NOP and the jump target of the poll program: the hash
    /// of every block of it changes (an identical rewrite is kept by revalidation, P6, and
    /// retranslates nothing), so `uop_maintain` drops and retranslates the whole program; returns
    /// the blocks that retranslation added.
    fn retranslated(c: &mut Core) -> usize {
        put(c, CODE, &[0xc8be]);
        put(c, POLL_PC + 1, &[0x0013]);
        assert!(c.uop_needs_maintenance(), "the writes made the page dirty");
        c.uop_maintain()
    }

    #[test]
    fn a_range_is_cut_at_the_idle_pc_only_when_the_pc_is_inside_it() {
        let (lo, hi) = (CODE, CODE + 12);
        let pc = CODE + 5;
        assert_eq!(split_at(&[(lo, hi)], pc), vec![(lo, pc), (pc, hi)]);
        // at an end, or outside: whole
        for edge in [lo, hi, CODE - 1, hi + 7] {
            assert_eq!(split_at(&[(lo, hi)], edge), vec![(lo, hi)], "pc {edge:#x}");
        }
        // of several ranges only the one that holds it
        let other = (CODE + 0x100, CODE + 0x110);
        assert_eq!(
            split_at(&[(lo, hi), other], pc),
            vec![(lo, pc), (pc, hi), other]
        );
        assert_eq!(split_at(&[], pc), Vec::new());
        // outside the code window the engine's error names the range as given
        let bad = (0, CODE + 12);
        assert_eq!(split_at(&[bad], CODE + 5), vec![bad]);
        let past = (CODE, CODE_WINDOW.end + 4);
        assert_eq!(split_at(&[past], CODE + 5), vec![past]);
    }

    #[test]
    fn arming_cuts_the_first_translation_at_the_idle_pc_which_a_bare_enable_does_not() {
        // ten NOPs and the poll jump at POLL_PC: a bare enable (no stop pc) builds the poll
        // program as one block per PEy mode, with the idle pc inside it. Arming makes the idle pc
        // a block start (P7): it sets the stop pc, which `enable_uop` keeps while it translates
        // (`core/uop/drive/mod.rs`), and cuts the range there as well.
        let (mut armed, mut bare) = (poll_core(), poll_core());
        let ranges = code_ranges(&bare);
        let built = arm_uop(&mut armed, UopConfig::default(), POLL_PC).unwrap();
        let plain = bare.enable_uop(UopConfig::default(), &ranges).unwrap();
        assert!(plain.blocks > 0, "the translator is real: {plain:?}");
        assert!(
            built.blocks > plain.blocks,
            "armed built {built:?}, bare {plain:?}: the idle pc is a block start"
        );
        assert_eq!(built.insns, plain.insns, "the same code is covered");
        // the stop pc alone does the same: `enable_uop` restores it before it translates, the
        // cut of the range is a second way to the same blocks
        let mut stop_only = poll_core();
        stop_only.uop_set_stop_pc(Some(POLL_PC));
        assert_eq!(
            stop_only.enable_uop(UopConfig::default(), &ranges),
            Ok(built)
        );
        assert_eq!(armed.uop_stats().translations, u64::from(built.blocks));
        // an idle pc at the start of the code, or outside it, cuts nothing
        for idle in [CODE, CODE - 1, CODE + 0x1000] {
            let mut c = poll_core();
            let r = arm_uop(&mut c, UopConfig::default(), idle).unwrap();
            assert_eq!(r, plain, "idle pc {idle:#x}");
        }
    }

    #[test]
    fn the_stop_pc_set_at_arming_cuts_the_retranslation_too() {
        // the retranslation after a code write (`uop_maintain`: a program change or an overlay
        // load) reads the stop pc again, with the dropped ranges merged back into one: only a
        // core armed with the stop pc cuts the poll program at the idle pc there. Without
        // `core.uop_set_stop_pc(Some(idle_pc))` in `arm_uop_ranges` the armed core retranslates
        // as many blocks as the bare one and this test fails. The write changes the code of both
        // blocks of the armed core (an identical rewrite is kept by revalidation).
        let (mut armed, mut bare) = (poll_core(), poll_core());
        let ranges = code_ranges(&bare);
        let built = arm_uop(&mut armed, UopConfig::default(), POLL_PC).unwrap();
        let plain = bare.enable_uop(UopConfig::default(), &ranges).unwrap();
        let (with_stop, without) = (retranslated(&mut armed), retranslated(&mut bare));
        assert_eq!(
            without as u32, plain.blocks,
            "the bare engine retranslates exactly the blocks it had: one per PEy mode"
        );
        assert_eq!(
            with_stop as u32, built.blocks,
            "armed retranslates the blocks it was built with, the idle pc cut included"
        );
        assert!(with_stop > without, "{with_stop} against {without}");
    }
}
