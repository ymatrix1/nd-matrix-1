//! OWNER: W1-FLOW. Control-flow lowering of the trace builder (design note 4.2, WP W1-FLOW).
//!
//! Every block ender (`Insn::is_ender`) reaches [`lower_flow`]; `trace::build` drives it and
//! allocates nothing for it. `done` is the number of instructions retired through the ender on
//! the path an exit belongs to (`FlowCtx::done + 1`), `last_pc` the ender's address; both are
//! static (I3). A conditional ender retires on both of its paths.
//!
//! | Ender | Result |
//! |---|---|
//! | `b` | followed with no uop (the builder covers it with `parts::nop`, see below); `LOOP_BACK` when the target is `entry_pc`; `EXIT` when `can_follow` refuses it (unmapped or `HOOK_X` page, page cap, already in the trace) |
//! | `bl` | `MOV_IMM r14, pc + 4`, then as `b`; the callee is inlined while `depth < opts.inline_depth` (the return address goes on `rets`), otherwise `EXIT` |
//! | `bcc` | `GUARD` leaving through the unpredicted side, the trace continues on the predicted one (backward taken unless `taken_hint`); polarity: guard `cond ^ 1` when the taken side is followed, `cond` when the fall-through is |
//! | `bx rm` | `EXIT_REG` (`Indirect` exit, interwork); at `depth > 0` with `rm == r14` an inlined return: `GUARD_REG_EQ r14 == return pc` (its exit is the `Indirect` one), the trace continues at the return pc |
//! | `mov pc, rm` | the same with `EXIT_REG_ARM` and `IndirectArm` (no interwork) |
//! | `bxcc`, `movcc pc,rm` | `GUARD` on the condition leaving through the register, the trace continues at `pc + 4` |
//! | anything else | `GENERIC_END` with a `Dynamic` exit: `ldr pc`, `ldm {.., pc}`, `mov pc` forms other than the plain one, `bl`/`blx` with a condition, `mrs`/`msr` and the rest of the `SLOW` class, `bx pc`, `mov pc, pc` |
//!
//! Caps (`opts`): at `max_guards` guards a conditional ender becomes `GENERIC_END`; the exit
//! table never exceeds `MAX_EXITS` (the builder keeps room for two exits per ender). `bx`/`mov
//! pc` on `r15` read `pc + 8`, which only the interpreter reproduces (I2), so they stay generic.
//!
//! `Uop::n` (S0: summing `n` over the uops up to an exit gives its `done`): the uop that stands for
//! the ender carries `n = 1` (`GUARD`, `EXIT*`, `LOOP_BACK` of an unconditional jump,
//! `GENERIC_END`, the `MOV_IMM` of a `bl`); the terminal after a guard has `n = 0`. The one ender
//! that lowers to no uop is a followed unconditional `b`: `lower_flow` returns `Flow::Follow` and
//! pushes nothing, and `trace::build` then pushes `parts::nop` (`MOV_REG r0, r0`, `n = 1`) for it,
//! so the count stays on a uop of its own. `n` is bookkeeping, never an input of an executor.
//!
//! # Mutation sites (L5, ids `200..=299` of `mutants.rs`)
//!
//! [`FLOW_MUTANTS`] lists the sites of this file and of `trace.rs`; `mutant(id)` is false outside
//! a test build. Each is caught by `lower_flow/kills.rs`, which re-runs the suites under it.
//!
//! `Flow`/`FlowCtx` are frozen (S0); the function is called by `trace::build`.

use super::decode::Insn;
use super::trace::BuildOpts;
use super::uop::{k, Exit, ExitKind, Uop, MAX_EXITS};
use crate::cpu::predecode::op;

#[cfg(test)]
mod cases;
#[cfg(test)]
mod kills;

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

/// `(id, name)` of the mutation sites of `lower_flow.rs` and `trace.rs` (block `200..=299`).
#[cfg(test)]
pub(crate) const FLOW_MUTANTS: &[(u32, &str)] = &[
    (200, "flow: guard polarity of a followed taken branch"),
    (201, "flow: guard polarity of a followed fall-through"),
    (202, "flow: bl link address off by one word"),
    (203, "flow: exit done off by one"),
    (204, "flow: exit last_pc is the next word"),
    (205, "flow: inlined return compares the wrong address"),
    (206, "flow: bx exit without interworking"),
    (207, "flow: mov pc exit with interworking"),
    (208, "flow: inlined return keeps the call depth"),
    (209, "flow: guard cap off by one"),
    (210, "trace: stale early exit target is the store itself"),
    (211, "trace: n_guest one short"),
    (212, "trace: no early exit on stores"),
    (213, "trace: words snapshot of an instruction is wrong"),
    (214, "trace: a flipped prediction is ignored"),
    (215, "trace: literal fold registers no word"),
    (216, "trace: a followed branch has no uop to count it"),
];

