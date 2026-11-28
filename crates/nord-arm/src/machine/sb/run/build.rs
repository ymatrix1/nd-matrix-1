//! OWNER: W2-DRV. Building, admitting and inserting traces, and the flipped re-layout (design
//! note 4.1).
//!
//! A built trace is admitted (no page of it volatile; translation validation when
//! `cfg.validate_cases > 0`), every page it read is marked as code (`Mem::mark_code`, so a store
//! into one queues the page and the executor's store uops end the trace), the cache is flushed
//! when over its cap, the trace is compiled when a backend is configured, and it is inserted. A
//! pc whose build or admission failed is marked failed (`HotTable::mark_failed`) until a stale
//! page there clears the mark.
//!
//! The re-layout: after `RELAYOUT_AFTER` entries of a trace, every guard exit taken more than
//! half of those times gives the address of its branch (`Exit::last_pc`), and the trace is rebuilt
//! once with the prediction of those branches flipped (`trace::build_flipped`). The replaced trace
//! is dropped with `SbState::drop_trace` (links reset, backend code released, hotness reset).

use super::super::stale::is_active;
use super::super::trace::{build, build_flipped, BuildOpts};
use super::super::uop::{k, Sb, SbId};
use super::super::validate::validate_trace;
use super::super::SbState;
use crate::machine::Machine;

/// Build a trace at `pc`, admit and insert it; false (and `pc` marked failed) when none results.
pub(super) fn build_at(m: &mut Machine, sb: &mut SbState, pc: u32) -> bool {
    let opts = BuildOpts::from(&sb.cfg);
    match build(&mut m.bus.mem, pc, &opts) {
        Some(t) if admit(m, sb, &t) => {
            insert(m, sb, t);
            true
        }
        built => {
            if built.is_none() {
                sb.stats.build_failed += 1;
            }
            sb.hot.mark_failed(pc);
            false
        }
    }
}

/// Whether `t` may enter the cache: none of its pages is volatile (code that keeps changing
/// stays in tier 1) and it passes translation validation when that is on.
fn admit(m: &Machine, sb: &mut SbState, t: &Sb) -> bool {
    if t.pages[..t.n_pages as usize]
        .iter()
        .any(|p| sb.cache.volatile(*p))
    {
        return false;
    }
    let cases = sb.cfg.validate_cases;
    if cases == 0 {
        return true;
    }
    sb.stats.validations += 1;
    let seed = u64::from(t.entry) ^ sb.stats.built << 32;
    if validate_trace(m, t, cases, seed).is_err() {
        sb.stats.shadow_mismatches += 1;
        return false;
    }
    true
}

/// Mark the pages of `t` as code, cap the cache, compile and insert.
fn insert(m: &mut Machine, sb: &mut SbState, mut t: Sb) {
    if !is_active(604) {
        for page in &t.pages[..t.n_pages as usize] {
            m.bus.mem.mark_code(page << 12);
        }
    }
    sb.flush_if_over_cap();
    if sb.cfg.native {
        if let Some(be) = sb.backend.as_mut() {
            if let Some(native) = be.compile(&t) {
                t.native = native;
            }
        }
    }
    sb.cache.insert(t);
    sb.stats.built += 1;
}

/// The branch addresses of the guard exits taken more than half of `enters` times.
fn hot_guards(t: &Sb) -> Vec<u32> {
    let guard = |kind| {
        matches!(
            kind,
            k::GUARD | k::CMP_GUARD_I | k::CMP_GUARD_R | k::TST_GUARD_I
        )
    };
    let mut flips: Vec<u32> = t
        .uops
        .iter()
        .filter(|u| guard(u.kind))
        .map(|u| &t.exits[u.exit_idx()])
        .filter(|e| u64::from(e.hits) * 2 > u64::from(t.enters))
        .map(|e| e.last_pc)
        .collect();
    flips.sort_unstable();
    flips.dedup();
    flips
}

