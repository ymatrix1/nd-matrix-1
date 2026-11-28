//! The stop pc and the start of a pass (design P7).
//!
//! OWNER: W2-DRIVE. The pass ends when a step lands on the idle pc, so no block may run past it:
//! the idle pc must be a block START, never an interior uop. When it is known before the engine is
//! armed, `uop_set_stop_pc` first and `enable_uop` second, the translator cuts in front of it.
//! When it is learnt or changed later (`uop_begin_pass(idle_pc)` of a pass the runner arms late, or
//! a second `uop_set_stop_pc`), blocks that already hold it as an interior uop are repaired here:
//! they are dropped (tombstoned, their range pushed to `hot.dropped`, the epoch bumped), their pcs
//! run through `Core::step` until `uop_maintain` retranslates the range with the new stop pc, and
//! the pass ends where the interpreter's does. A repeated call with the same pc does nothing, so
//! a steady pass pays one compare.
use crate::core::uop::run::CodePages;
use crate::core::uop::types::NONE;
use crate::core::Core;

impl Core {
    /// pc at which block chaining must return to the caller (the pass idle pc); `None` clears it.
    /// A new pc that lies inside a block drops that block (see the module header).
    pub fn uop_set_stop_pc(&mut self, pc: Option<u32>) {
        let new = pc.unwrap_or(NONE);
        let old = std::mem::replace(&mut self.uop_run.stop_pc, new);
        if new != NONE && new != old {
            self.uop_repair_stop_pc(new);
        }
    }

    /// Marks the start of a pass: counts it and sets the pc at which chaining returns (the idle
    /// pc of the pass). No-op while the engine is off.
    pub fn uop_begin_pass(&mut self, idle_pc: u32) {
        if self.uop_run.active {
            self.uop_run.stats.passes += 1;
            self.uop_set_stop_pc(Some(idle_pc));
        }
    }

    /// Drops every live block that holds `pc` as an interior uop (`b.pc < pc <= b.last_pc`), in
    /// both PEy modes and including a block the pc map no longer points at. Returns the number
    /// dropped. Allocates nothing: `drop_block` pushes into the capacity `install` reserved.
    fn uop_repair_stop_pc(&mut self, pc: u32) -> usize {
        let (Some(hot), Some(page)) = (self.uop.as_deref_mut(), CodePages::page_of(pc)) else {
            return 0;
        };
        let (mut k, mut dropped) = (0, 0);
        while let Some(&id) = hot.page_blocks[page].get(k) {
            k += 1;
            let b = &hot.blocks[id as usize];
            if b.len != 0 && b.pc < pc && pc <= b.last_pc && hot.drop_block(id) {
                dropped += 1;
            }
        }
        dropped
    }
}

#[cfg(test)]
mod tests {
    use crate::core::support::CODE;
    use crate::core::uop::drive::fixture::{arm_interp, nop_core};
    use crate::core::uop::types::NONE;

    #[test]
    fn uop_begin_pass_repairs_a_late_stop_pc() {
        // armed with no stop pc: ten NOPs are one block per mode and the idle pc CODE + 6 is an
        // interior uop
        let mut c = nop_core(10);
        arm_interp(&mut c, &[(CODE, CODE + 10)]);
        let hot = c.uop.as_ref().unwrap();
        assert_eq!(
            (
                hot.pcmap(false).get(CODE + 6),
                hot.pcmap(true).get(CODE + 6)
            ),
            (Some((0, 6)), Some((1, 6)))
        );
        assert!(!c.uop_needs_maintenance());
        c.uop_begin_pass(CODE + 6);
        assert_eq!((c.uop_stats().passes, c.uop_run.stop_pc), (1, CODE + 6));
        // both blocks are gone and wait for the retranslation with the new stop pc
        let hot = c.uop.as_ref().unwrap();
        assert!(hot.blocks.iter().all(|b| b.len == 0 && b.poisoned));
        assert_eq!(
            (hot.pcmap(false).get(CODE), hot.pcmap(true).get(CODE + 9)),
            (None, None)
        );
        assert_eq!(hot.dropped.len(), 2);
        assert!(c.uop_needs_maintenance());
        // the same pc again is free: nothing more is dropped, the epoch stays
        let epoch = hot.epoch;
        c.uop_begin_pass(CODE + 6);
        assert_eq!(
            (c.uop_stats().passes, c.uop.as_ref().unwrap().epoch),
            (2, epoch)
        );
        // until then the pc runs through the interpreter, and a maintain call retranslates: the
        // block in front of the idle pc ends there and a block starts at it
        c.irq.due = u64::MAX;
        assert!(!c.uop_try(100));
        assert!(c.uop_maintain() > 0);
        let hot = c.uop.as_ref().unwrap();
        for simd in [false, true] {
            let (id, idx) = hot.pcmap(simd).get(CODE + 6).expect("mapped again");
            assert_eq!(
                (idx, hot.blocks[id as usize].pc),
                (0, CODE + 6),
                "mode {simd}"
            );
            let (before, _) = hot.pcmap(simd).get(CODE + 5).unwrap();
            assert_eq!(hot.blocks[before as usize].end_pc, CODE + 6, "mode {simd}");
        }
    }

    #[test]
    fn a_pc_that_is_already_a_block_start_or_not_in_a_block_drops_nothing() {
        let mut c = nop_core(10);
        c.uop_set_stop_pc(Some(CODE + 4));
        arm_interp(&mut c, &[(CODE, CODE + 10)]);
        let blocks = c.uop.as_ref().unwrap().blocks.len();
        c.uop_begin_pass(CODE + 4);
        assert!(
            !c.uop_needs_maintenance(),
            "armed with it: it is a block start"
        );
        c.uop_begin_pass(CODE); // the first pc of the first block: a start too
        c.uop_begin_pass(CODE + 40); // outside every block
        c.uop_begin_pass(0x40); // outside the code window
        assert!(!c.uop_needs_maintenance());
        assert!(c.uop.as_ref().unwrap().blocks.iter().all(|b| b.len != 0));
        assert_eq!(c.uop.as_ref().unwrap().blocks.len(), blocks);
        // clearing the stop pc drops nothing either
        c.uop_set_stop_pc(None);
        assert_eq!(c.uop_run.stop_pc, NONE);
        assert!(!c.uop_needs_maintenance());
    }

    #[test]
    fn a_disabled_engine_counts_no_pass_and_remembers_the_pc_for_enable() {
        let mut c = nop_core(10);
        c.uop_begin_pass(CODE + 3);
        assert_eq!((c.uop_stats().passes, c.uop_run.stop_pc), (0, NONE));
        c.uop_set_stop_pc(Some(CODE + 3));
        assert_eq!(c.uop_run.stop_pc, CODE + 3);
        arm_interp(&mut c, &[(CODE, CODE + 10)]);
        let hot = c.uop.as_ref().unwrap();
        assert_eq!(hot.pcmap(false).get(CODE + 3).map(|e| e.1), Some(0));
    }
}
