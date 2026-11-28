//! Entry-guard classification of a block and the names of the guard rows.
//!
//! OWNER: W1-LOWER. The guard itself is evaluated at block entry by the driver (design 7.1, rows
//! below, `UopStats::guard_fail` is indexed by [`GuardFail`]); this file decides how strong a
//! guard a block needs from its uops (`Loop` when it contains a loop end, `Delayed` for delay
//! slots, `Cb` for circular-buffer uops, `Arith` for uops that can raise an arithmetic
//! exception). Skeleton: every block is `Plain`, which is right while it holds `Interp` uops only
//! (the interpreter raises its own events).
use super::super::types::{Block, GuardClass, GuardFail, Uop};
use crate::core::Core;

/// The guard class a block of `uops` needs.
pub fn class_of(uops: &[Uop]) -> GuardClass {
    let _ = uops;
    GuardClass::Plain
}

/// The first failing row of the entry guard of design 7.1 for `block` on `core`, or `None` when
/// the block may run. `room` is `min(limit, irq.due) - steps` (`exec::event::room`); the `Steps`
/// row fails only when there is no room at all, because a block that does not fit runs in the
/// checked loop (design 7.2) and exits `Slow` at the first uop that does not.
pub(in crate::core) fn check(core: &Core, block: &Block, room: u32) -> Option<GuardFail> {
    let can_delay = block.guard == GuardClass::Delayed;
    let loop_hit = core
        .loops
        .last()
        .is_some_and(|f| f.term.is_some() || (block.pc..block.last_pc).contains(&f.end));
    let row = [
        (room == 0, GuardFail::Steps),
        (
            core.irq.window_len != 0 || core.irq.idle || core.irq.stall != 0,
            GuardFail::Window,
        ),
        (core.pending.is_some() && !can_delay, GuardFail::Pending),
        (loop_hit, GuardFail::Loop),
        (core.simd() != block.simd, GuardFail::Simd),
        (
            core.trace.is_some() || !core.site_hits.is_empty(),
            GuardFail::Trace,
        ),
        (
            core.cb_wrap.get() != 0 && block.guard != GuardClass::Cb,
            GuardFail::CbWrap,
        ),
        (
            block.guard == GuardClass::Arith && core.arith_irq_enabled(),
            GuardFail::Arith,
        ),
        (
            block.poisoned || core.isa_at(block.pc) != block.isa,
            GuardFail::Valid,
        ),
    ];
    row.iter().find(|(fails, _)| *fails).map(|(_, r)| *r)
}

/// Name of a guard row, for reports.
pub const fn name(row: GuardFail) -> &'static str {
    match row {
        GuardFail::Steps => "steps past min(limit, irq.due)",
        GuardFail::Window => "interrupt window, idle or stall",
        GuardFail::Pending => "delayed branch pending",
        GuardFail::Loop => "loop frame ends inside the block",
        GuardFail::Simd => "SIMD mode differs",
        GuardFail::Trace => "trace or site counters active",
        GuardFail::CbWrap => "circular-buffer wrap latch set",
        GuardFail::Arith => "arithmetic-exception interrupt enabled",
        GuardFail::Valid => "block poisoned or ISA mode changed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::uop::types::UopKind;

    fn block_at(pc: u32) -> Block {
        Block {
            last_pc: pc + 2,
            steps: 3,
            ..Block::empty(pc)
        }
    }

    #[test]
    fn a_quiet_core_passes_and_each_row_fails_for_its_own_reason() {
        let mut c = core_with(&[0x0001, 0x0001, 0x0001]);
        let b = block_at(c.pc);
        assert_eq!(check(&c, &b, 3), None);
        assert_eq!(check(&c, &b, 0), Some(GuardFail::Steps));
        c.irq.stall = 1;
        assert_eq!(check(&c, &b, 3), Some(GuardFail::Window));
        c.irq.stall = 0;
        c.pending = Some(crate::core::types::Pending {
            slots: 2,
            target: 0,
            call: false,
        });
        assert_eq!(check(&c, &b, 3), Some(GuardFail::Pending));
        let delayed = Block {
            guard: GuardClass::Delayed,
            ..b.clone()
        };
        assert_eq!(
            check(&c, &delayed, 3),
            None,
            "a delayed-branch block may start pending"
        );
        c.pending = None;
        c.mode1 |= 1 << 21;
        assert_eq!(check(&c, &b, 3), Some(GuardFail::Simd));
        c.mode1 &= !(1 << 21);
        c.cb_wrap.set(1);
        assert_eq!(check(&c, &b, 3), Some(GuardFail::CbWrap));
        assert_eq!(
            check(
                &c,
                &Block {
                    guard: GuardClass::Cb,
                    ..b.clone()
                },
                3
            ),
            None
        );
        c.cb_wrap.set(0);
        assert_eq!(
            check(
                &c,
                &Block {
                    poisoned: true,
                    ..b.clone()
                },
                3
            ),
            Some(GuardFail::Valid)
        );
    }

    #[test]
    fn a_loop_ending_inside_the_block_fails_but_one_ending_at_the_last_instruction_does_not() {
        let mut c = core_with(&[0x0001, 0x0001, 0x0001]);
        let b = block_at(c.pc);
        let frame = |end| crate::core::types::LoopFrame {
            end,
            ..Default::default()
        };
        c.loops.push(frame(c.pc + 1));
        assert_eq!(check(&c, &b, 3), Some(GuardFail::Loop));
        c.loops[0] = frame(c.pc + 2);
        assert_eq!(
            check(&c, &b, 3),
            None,
            "the loop end is block-final (a Fin uop)"
        );
        c.loops[0] = frame(c.pc + 9);
        assert_eq!(check(&c, &b, 3), None);
    }

    #[test]
    fn the_skeleton_needs_only_a_plain_guard_and_every_row_has_a_distinct_name() {
        assert_eq!(
            class_of(&[Uop::new(UopKind::INTERP, 1, 0, 0, 0)]),
            GuardClass::Plain
        );
        let rows = [
            GuardFail::Steps,
            GuardFail::Window,
            GuardFail::Pending,
            GuardFail::Loop,
            GuardFail::Simd,
            GuardFail::Trace,
            GuardFail::CbWrap,
            GuardFail::Arith,
            GuardFail::Valid,
        ];
        assert_eq!(rows.len(), GuardFail::COUNT);
        let mut names: Vec<_> = rows.iter().map(|r| name(*r)).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), GuardFail::COUNT);
    }
}
