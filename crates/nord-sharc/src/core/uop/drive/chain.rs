//! The block-entry hook: `uop_try` runs one or more chained blocks.
//!
//! OWNER: W2-DRIVE. Same contract as `aot_try` (`aot_hot.rs`): it returns whether any step ran, a
//! fault is left for `uop_take_fault`, and after a `false` the caller runs exactly one
//! `Core::step`. One call is a chain of blocks:
//!
//! 1. revalidate the pages a write made dirty (`uop_revalidate_dirty`, no work when none is);
//! 2. find the block at `core.pc` in the pc map of the current `core.simd()` (design P4), or
//!    follow the cached link of the block that just exited (valid while its epoch is the
//!    engine's, so a dropped block never runs through a stale link);
//! 3. evaluate the entry guard (`lower::guard::check`) against `room = min(limit, irq.due) -
//!    steps`; the first failing row is counted, but only for the first block of the call (a later
//!    block that fails is entered again by the next call, which then counts it);
//! 4. run it (`exec::run_block`, or the checked loop when the room does not cover the rest of
//!    the block, or the native entry) and account the exit;
//! 5. continue with the next block after `Ok`, `Jump` and `Smc`, unless the pc is the stop pc
//!    (the pass idle pc: the pass ends when a step lands on it, so chaining returns there), no
//!    step ran, or a fault is waiting.
//!
//! Exits (design 4.5): `Fault`, `Guard` and `Slow` did not run their uop and materialized its
//! pc. When steps ran before them, the call returns `true` and the next call enters the same
//! block at that uop (mid-block entry through the pc map). When none ran in the first block the
//! call returns `false` (the caller steps); in a later block of the chain the driver runs that
//! step itself ([`Core::uop_replay`]), so no exit is counted twice and the counters keep
//! `steps_uop + steps_core_step` equal to the steps run. A `Fault` from a fast handler
//! (precondition failed) is replayed through `Core::step`: the real `Fault` comes from the
//! interpreter, so partial effects are exact by construction; the `Interp` handler has already
//! stored its own.
use crate::core::uop::config::UopStats;
use crate::core::uop::exec::{self, dispatch::steps_from, event, exit};
use crate::core::uop::hot::UopHot;
use crate::core::uop::lower::guard;
use crate::core::uop::types::{Exit, Link, NONE};
use crate::core::Core;

impl Core {
    /// Runs one or more chained blocks when `limit` leaves room and the guard holds; returns
    /// whether any step ran. A fault is left for [`Core::uop_take_fault`]. Same contract as
    /// `aot_try`.
    #[inline]
    pub fn uop_try(&mut self, limit: u64) -> bool {
        self.uop_run.active && self.uop_enter(limit)
    }

    #[inline(never)]
    fn uop_enter(&mut self, limit: u64) -> bool {
        let Some(mut hot) = self.uop.take() else {
            return false;
        };
        let ran = self.uop_chain(&mut hot, limit);
        self.uop = Some(hot);
        // the caller runs exactly one `Core::step` after a false return
        self.uop_run.stats.steps_core_step += u64::from(!ran);
        ran
    }

    /// The chain of one `uop_try` (steps 1 to 5 of the module header).
    fn uop_chain(&mut self, hot: &mut UopHot, limit: u64) -> bool {
        let mut ran = false;
        let mut via: Option<(u32, usize)> = None;
        loop {
            if self.uop_run.dirty != 0 {
                self.uop_revalidate_dirty(hot);
            }
            let Some((id, from)) = next_block(hot, via, self.pc, self.simd()) else {
                return ran;
            };
            let room = event::room(self, limit);
            if let Some(row) = guard::check(self, &hot.blocks[id as usize], room) {
                if !ran {
                    self.uop_run.stats.guard_fail[row as usize] += 1;
                }
                return ran;
            }
            let native = hot.blocks[id as usize].native != NONE && from == 0;
            let (e, at, done) = self.uop_exec(hot, id, from, room, native);
            count(&mut self.uop_run.stats, (e, from, at, done), native);
            match e {
                Exit::Fault => {
                    if self.uop_run.fault.is_none() {
                        self.uop_replay();
                    }
                    return true;
                }
                Exit::Guard | Exit::Slow => {
                    if done == 0 && !ran {
                        return false;
                    }
                    if done == 0 {
                        self.uop_replay();
                    }
                    return true;
                }
                Exit::Ok | Exit::Jump | Exit::Smc => {}
            }
            if done == 0 || self.pc == self.uop_run.stop_pc || self.uop_run.fault.is_some() {
                return true;
            }
            ran = true;
            via = Some((id, usize::from(e == Exit::Jump)));
        }
    }

