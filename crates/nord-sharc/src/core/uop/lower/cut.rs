//! Cut rules: where a basic block ends.
//!
//! OWNER: W1-LOWER. Design 4.2: a block ends after any flow instruction (jump, call, return, DO
//! setup whose loop end is outside, IDLE), after any system-register access with side effects
//! (`Sys`), after the last instruction of a statically known loop body (`UF_FIN` terminator),
//! after an `Interp` instruction that can reach `poke_irq`, before `stop_pc` and at
//! `max_block_steps`. `stop_pc` (the pass idle pc) is always a block START, never an interior
//! instruction: the pass loop ends a pass when a step LANDS on it (`pc == idle_pc` after the
//! step), so the block that reaches it must end there, and `uop_try` at `pc == stop_pc` runs
//! nothing. A pc that is not known at translation time is repaired by `uop_begin_pass` (design
//! 11.1, rule P7). A cut may not separate a delayed branch from its owed slots
//! (`BlockBuilder::owed_slots`).
//!
//! Skeleton: cut after every instruction. One-instruction blocks are legal (`min_block_steps` is
//! 1) and need no assumption about neighbouring instructions, so this is the safe fallback.
use super::LowerCtx;
use crate::core::decode_cache::CachedInsn;

/// Whether the block under construction ends after `c`, which is its `steps_in_block`-th step.
pub(in crate::core) fn cuts_after(cx: &LowerCtx<'_>, c: &CachedInsn, steps_in_block: u32) -> bool {
    let _ = (cx, c, steps_in_block);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::uop::config::UopConfig;
    use crate::core::uop::lower::decode::decode_at;

    #[test]
    fn the_skeleton_cuts_after_every_instruction() {
        let cfg = UopConfig::default();
        let cx = LowerCtx {
            cfg: &cfg,
            stop_pc: None,
            simd: false,
            isa: false,
        };
        let mut core = core_with(&[0x0001]);
        let pc = core.pc;
        let c = decode_at(&mut core, pc);
        assert!(cuts_after(&cx, &c, 1) && cuts_after(&cx, &c, cfg.max_block_steps));
    }
}