/// Rebuild trace `id` with the prediction of its hot guards flipped and replace it. False when
/// no guard is hot or the rebuild is not admitted (the trace stays as it is).
pub(super) fn relayout(m: &mut Machine, sb: &mut SbState, id: SbId) -> bool {
    let flips = hot_guards(sb.cache.get(id));
    if flips.is_empty() {
        return false;
    }
    let entry = sb.cache.get(id).entry;
    let opts = BuildOpts::from(&sb.cfg);
    let Some(t) = build_flipped(&mut m.bus.mem, entry, &opts, &flips) else {
        return false;
    };
    if !admit(m, sb, &t) {
        return false;
    }
    sb.drop_trace(id);
    insert(m, sb, t);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::cache::tiny_trace;
    use crate::machine::sb::uop::{Exit, ExitKind, Uop};
    use crate::machine::sb::SbConfig;

    fn state() -> SbState {
        SbState {
            cfg: SbConfig::on(),
            ..SbState::default()
        }
    }

    #[test]
    fn a_build_marks_the_pages_as_code_and_a_failed_one_marks_the_pc() {
        let (mut m, mut sb) = (Machine::new(), state());
        m.bus.mem.write32(0x1000, 0xE280_0001); // add r0,r0,#1
        m.bus.mem.write32(0x1004, 0xE12F_FF1E); // bx lr
        assert!(build_at(&mut m, &mut sb, 0x1000));
        assert_eq!((sb.stats.built, sb.cache.live_count()), (1, 1));
        m.bus.mem.write32(0x1000, 0xE280_0002);
        assert_eq!(
            m.bus.mem.stale_code,
            [1],
            "the page of the trace was marked"
        );
        // unmapped code: nothing to build, the pc is marked and nothing is mapped
        assert!(!build_at(&mut m, &mut sb, 0x7000_0000));
        assert!(sb.hot.failed(0x7000_0000) && !m.bus.mem.is_mapped(0x7000_0000));
        assert_eq!(sb.stats.build_failed, 1);
    }

    #[test]
    fn a_trace_on_a_volatile_page_is_not_admitted_and_validation_is_fail_closed() {
        let (m, mut sb) = (Machine::new(), state());
        let t = tiny_trace(0x5000, &[(0x5000, 1)]);
        assert!(admit(&m, &mut sb, &t));
        sb.cache.mark_volatile(5);
        assert!(!admit(&m, &mut sb, &t));
        // the skeleton validator rejects everything: a trace is not admitted unvalidated
        let mut sb = state();
        sb.cfg.validate_cases = 2;
        assert!(!admit(&m, &mut sb, &t));
        assert_eq!((sb.stats.validations, sb.stats.shadow_mismatches), (1, 1));
    }

    #[test]
    fn only_guard_exits_taken_more_than_half_of_the_entries_are_hot() {
        let mut first = Uop::new(k::GUARD, 0x1004);
        first.rs = 0;
        let mut second = Uop::new(k::CMP_GUARD_I, 0x1008);
        second.rs = 1;
        let uops = vec![first, second, Uop::exit(2, 0x100C)];
        let exits = vec![
            Exit::new(ExitKind::Direct, 2, 0x1004, 0x2000),
            Exit::new(ExitKind::Direct, 3, 0x1008, 0x3000),
            Exit::new(ExitKind::Direct, 4, 0x100C, 0x4000),
        ];
        let mut t = Sb::from_parts(0x1000, 4, uops, exits, vec![(0x1000, 0)]).unwrap();
        t.enters = 64;
        (t.exits[0].hits, t.exits[1].hits, t.exits[2].hits) = (33, 32, 64);
        assert_eq!(
            hot_guards(&t),
            [0x1004],
            "above half; a plain exit is no guard"
        );
        t.exits[1].hits = 60;
        assert_eq!(hot_guards(&t), [0x1004, 0x1008]);
        (t.exits[0].hits, t.exits[1].hits) = (0, 0);
        assert!(hot_guards(&t).is_empty());
    }
}