    /// Runs block `id` from uop `from`: the native entry when it has one and starts at the first
    /// uop, else the dispatch loop, hot when `room` covers the rest of the block.
    fn uop_exec(
        &mut self,
        hot: &UopHot,
        id: u32,
        from: u32,
        room: u32,
        native: bool,
    ) -> (Exit, u32, u32) {
        let need = steps_from(&hot.blocks[id as usize], from);
        // SAFETY: `hot` is the table the block was built for, `id` and `from` come from its pc
        // map, and no other reference to the core is alive: `hot` was taken out of `self.uop`.
        unsafe {
            if native {
                self.uop_native(hot, id, room)
            } else if room >= need {
                exec::run_block(self, hot, id, from, room)
            } else {
                exec::run_block_checked(self, hot, id, from, room)
            }
        }
    }

    /// The step a caller would run after a false `uop_try`, run by the driver when a block of the
    /// chain makes no progress after earlier blocks did, and for a `Fault` exit of a fast handler.
    /// A fault is stored for `uop_take_fault`.
    fn uop_replay(&mut self) {
        match self.step() {
            Ok(()) => self.uop_run.stats.steps_core_step += 1,
            Err(f) => self.uop_run.fault = Some(f),
        }
    }

    /// Runs block `id` through the native tier.
    ///
    /// # Safety
    /// As `exec::run_block`.
    #[cfg(feature = "uop-native")]
    unsafe fn uop_native(&mut self, hot: &UopHot, id: u32, room: u32) -> (Exit, u32, u32) {
        crate::core::uop::native::enter(self, hot, id, room)
    }

    /// Without the native tier no block carries a native entry; the interpreter runs it.
    ///
    /// # Safety
    /// None needed; `unsafe` only to match the native variant.
    #[cfg(not(feature = "uop-native"))]
    unsafe fn uop_native(&mut self, _hot: &UopHot, _id: u32, _room: u32) -> (Exit, u32, u32) {
        (Exit::Slow, 0, 0)
    }
}

/// Accounts one block run: the block, the uops and steps it ran and its exit.
fn count(st: &mut UopStats, (e, from, at, done): (Exit, u32, u32, u32), native: bool) {
    st.blocks_run += 1;
    st.uops += u64::from(exit::uops_run(e, from, at));
    st.steps_uop += u64::from(done);
    if native {
        st.steps_native += u64::from(done);
    }
    exit::count_exit(st, e);
}