/// Mutation site: true only inside `mutants::with(id, ..)` of a test build.
#[cfg(test)]
pub(super) fn mutant(id: u32) -> bool {
    super::mutants::is_active(id)
}

/// Mutation site: false outside a test build.
#[cfg(not(test))]
pub(super) const fn mutant(_: u32) -> bool {
    false
}

/// Lower the control-flow instruction of `cx.insn` (see the module docs).
pub fn lower_flow(cx: &mut FlowCtx<'_>) -> Flow {
    if cx.exits.len() + 2 > MAX_EXITS {
        return generic_end(cx); // no room for a guard and a terminal
    }
    let word = cx.insn.word;
    match cx.insn.entry.op {
        op::B => lower_b(cx),
        op::BL => lower_bl(cx),
        op::BX => lower_indirect(cx, k::EXIT_REG, ExitKind::Indirect),
        id if id >= op::DP_BASE && is_mov_pc_reg(word) => {
            lower_indirect(cx, k::EXIT_REG_ARM, ExitKind::IndirectArm)
        }
        _ => generic_end(cx),
    }
}

/// `mov pc, rm` (no shift, no S): the only data-processing ender with a static shape. The `Rn`
/// field is ignored by `MOV`, as in the interpreter.
fn is_mov_pc_reg(word: u32) -> bool {
    word & 0x0FF0_FFF0 == 0x01A0_F000
}

fn guards(uops: &[Uop]) -> usize {
    uops.iter()
        .filter(|u| matches!(u.kind, k::GUARD | k::GUARD_REG_EQ))
        .count()
}

fn guard_cap(cx: &FlowCtx<'_>) -> usize {
    cx.opts.max_guards + usize::from(mutant(209))
}

/// Allocate an exit of the current ender: `done` through this instruction, `last_pc` its address.
fn push_exit(cx: &mut FlowCtx<'_>, kind: ExitKind, reg: u8, target: u32) -> u8 {
    let idx = cx.exits.len() as u8;
    let done = cx.done + u16::from(!mutant(203));
    let last_pc = if mutant(204) {
        cx.insn.pc.wrapping_add(4)
    } else {
        cx.insn.pc
    };
    let mut e = Exit::new(kind, done, last_pc, target);
    e.reg = reg;
    cx.exits.push(e);
    idx
}

/// A control uop of the current ender leaving through exit `idx`.
fn control(cx: &FlowCtx<'_>, kind: u8, idx: u8, n: u8) -> Uop {
    let mut u = Uop::new(kind, cx.insn.pc);
    u.n = n;
    u.rs = idx;
    u
}

fn generic_end(cx: &mut FlowCtx<'_>) -> Flow {
    let idx = push_exit(cx, ExitKind::Dynamic, 0, 0);
    cx.uops
        .push(Uop::generic_end(cx.insn.word, cx.insn.pc, idx));
    Flow::End
}

/// The trace goes on at `target`: loop back at the entry, follow it when allowed, otherwise leave
/// through a `Direct` exit. `n` is the instruction count the terminal uop carries.
fn continue_at(cx: &mut FlowCtx<'_>, target: u32, n: u8, follow: bool) -> Flow {
    if target == cx.entry_pc {
        let idx = push_exit(cx, ExitKind::LoopBack, 0, target);
        cx.uops.push(control(cx, k::LOOP_BACK, idx, n));
        Flow::End
    } else if follow && (cx.can_follow)(target) {
        Flow::Follow(target)
    } else {
        let idx = push_exit(cx, ExitKind::Direct, 0, target);
        cx.uops.push(control(cx, k::EXIT, idx, n));
        Flow::End
    }
}

fn lower_b(cx: &mut FlowCtx<'_>) -> Flow {
    let (pc, target) = (cx.insn.pc, cx.insn.branch_target());
    if !cx.insn.is_cond() {
        return continue_at(cx, target, 1, true);
    }
    if guards(cx.uops) >= guard_cap(cx) {
        return generic_end(cx);
    }
    let (cond, fall) = (cx.insn.cond(), pc.wrapping_add(4));
    let taken = cx.taken_hint.unwrap_or(target <= pc);
    // the guard leaves through the side the trace does not follow
    let (follow, other, gcond) = if taken {
        (target, fall, if mutant(200) { cond } else { cond ^ 1 })
    } else {
        (fall, target, if mutant(201) { cond ^ 1 } else { cond })
    };
    let idx = push_exit(cx, ExitKind::Direct, 0, other);
    let mut g = control(cx, k::GUARD, idx, 1);
    g.cond = gcond;
    cx.uops.push(g);
    continue_at(cx, follow, 0, true)
}

