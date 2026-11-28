//! Entry-guard classification of a block and the names of the guard rows.
//!
//! OWNER: W1-LOWER. The guard itself is evaluated at block entry by the driver (design 7.1, rows
//! below, `UopStats::guard_fail` is indexed by [`GuardFail`]); this file decides how strong a
//! guard a block needs from its uops (`Loop` when it contains a loop end, `Delayed` for delay
//! slots, `Cb` for circular-buffer uops, `Arith` for uops that can raise an arithmetic
//! exception). A block holding `Interp` uops only is `Plain` (the interpreter raises its own
//! events and runs its own `finish_step`).
//!
//! The class is one value, so a block that qualifies for several gets the one whose absence is
//! worst: `Arith` first (a fast uop that raised the exception late would be WRONG, not slow),
//! then `Delayed`, `Cb`, `Loop`. The others only cost a failed guard, never exactness.
use super::super::types::{Block, GuardClass, GuardFail, KindFamily, Uop, UopKind, UF_CB};
use super::builder::FP_LAST_SEQ;
use crate::core::Core;

/// The guard class of the uops alone (no translator knowledge): `Arith` when one is in the float
/// or fixed-point family range, `Cb` when one has `UF_CB`, else `Plain`.
pub fn class_of(uops: &[Uop]) -> GuardClass {
    classify(uops, false, false, false)
}

/// The guard class of a block: the translator adds what it knows from the instructions (a delay
/// group, a fast uop for an instruction that can raise an arithmetic exception, a static loop
/// end). Any uop of the float or fixed-point family counts as able to raise an arithmetic
/// exception even when the translator did not note it (cheap: `Arith` only matters while the
/// exception interrupt is enabled).
pub fn classify(uops: &[Uop], delayed: bool, arith: bool, loop_end: bool) -> GuardClass {
    let unit = |u: &Uop| {
        matches!(
            UopKind(u.h).family(),
            Some(KindFamily::Float | KindFamily::Fixed)
        )
    };
    if arith || uops.iter().any(unit) {
        GuardClass::Arith
    } else if delayed {
        GuardClass::Delayed
    } else if uops.iter().any(|u| u.flags & UF_CB != 0) {
        GuardClass::Cb
    } else if loop_end {
        GuardClass::Loop
    } else {
        GuardClass::Plain
    }
}

/// The first failing row of the entry guard of design 7.1 for `block` on `core`, or `None` when
/// the block may run. `room` is `min(limit, irq.due) - steps` (`exec::event::room`); the `Steps`
/// row fails only when there is no room at all, because a block that does not fit runs in the
/// checked loop (design 7.2) and exits `Slow` at the first uop that does not.
pub(in crate::core) fn check(core: &Core, block: &Block, room: u32) -> Option<GuardFail> {
    let can_delay = block.guard == GuardClass::Delayed;
    // A frame ending at the LAST instruction is fine only when the last uop runs `finish_step`
    // (`FP_LAST_SEQ`): the translator knows the loop ends of the code it saw, but a frame can
    // come from a DO outside the translated range or from PUSH LOOP / LADDR writes.
    let seq_last = block.footprint.writes & FP_LAST_SEQ != 0;
    let loop_hit = core.loops.last().is_some_and(|f| {
        f.term.is_some()
            || (block.pc..block.last_pc).contains(&f.end)
            || (f.end == block.last_pc && !seq_last)
    });
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
    use crate::core::uop::types::{Footprint, UF_FIN};

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
    fn a_loop_ending_inside_the_block_fails_and_one_at_the_last_instruction_needs_a_finishing_uop()
    {
        let mut c = core_with(&[0x0001, 0x0001, 0x0001]);
        let b = block_at(c.pc);
        let finishing = Block {
            footprint: Footprint {
                writes: FP_LAST_SEQ,
                ..Footprint::default()
            },
            ..b.clone()
        };
        let frame = |end| crate::core::types::LoopFrame {
            end,
            ..Default::default()
        };
        c.loops.push(frame(c.pc + 1));
        assert_eq!(check(&c, &b, 3), Some(GuardFail::Loop));
        assert_eq!(
            check(&c, &finishing, 3),
            Some(GuardFail::Loop),
            "interior end"
        );
        c.loops[0] = frame(c.pc + 2);
        assert_eq!(
            check(&c, &finishing, 3),
            None,
            "the loop end is the last instruction and its uop runs finish_step"
        );
        assert_eq!(
            check(&c, &b, 3),
            Some(GuardFail::Loop),
            "a loop end the translator did not know about must not slip through"
        );
        c.loops[0] = frame(c.pc + 9);
        assert_eq!(check(&c, &b, 3), None);
        let arith_loop = crate::core::types::LoopFrame {
            term: Some(0),
            ..frame(c.pc + 9)
        };
        c.loops[0] = arith_loop;
        assert_eq!(check(&c, &finishing, 3), Some(GuardFail::Loop), "T13a loop");
    }

    fn uop(kind: u16, flags: u8) -> Uop {
        Uop {
            h: kind,
            flags,
            ..Uop::default()
        }
    }

    #[test]
    fn the_class_is_the_strongest_need_in_the_order_arith_delayed_cb_loop() {
        let plain = [uop(UopKind::INTERP.0, 0), uop(UopKind::NOP.0, 0)];
        assert_eq!(class_of(&plain), GuardClass::Plain);
        assert_eq!(classify(&plain, false, false, true), GuardClass::Loop);
        let cb = [uop(UopKind::MOVE_BASE, UF_CB)];
        assert_eq!(class_of(&cb), GuardClass::Cb);
        assert_eq!(classify(&cb, false, false, true), GuardClass::Cb);
        assert_eq!(classify(&cb, true, false, true), GuardClass::Delayed);
        assert_eq!(classify(&cb, true, true, true), GuardClass::Arith);
        assert_eq!(classify(&plain, false, true, false), GuardClass::Arith);
        // a uop of the float or fixed-point family is Arith even if the translator did not say
        for base in [UopKind::FLOAT_BASE, UopKind::FIXED_BASE] {
            assert_eq!(class_of(&[uop(base + 3, UF_FIN)]), GuardClass::Arith);
        }
        for base in [UopKind::MOVE_BASE, UopKind::FLOW_BASE] {
            assert_eq!(class_of(&[uop(base + 3, 0)]), GuardClass::Plain);
        }
    }

    #[test]
    fn every_guard_row_has_a_distinct_name() {
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
