//! OWNER: W2-DRV. Entering a trace and chaining into its successor (design note 4.1).
//!
//! [`enter`] runs one trace and settles what the run left behind in the order the contract needs:
//! the exit's counters and hit count, the fault (`cpu.fault`, taken after every exit kind), the
//! stale drain (a chain never continues into a trace that just died, I11), the flipped
//! re-layout check, and only then the [`Leave`] the next dispatch links through. [`resolve`] is
//! that next dispatch: the linked trace of the exit when it is live and starts at the pc, else a
//! lookup that patches the link. A link or inline-cache entry may name a dead trace (a trace that
//! was killed without the link reset, a flush); it is never followed.

use super::super::backend::{RunOut, EXIT_SLOW};
use super::super::exec;
use super::super::stale::is_active;
use super::super::uop::{ExitKind, Sb, SbId, NO_SB};
use super::super::SbState;
use super::{build, drain_stale};
use crate::cpu::Fault;
use crate::machine::Machine;

/// Entries of a trace after which the guard exits are inspected for the flipped re-layout.
/// ASSUMED: design note 4.1 (64 entries, guard exit above half of them); the counters run (G0)
/// confirm or move it.
pub(super) const RELAYOUT_AFTER: u32 = 64;

/// The exit a run left through, kept for the next dispatch to link or look up through.
#[derive(Clone, Copy, Debug)]
pub(super) struct Leave {
    pub id: SbId,
    pub exit: usize,
    pub kind: ExitKind,
}

/// I7: whether `t` may be entered with `left` instructions of budget and the stop addresses
/// `until` and `stop` (`u32::MAX` for none): the whole trace fits the budget and none of its
/// pages holds either address.
pub(super) fn enterable(t: &Sb, left: u64, until: u32, stop: u32) -> bool {
    (u64::from(t.n_guest) <= left || is_active(600))
        && ((!t.covers_page(until >> 12) && !t.covers_page(stop >> 12)) || is_active(601))
}

/// True when the exit kind has a static or cacheable successor worth linking.
fn linkable(kind: ExitKind) -> bool {
    matches!(
        kind,
        ExitKind::Direct
            | ExitKind::LoopBack
            | ExitKind::Indirect
            | ExitKind::IndirectArm
            | ExitKind::Dynamic
    )
}

/// True for the exit kinds with one fixed target (the link), false for the ones that vary
/// (the two-way inline cache).
fn fixed_target(kind: ExitKind) -> bool {
    matches!(kind, ExitKind::Direct | ExitKind::LoopBack)
}

/// A trace that is live and starts at `pc`: the only successor a link may name.
fn follows(sb: &SbState, id: SbId, pc: u32) -> bool {
    id != NO_SB
        && (is_active(603) && id < sb.cache.slots() as SbId
            || sb.cache.is_live(id) && sb.cache.get(id).entry == pc)
}

/// The trace to run at `pc`: through the exit `from` left by when it names one, else by lookup
/// (which patches the link or inline cache of `from`).
pub(super) fn resolve(sb: &mut SbState, from: Option<Leave>, pc: u32) -> Option<SbId> {
    let Some(l) = from.filter(|l| sb.cache.is_live(l.id)) else {
        return sb.cache.lookup(pc);
    };
    let x = &sb.cache.get(l.id).exits[l.exit];
    if fixed_target(l.kind) {
        if follows(sb, x.link, pc) {
            return Some(x.link);
        }
    } else if let Some(w) = (0..2).find(|&w| x.ic_pc[w] == pc && follows(sb, x.ic_id[w], pc)) {
        return Some(x.ic_id[w]);
    }
    let found = sb.cache.lookup(pc)?;
    if !is_active(606) {
        let x = &mut sb.cache.get_mut(l.id).exits[l.exit];
        if fixed_target(l.kind) {
            x.link = found;
        } else {
            (x.ic_pc[1], x.ic_id[1]) = (x.ic_pc[0], x.ic_id[0]);
            (x.ic_pc[0], x.ic_id[0]) = (pc, found);
        }
    }
    Some(found)
}

/// Run trace `id` with `*left` instructions of budget: the backend when the trace is compiled
/// and no watch is armed (a backend has no watch mode), else `exec::run_sb`. A backend that
/// reports `EXIT_SLOW` ran nothing: the trace runs again with `exec::run_sb`.
fn run_trace(m: &mut Machine, sb: &mut SbState, id: SbId, left: &mut u64, watch: bool) -> RunOut {
    let native = sb.cache.get(id).native;
    if native != NO_SB && !watch {
        if let Some(be) = sb.backend.as_mut() {
            sb.stats.native_enters += 1;
            let out = be.run(native, &mut m.cpu, &mut m.bus, left);
            if out.exit != EXIT_SLOW {
                return out;
            }
            sb.stats.native_slow_exits += 1;
        }
    }
    exec::run_sb(&mut m.cpu, &mut m.bus, sb.cache.get(id), left, watch)
}

