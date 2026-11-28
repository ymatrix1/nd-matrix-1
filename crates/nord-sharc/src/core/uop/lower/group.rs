//! Delay groups: which instructions the translator may lower, and which delayed branches it may
//! translate together with their delay slots.
//!
//! OWNER: W1-LOWER. A delayed branch resolves in the `finish_step` of its last delay slot, so the
//! branch and its slots are one group inside one block (rule P1). The interpreter is exact for
//! anything else, so a group that cannot be one block is not translated at all: the block ends
//! before the branch and the branch runs on `Core::step`.
use super::cut::{flow_of, touches_sys, translatable, Flow};
use super::decode::{decode_at, window_hides};
use super::LowerCtx;
use crate::core::decode_cache::CachedInsn;
use crate::core::Core;

/// Whether the decode path may be used at `pc`: not an ISA word, not in an open vector window.
pub(super) fn readable(core: &Core, pc: u32) -> bool {
    !core.isa_at(pc) && !window_hides(core, pc)
}

/// Whether `c` can be lowered into a block bounded by `hi`: translatable, and its parcels end
/// inside the range (the code pages the engine watches).
pub(super) fn fits(core: &Core, c: &CachedInsn, hi: u32) -> bool {
    translatable(c) && c.next_pc > c.pc && c.next_pc <= hi && !core.isa_at(c.pc)
}

/// Whether the delay group of the branch `c` (its `slots` instructions follow it) can be one
/// block: every slot is a plain instruction inside the range that is not `stop_pc` (the pass
/// would have ended there), and the first slot does not load a system register (the second slot
/// still runs in the mode the block was translated for, so nothing may change it before).
pub(super) fn group_fits(
    core: &mut Core,
    cx: &LowerCtx<'_>,
    c: &CachedInsn,
    slots: u8,
    hi: u32,
) -> bool {
    let mut pc = c.next_pc;
    for k in 0..slots {
        if pc >= hi || cx.stop_pc == Some(pc) || !readable(core, pc) {
            return false;
        }
        let s = decode_at(core, pc);
        let plain = flow_of(&s) == Flow::Plain;
        if !fits(core, &s, hi) || !plain || (k == 0 && touches_sys(&s)) {
            return false;
        }
        pc = s.next_pc;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::support::{build_cfg, spans, ALU, JUMP_DB, LOAD_SYS, NOP};
    use super::*;
    use crate::core::support::{core_with, CODE};
    use crate::core::uop::config::UopConfig;

    fn fits_group(program: &[u16], stop_pc: Option<u32>, hi: u32) -> bool {
        let mut core = core_with(program);
        let cfg = UopConfig::default();
        let cx = LowerCtx {
            cfg: &cfg,
            stop_pc,
            simd: false,
            isa: false,
        };
        let branch = decode_at(&mut core, CODE);
        group_fits(&mut core, &cx, &branch, 2, hi)
    }

    #[test]
    fn a_group_needs_two_plain_slots_inside_the_range_that_are_not_stop_pc() {
        let ok = [&JUMP_DB[..], &ALU, &[NOP]].concat();
        let hi = CODE + ok.len() as u32;
        assert!(fits_group(&ok, None, hi));
        assert!(
            !fits_group(&ok, None, hi - 1),
            "the second slot overhangs hi"
        );
        assert!(!fits_group(&ok, None, CODE + 3), "no slot inside the range");
        assert!(
            !fits_group(&ok, Some(CODE + 3), hi),
            "stop_pc on the first slot"
        );
        assert!(
            !fits_group(&ok, Some(CODE + 6), hi),
            "stop_pc on the second slot"
        );
        assert!(
            fits_group(&ok, Some(hi), hi),
            "stop_pc after the group is fine"
        );
        let sys = [&JUMP_DB[..], &LOAD_SYS, &[NOP]].concat();
        assert!(
            !fits_group(&sys, None, CODE + 6),
            "a system load in the first slot"
        );
        let sys2 = [&JUMP_DB[..], &[NOP], &LOAD_SYS].concat();
        assert!(
            fits_group(&sys2, None, CODE + 6),
            "in the second it ends the block anyway"
        );
        let branch = [&JUMP_DB[..], &[0x063E, 0x0000, 0x0010], &[NOP]].concat();
        assert!(!fits_group(&branch, None, CODE + 7), "a branch in a slot");
    }

    #[test]
    fn a_group_that_does_not_fit_is_left_to_the_interpreter() {
        let cfg = UopConfig::default();
        let blocks = |program: &[u16], stop| spans(&build_cfg(program, &cfg, stop), false);
        // the second slot is outside the range: no block holds the branch
        let short = blocks(&[&[NOP][..], &JUMP_DB, &[NOP]].concat(), None);
        assert_eq!(short, [(CODE, 1), (CODE + 4, 1)]);
        // a branch in a slot (the scan does not judge the group: the pc after the slots is a
        // leader all the same)
        let jump = [0x063E, 0x0000, 0x0010];
        let in_slot = blocks(&[&JUMP_DB[..], &jump, &[NOP; 2]].concat(), None);
        assert_eq!(in_slot, [(CODE + 3, 1), (CODE + 6, 1), (CODE + 7, 1)]);
        // a system-register load in the first slot
        let sys = blocks(&[&JUMP_DB[..], &LOAD_SYS, &[NOP; 2]].concat(), None);
        assert_eq!(sys, [(CODE + 3, 1), (CODE + 5, 1), (CODE + 6, 1)]);
        // stop_pc on the second slot: the pass would have ended there
        let program = [&[NOP][..], &JUMP_DB, &[NOP; 3]].concat();
        let on_slot = blocks(&program, Some(CODE + 5));
        assert_eq!(
            on_slot,
            [(CODE, 1), (CODE + 4, 1), (CODE + 5, 1), (CODE + 6, 1)]
        );
        // stop_pc right after the group is fine: the group is a block and the pass ends after it
        assert_eq!(blocks(&program, Some(CODE + 6)), [(CODE, 4), (CODE + 6, 1)]);
    }
}