fn lower_bl(cx: &mut FlowCtx<'_>) -> Flow {
    if cx.insn.is_cond() {
        return generic_end(cx); // a conditional call stays with the interpreter
    }
    let (pc, target) = (cx.insn.pc, cx.insn.branch_target());
    let ret = pc.wrapping_add(4);
    let mut link = Uop::new(k::MOV_IMM, pc);
    link.rd = 14;
    link.imm = if mutant(202) {
        ret.wrapping_add(4)
    } else {
        ret
    };
    cx.uops.push(link);
    let inline = target != cx.entry_pc && *cx.depth < cx.opts.inline_depth;
    if inline && (cx.can_follow)(target) {
        cx.rets.push(ret);
        *cx.depth += 1;
        return Flow::Follow(target);
    }
    continue_at(cx, target, 0, false)
}

/// `bx rm` (`kind = EXIT_REG`, `Indirect`) and `mov pc, rm` (`EXIT_REG_ARM`, `IndirectArm`).
fn lower_indirect(cx: &mut FlowCtx<'_>, kind: u8, exit_kind: ExitKind) -> Flow {
    let rm = cx.insn.rm();
    if rm == 15 {
        return generic_end(cx);
    }
    let exit_kind = match (exit_kind, mutant(206), mutant(207)) {
        (ExitKind::Indirect, true, _) => ExitKind::IndirectArm,
        (ExitKind::IndirectArm, _, true) => ExitKind::Indirect,
        (other, ..) => other,
    };
    let can_guard = guards(cx.uops) < guard_cap(cx);
    if cx.insn.is_cond() {
        if !can_guard {
            return generic_end(cx);
        }
        let idx = push_exit(cx, exit_kind, rm, 0);
        let mut g = control(cx, k::GUARD, idx, 1);
        g.cond = cx.insn.cond();
        cx.uops.push(g);
        let fall = cx.insn.pc.wrapping_add(4);
        return continue_at(cx, fall, 0, true);
    }
    if rm == 14 && can_guard && *cx.depth > 0 {
        if let Some(ret) = cx.rets.pop() {
            if !mutant(208) {
                *cx.depth -= 1;
            }
            let idx = push_exit(cx, exit_kind, 14, 0);
            let mut g = control(cx, k::GUARD_REG_EQ, idx, 1);
            g.rn = 14;
            g.imm = if mutant(205) {
                ret.wrapping_add(4)
            } else {
                ret
            };
            cx.uops.push(g);
            return continue_at(cx, ret, 0, true);
        }
    }
    let idx = push_exit(cx, exit_kind, rm, 0);
    let mut u = control(cx, kind, idx, 1);
    u.rn = rm;
    cx.uops.push(u);
    Flow::End
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::decode::decode;

    #[test]
    fn an_unrecognised_ender_ends_the_trace_with_a_generic_end_and_a_dynamic_exit() {
        let opts = BuildOpts::default();
        let (mut uops, mut exits, mut depth, mut rets) = (Vec::new(), Vec::new(), 0u8, Vec::new());
        let can = |_: u32| true;
        let mut cx = FlowCtx {
            insn: decode(0xC000_0010, 0xE591_F000), // ldr pc,[r1]
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
            (k::GENERIC_END, 0, 0xE591_F000)
        );
        assert_eq!(
            (exits[0].kind, exits[0].done, exits[0].last_pc),
            (ExitKind::Dynamic, 7, 0xC000_0010)
        );
    }

    #[test]
    fn the_mov_pc_pattern_is_exactly_the_plain_register_move() {
        assert!(is_mov_pc_reg(0xE1A0_F00E)); // mov pc,lr
        assert!(is_mov_pc_reg(0x01A0_F003)); // moveq pc,r3
        assert!(is_mov_pc_reg(0xE1A0_F00E | 5 << 16)); // Rn is ignored by MOV
        assert!(!is_mov_pc_reg(0xE1B0_F00E), "movs pc,lr restores the CPSR");
        assert!(!is_mov_pc_reg(0xE1A0_F10E), "a shifted operand");
        assert!(!is_mov_pc_reg(0xE3A0_F001), "an immediate operand");
        assert!(!is_mov_pc_reg(0xE1A0_000E), "rd is not pc");
        assert!(!is_mov_pc_reg(0xE080_F00E), "add pc,r0,lr");
    }

    #[test]
    fn the_mutation_sites_are_off_outside_a_mutant_run() {
        assert!((200..=216).all(|id| !mutant(id)));
        assert_eq!(FLOW_MUTANTS.len(), 17);
        assert!(FLOW_MUTANTS.iter().all(|(id, _)| (200..=299).contains(id)));
    }
}