/// Run trace `id` (I7 holds) with the budget `limit` and settle its exit. `Err` is the fault of
/// the run (state drained, trace counters updated). `Ok` carries the exit to link through, if any.
pub(super) fn enter(
    m: &mut Machine,
    sb: &mut SbState,
    id: SbId,
    limit: u64,
) -> Result<Option<Leave>, Fault> {
    let before = m.cpu.insns;
    let mut left = limit - before;
    let out = run_trace(m, sb, id, &mut left, sb.watch.armed);
    sb.stats.enters += 1;
    sb.stats.guest_in_traces += m.cpu.insns - before;
    let idx = usize::from(out.exit);
    let t = sb.cache.get_mut(id);
    t.enters = t.enters.saturating_add(1);
    let enters = t.enters;
    // an exit index past the table is `EXIT_STEPPED`: applied, nothing static to count or link
    let kind = t.exits.get_mut(idx).map(|e| {
        e.hits = e.hits.saturating_add(1);
        e.kind
    });
    match kind {
        Some(k) => sb.stats.record_exit(k),
        None => sb.stats.native_slow_exits += 1,
    }
    let fault = if is_active(605) {
        None
    } else {
        m.cpu.fault.take()
    };
    if !is_active(602) {
        drain_stale(m, sb);
    }
    if let Some(f) = fault {
        return Err(f);
    }
    if enters == RELAYOUT_AFTER && sb.cache.is_live(id) && build::relayout(m, sb, id) {
        return Ok(None);
    }
    Ok(kind
        .filter(|k| linkable(*k) && sb.cache.is_live(id))
        .map(|kind| Leave {
            id,
            exit: idx,
            kind,
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::cache::tiny_trace;
    use crate::machine::sb::SbConfig;

    #[test]
    fn a_trace_is_enterable_only_inside_the_budget_and_off_the_stop_pages() {
        let t = tiny_trace(0x1000, &[(0x1000, 1), (0x3000, 2)]); // n_guest 1, pages 1 and 3
        assert!(enterable(&t, 1, 0, u32::MAX));
        assert!(!enterable(&t, 0, 0, u32::MAX), "budget below n_guest");
        assert!(!enterable(&t, 5, 0x1FFC, u32::MAX), "until on a page of it");
        assert!(!enterable(&t, 5, 0, 0x3004), "stop on a page of it");
        assert!(enterable(&t, 5, 0x2000, 0x4000), "other pages");
    }

    #[test]
    fn only_exits_with_a_successor_are_linkable() {
        let kinds = [
            ExitKind::Direct,
            ExitKind::Indirect,
            ExitKind::IndirectArm,
            ExitKind::Dynamic,
            ExitKind::Fault,
            ExitKind::Stale,
            ExitKind::LoopBack,
            ExitKind::Watch,
        ];
        let got: Vec<bool> = kinds.into_iter().map(linkable).collect();
        assert_eq!(got, [true, true, true, true, false, false, true, false]);
        assert!(fixed_target(ExitKind::Direct) && !fixed_target(ExitKind::Indirect));
    }

    #[test]
    fn resolve_patches_the_link_and_the_inline_cache_and_ignores_dead_ids() {
        let mut sb = SbState {
            cfg: SbConfig::on(),
            ..SbState::default()
        };
        let a = sb.cache.insert(tiny_trace(0x1000, &[(0x1000, 1)]));
        let b = sb.cache.insert(tiny_trace(0x2000, &[(0x2000, 1)]));
        let c = sb.cache.insert(tiny_trace(0x3000, &[(0x3000, 1)]));
        let via = |kind| {
            Some(Leave {
                id: a,
                exit: 0,
                kind,
            })
        };
        // no exit to link through: a plain lookup
        assert_eq!(resolve(&mut sb, None, 0x2000), Some(b));
        assert_eq!(resolve(&mut sb, None, 0x2004), None);
        // a fixed target is looked up once and linked
        assert_eq!(resolve(&mut sb, via(ExitKind::Direct), 0x2000), Some(b));
        assert_eq!(sb.cache.get(a).exits[0].link, b);
        // a link that starts elsewhere than the pc is not followed
        sb.cache.get_mut(a).exits[0].link = c;
        assert_eq!(resolve(&mut sb, via(ExitKind::Direct), 0x2000), Some(b));
        // an indirect exit fills its two-way cache, newest first
        assert_eq!(resolve(&mut sb, via(ExitKind::Indirect), 0x2000), Some(b));
        assert_eq!(resolve(&mut sb, via(ExitKind::Indirect), 0x3000), Some(c));
        let x = &sb.cache.get(a).exits[0];
        assert_eq!((x.ic_pc, x.ic_id), ([0x3000, 0x2000], [c, b]));
        // a dead target is never followed
        sb.cache.kill(b);
        assert_eq!(resolve(&mut sb, via(ExitKind::Indirect), 0x2000), None);
        // and a dead source falls back to a plain lookup
        sb.cache.kill(a);
        assert_eq!(resolve(&mut sb, via(ExitKind::Direct), 0x3000), Some(c));
    }
}