/// The `(block, uop index)` to enter at `pc`: the block `via` (the last block and its link slot,
/// 0 after `Ok`, 1 after `Jump`) points at when the link is current, else the pc map entry; a
/// block found through the map at its first uop is cached as the link of `via` for next time.
fn next_block(
    hot: &mut UopHot,
    via: Option<(u32, usize)>,
    pc: u32,
    simd: bool,
) -> Option<(u32, u32)> {
    if let Some((from, slot)) = via {
        let l = hot.blocks[from as usize].links[slot];
        if l.id != NONE && l.epoch == hot.epoch {
            if let Some(b) = hot.blocks.get(l.id as usize) {
                if b.pc == pc && b.len != 0 && b.simd == simd {
                    return Some((l.id, 0));
                }
            }
        }
    }
    let (id, idx) = hot.pcmap(simd).get(pc)?;
    if let (0, Some((from, slot))) = (idx, via) {
        hot.blocks[from as usize].links[slot] = Link {
            id,
            epoch: hot.epoch,
        };
    }
    Some((id, idx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::CODE;
    use crate::core::uop::drive::fixture::{arm_interp, nop_core};
    use crate::core::uop::types::GuardFail;

    #[test]
    fn a_core_without_the_engine_or_without_a_block_leaves_the_step_to_the_caller() {
        let mut off = nop_core(4);
        assert!(!off.uop_try(100), "not armed: one byte load, no counter");
        assert_eq!(off.uop_stats(), UopStats::default());
        let mut c = nop_core(8);
        c.enable_uop(Default::default(), &[(CODE + 4, CODE + 8)])
            .unwrap();
        assert!(!c.uop_try(100), "the pc is outside every block");
        // the only counters that moved: the one step and the two blocks (one per PEy mode)
        let one_core_step = UopStats {
            steps_core_step: 1,
            translations: 2,
            ..UopStats::default()
        };
        assert_eq!(c.uop_stats(), one_core_step);
        assert_eq!(c.run_uop(3), None);
        assert_eq!(
            (c.steps, c.pc, c.uop_stats().steps_core_step),
            (3, CODE + 3, 4)
        );
    }

    #[test]
    fn a_block_runs_whole_and_the_chain_stops_where_the_blocks_end() {
        let mut c = nop_core(10);
        arm_interp(&mut c, &[(CODE, CODE + 10)]);
        assert!(c.uop_try(100));
        let s = c.uop_stats();
        assert_eq!((c.steps, c.pc), (10, CODE + 10));
        assert_eq!((s.blocks_run, s.uops, s.steps_uop), (1, 10, 10));
        assert_eq!((s.exits[Exit::Ok as usize], s.steps_core_step), (1, 0));
        assert_eq!(s.steps_interp_uop, 10);
        // past the translated code there is no block: the caller steps
        assert!(!c.uop_try(100));
        assert_eq!(c.uop_stats().steps_core_step, 1);
    }

    #[test]
    fn the_limit_is_the_room_and_the_checked_loop_stops_in_front_of_the_uop_that_does_not_fit() {
        for limit in 0..=11 {
            let (mut want, mut got) = (nop_core(10), nop_core(10));
            arm_interp(&mut got, &[(CODE, CODE + 10)]);
            let ran = got.uop_try(limit);
            assert_eq!(ran, limit > 0, "limit {limit}");
            let n = limit.min(10);
            assert_eq!(want.run(n), None);
            assert_eq!(want.state_mismatch(&got), None, "limit {limit}");
            if limit == 0 {
                let guard = got.uop_stats().guard_fail;
                assert_eq!(
                    guard[GuardFail::Steps as usize],
                    1,
                    "no room is the first guard row"
                );
            }
        }
        let mut c = nop_core(10);
        arm_interp(&mut c, &[(CODE, CODE + 10)]);
        assert!(c.uop_try(4));
        let s = c.uop_stats();
        assert_eq!(
            (c.steps, s.exits[Exit::Slow as usize], s.steps_uop),
            (4, 1, 4)
        );
    }

    #[test]
    fn the_stop_pc_ends_the_chain_and_every_exit_is_counted_once() {
        // two blocks: the NOPs up to the stop pc, then the rest; chaining runs the first only
        let mut c = nop_core(10);
        c.uop_set_stop_pc(Some(CODE + 5));
        arm_interp(&mut c, &[(CODE, CODE + 10)]);
        assert!(c.uop_try(100));
        assert_eq!((c.steps, c.pc), (5, CODE + 5));
        // entered at the stop pc itself, the block runs: that is how a pass starts
        assert!(c.uop_try(100));
        assert_eq!((c.steps, c.pc), (10, CODE + 10));
        let s = c.uop_stats();
        assert_eq!((s.blocks_run, s.exits[Exit::Ok as usize]), (2, 2));
        // with no stop pc the same two blocks chain in one call
        let mut free = nop_core(10);
        arm_interp(&mut free, &[(CODE, CODE + 5), (CODE + 5, CODE + 10)]);
        assert!(free.uop_try(100));
        assert_eq!(free.steps, 10);
        assert_eq!(free.uop_stats().blocks_run, 2);
    }

    #[test]
    fn a_link_is_cached_on_first_use_and_dies_with_the_epoch() {
        let mut c = nop_core(10);
        arm_interp(&mut c, &[(CODE, CODE + 5), (CODE + 5, CODE + 10)]);
        assert!(c.uop_try(100));
        let first = c.uop.as_ref().unwrap().blocks[0].links[0];
        assert_eq!(
            first.id,
            c.uop
                .as_ref()
                .unwrap()
                .pcmap(false)
                .get(CODE + 5)
                .unwrap()
                .0
        );
        assert_ne!(first.id, NONE);
        // drop the target block: the epoch moves, the link is not followed any more
        let hot = c.uop.as_mut().unwrap();
        assert!(hot.drop_block(first.id));
        assert_ne!(hot.epoch, first.epoch);
        c.pc = CODE;
        c.steps = 0;
        c.irq.due = u64::MAX;
        assert!(c.uop_try(100));
        assert_eq!(
            c.steps, 5,
            "the chain ended: the target is gone, the caller steps next"
        );
    }
}
