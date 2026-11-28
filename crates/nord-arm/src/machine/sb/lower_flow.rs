//! OWNER: W1-FLOW. Control-flow lowering of the trace builder (skeleton: every ender becomes a
//! `GENERIC_END` that leaves through a `Dynamic` exit).
//!
//! Spec (design note 4.2, WP W1-FLOW): every block ender (`Insn::is_ender`) reaches this function.
//! Follow unconditional `B` and `BL` (`BL` emits `MOV_IMM r14, pc + 4`, inline depth up to
//! `opts.inline_depth`; at depth > 0 the callee's `bx lr` becomes `GUARD_REG_EQ r14 == return
//! pc` and the trace continues); follow the predicted side of `Bcc` (static: backward taken) and
//! make the other side a `GUARD` exit; end at an indirect branch (`EXIT_REG` for `bx`,
//! `EXIT_REG_ARM` for `mov pc,rm`), at a generic ender (`GENERIC_END`), or when `can_follow`
//! refuses the target (unmapped or `HOOK_X` page, page cap). Returning to `entry_pc` emits
//! `LOOP_BACK`; returning to another pc already in the trace ends with `EXIT` (no inner loops).
//! Conditional `BL` stays generic. `mrs`/`msr` (SLOW class) may lower to `MRS`/`MSR_C` and
//! `Follow` the next word. Every exit gets its static `done` (instructions retired through this
//! one on the path) and `last_pc`; `exits` never exceeds `MAX_EXITS`.
//!
//! `Flow`/`FlowCtx` are frozen (S0); the function is called by `trace::build`, so land the body
//! here only.

use super::decode::Insn;
use super::trace::BuildOpts;
use super::uop::{Exit, ExitKind, Uop};

/// What the builder does after a control-flow instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Continue the trace at this pc.
    Follow(u32),
    /// The trace ends here (an `EXIT*` or `GENERIC_END` uop was pushed).
    End,
}

/// Context of [`lower_flow`].
pub struct FlowCtx<'a> {
    pub insn: Insn,
    pub uops: &'a mut Vec<Uop>,
    pub exits: &'a mut Vec<Exit>,
    /// Guest instructions retired on the current path before this one.
    pub done: u16,
    /// Static prediction override for a conditional branch (`None`: backward taken).
    pub taken_hint: Option<bool>,
    /// Whether the trace may continue at an address (mapped, unhooked, within the page cap).
    pub can_follow: &'a dyn Fn(u32) -> bool,
    /// Current inline call depth.
    pub depth: &'a mut u8,
    /// Return addresses of the inlined calls.
    pub rets: &'a mut Vec<u32>,
    pub entry_pc: u32,
    pub opts: &'a BuildOpts,
}

/// Lower the control-flow instruction of `cx.insn` (see the module docs).
pub fn lower_flow(cx: &mut FlowCtx<'_>) -> Flow {
    // skeleton: run the instruction through the interpreter and leave through a Dynamic exit
    let exit = cx.exits.len() as u8;
    cx.exits
        .push(Exit::new(ExitKind::Dynamic, cx.done + 1, cx.insn.pc, 0));
    cx.uops
        .push(Uop::generic_end(cx.insn.word, cx.insn.pc, exit));
    Flow::End
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::decode::decode;
    use crate::machine::sb::uop::k;

    #[test]
    fn the_skeleton_ends_the_trace_with_a_generic_end_and_a_dynamic_exit() {
        let opts = BuildOpts::default();
        let (mut uops, mut exits, mut depth, mut rets) = (Vec::new(), Vec::new(), 0u8, Vec::new());
        let can = |_: u32| true;
        let mut cx = FlowCtx {
            insn: decode(0xC000_0010, 0xEAFF_FFFE), // b .
            uops: &mut uops,
            exits: &mut exits,
            done: 6,
            taken_hint: None,
            can_follow: &can,
            depth: &mut depth,
            rets: &mut rets,
            entry_pc: 0xC000_0000,
            opts: &opts,
        };
        assert_eq!(lower_flow(&mut cx), Flow::End);
        assert_eq!(uops.len(), 1);
        assert_eq!(
            (uops[0].kind, uops[0].exit_idx(), uops[0].imm),
            (k::GENERIC_END, 0, 0xEAFF_FFFE)
        );
        assert_eq!(
            (exits[0].kind, exits[0].done, exits[0].last_pc),
            (ExitKind::Dynamic, 7, 0xC000_0010)
        );
    }
}
