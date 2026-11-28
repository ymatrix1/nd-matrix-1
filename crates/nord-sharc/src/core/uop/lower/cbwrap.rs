//! The circular-buffer wrap latch inside a block: why a `UF_CB` uop ends its block.
//!
//! OWNER: W1-LOWER. A uop with `UF_CB` (a DAG access that can wrap I7 or I15) is followed by
//! `latch_cb_overflow` in the dispatch loop (design 4.5 rule 6, P2). When a wrap happened, that
//! sets the sticky bit and latches CB7I or CB15I, and `raise_interrupt` pokes `irq.due` to 0. The
//! interpreter's NEXT step then starts with the sequencer prologue (`sequencer_event`): it takes
//! the interrupt when it is unmasked and recomputes `irq.due`, `irq.wake_at` and the timers when
//! it is not. A block that went on with its next uop would skip that prologue, so the interrupt
//! would be taken late and, even with CB7I and CB15I masked, `exact_diff` would see a stale
//! `irq_due` and `irq_wake_at` (both are compared fields). Design 4.5 rule 4 lists only `Interp`,
//! `Sys` and generic memory as uops that reach `poke_irq`, and P2 has no `due` re-check after
//! `UF_CB`, so the lowering closes the gap on its own, with no change to the dispatch loop:
//!
//! * the block ends after an instruction whose uops latch (`translate.rs` calls `end_block`): the
//!   next entry computes `room = min(limit, irq.due) - steps`, which is 0 after a wrap, fails the
//!   `Steps` row of the entry guard and runs one `Core::step`, whose prologue is the interpreter's;
//! * where the block cannot end there, because delay slots are still owed, the instruction is
//!   lowered as `Interp` instead (rule 5: it re-checks `due` itself).
//!
//! The cost is one block transition after each such instruction, paid whether or not the buffer
//! wrapped. If `exec/mod.rs` later re-checks `due` after a latching `UF_CB` uop (design 4.5 rule 4
//! extended to `UF_CB`), this rule and its tests can go.
use crate::core::uop::types::{Uop, UF_CB};

/// Whether one of `uops` (the uops of one instruction) is followed by the circular-buffer latch.
pub(in crate::core) fn latches_cb(uops: &[Uop]) -> bool {
    uops.iter().any(|u| u.flags & UF_CB != 0)
}

#[cfg(test)]
mod tests {
    use super::super::support::{build, fast_lower, fast_lower_cb, spans, JUMP_DB, NOP};
    use super::*;
    use crate::core::support::{core_with, CODE};
    use crate::core::uop::config::UopConfig;
    use crate::core::uop::types::{GuardClass, UopKind};

    /// MODE1.IRPTEN and the IMASK bits of CB7I and CB15I (`vectors.rs::slot(30)` and `slot(31)`
    /// are IRPTL bits 20 and 21; `interrupts.rs` reads `imask >> bit & 1`).
    const IRPTEN: u32 = 1 << 12;
    const CB_MASK: u32 = 0b11 << 20;

    #[test]
    fn a_wrap_latch_leaves_the_sequencer_to_the_next_interpreter_step() {
        // the premise of the rule, on the interpreter: three NOPs, a wrap recorded by the first
        let run = |mode1: u32, imask: u32| {
            let mut c = core_with(&[NOP; 3]);
            (c.mode1, c.imask) = (mode1, imask);
            c.irq.due = u64::MAX;
            c.cb_wrap.set(1);
            c.step().unwrap();
            let after_wrap = (c.irq.due, c.irq.irptl >> 20 & 1, c.irq.taken);
            c.step().unwrap();
            (after_wrap, c.irq.due, c.irq.taken, c.pc)
        };
        // masked: the latch pokes `due` to 0, the next step's prologue recomputes it
        let (after_wrap, due, taken, pc) = run(0, 0);
        assert_eq!(after_wrap, (0, 1, 0), "CB7I latched, due poked");
        assert_ne!(due, 0, "the prologue of the second step recomputed irq.due");
        assert_eq!((taken, pc), (0, CODE + 2));
        // enabled: the second step takes the interrupt instead of running the second NOP
        let (_, _, taken, pc) = run(IRPTEN, CB_MASK);
        assert_eq!(taken, 1);
        assert_ne!(pc, CODE + 2, "the vector, not the next instruction");
    }

    #[test]
    fn latches_cb_looks_at_the_flag_of_every_uop() {
        let plain = Uop::new(UopKind(UopKind::MOVE_BASE + 1), 1, CODE, 0, CODE + 1);
        let flagged = Uop {
            flags: UF_CB,
            ..plain
        };
        assert!(!latches_cb(&[]));
        assert!(!latches_cb(&[plain, plain]));
        assert!(latches_cb(&[plain, flagged]));
    }

    #[test]
    fn a_block_ends_after_the_instruction_whose_uop_latches_the_wrap() {
        let cfg = UopConfig::default();
        let none = build(&[NOP; 6], &cfg, None, fast_lower).1;
        assert_eq!(spans(&none, false), [(CODE, 6)], "no flag, one block");
        for cb_at in [2, 0, 5] {
            let lower = fast_lower_cb(vec![CODE + cb_at]);
            let (_, built) = build(&[NOP; 6], &cfg, None, lower);
            let mut want = vec![(CODE, cb_at + 1)];
            if cb_at < 5 {
                want.push((CODE + cb_at + 1, 5 - cb_at));
            }
            assert_eq!(spans(&built, false), want, "flag on instruction {cb_at}");
            assert_eq!(spans(&built, true), want, "and in the PEy-on variant");
            let (b, uops) = built.at(CODE, false).unwrap();
            assert_eq!(b.guard, GuardClass::Cb);
            assert_eq!(uops.last().map(|u| u.flags & UF_CB), Some(UF_CB));
        }
    }

    #[test]
    fn inside_a_delay_group_the_latching_instruction_is_lowered_interp() {
        let program = [&[NOP][..], &JUMP_DB, &[NOP; 3]].concat();
        let cfg = UopConfig::default();
        let kinds = |cb_at: u32| {
            let (_, built) = build(&program, &cfg, None, fast_lower_cb(vec![CODE + cb_at]));
            let (b, uops) = built.at(CODE, false).unwrap();
            let interp: Vec<bool> = uops.iter().map(|u| u.h == UopKind::INTERP.0).collect();
            (b.steps, interp, uops.iter().any(|u| u.flags & UF_CB != 0))
        };
        // NOP, JUMP (DB), slot, slot: the branch and the first slot cannot end the block, the
        // last slot ends it anyway
        assert_eq!(
            kinds(1),
            (4, vec![false, true, false, false], false),
            "branch"
        );
        assert_eq!(
            kinds(4),
            (4, vec![false, false, true, false], false),
            "slot 1"
        );
        assert_eq!(
            kinds(5),
            (4, vec![false, false, false, false], true),
            "slot 2"
        );
    }
}
