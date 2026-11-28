//! OWNER: W1-NCORE. Loops in native traces: the loop header block, the iteration accumulator
//! and the native back-edge of a `LOOP_BACK` uop (design note 4.3 and 9).
//!
//! A trace is a single entry, so the entry block cannot be the target of its own back-edge (it
//! loads the registers once and has no predecessors). [`NCtx::new`] therefore ends the entry
//! block with a jump to a header block when the trace has a `LOOP_BACK` uop, and the body is
//! emitted from the header on; the header stays unsealed until [`NCtx::finish`], when the
//! back-edge is known. Registers and flags are Cranelift variables, so their values flow around
//! the back-edge without a store.
//!
//! The accumulator is the `i64` variable `acc`: `cpu.insns` at entry, advanced by the length of
//! every iteration that jumped back. A path that leaves adds its static `done` to it (see
//! `exits.rs`), so `cpu.insns` is exact whatever iteration the exit was taken in, and the
//! `LOOP_BACK` check is the executor's `left >= acc + n_guest` written on absolute counts:
//! `acc + n_guest <= ctl.limit`. Because an earlier iteration may have written a register after
//! the place a later iteration exits, a trace with a loop starts with every register and flag
//! marked written, so every exit stores all of them (the stores of unchanged values are the
//! values loaded at entry).

use super::super::abi::{CTL_LIMIT, OFF_INSNS};
use super::{acc_var, NCtx, Unsupported, NREGS};
use crate::machine::sb::uop::{k, ExitKind};
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, MemFlags};

impl NCtx<'_> {
    /// Open the loop of a trace that has a `LOOP_BACK` uop: the accumulator, then the header
    /// block, which becomes the current block. No-op for a trace without one.
    pub(super) fn open_loop(&mut self) {
        if !self.sb().uops.iter().any(|u| u.kind == k::LOOP_BACK) {
            return;
        }
        let insns = self
            .b
            .ins()
            .load(types::I64, MemFlags::trusted(), self.cpu, OFF_INSNS as i32);
        self.b.declare_var(acc_var(), types::I64);
        self.b.def_var(acc_var(), insns);
        let header = self.b.create_block();
        self.b.ins().jump(header, &[]);
        self.b.switch_to_block(header);
        self.header = Some(header);
        self.dirty = (1 << NREGS) - 1;
        self.flags_def = 0xF;
    }

    /// True when the trace has a loop header: its exits add their `done` to the accumulator.
    pub fn in_loop(&self) -> bool {
        self.header.is_some()
    }

    /// The native back-edge through the `LoopBack` exit `idx`: add the iteration length
    /// (`done`) to the accumulator and jump to the header while `acc + n_guest <= ctl.limit`,
    /// else leave through the exit with the accumulated count (`cpu.insns += acc`, `r15` the
    /// exit's target). [`Unsupported`] (nothing emitted) when the trace has no loop header or
    /// `idx` is not a `LoopBack` exit.
    pub fn loop_back(&mut self, idx: usize) -> Result<(), Unsupported> {
        let sb = self.sb();
        let e = &sb.exits[idx];
        let Some(header) = self.header else {
            return Err(Unsupported);
        };
        if e.kind != ExitKind::LoopBack {
            return Err(Unsupported);
        }
        let (done, target) = (e.done, e.target);
        let acc = self.b.use_var(acc_var());
        let acc = self.b.ins().iadd_imm(acc, i64::from(done));
        self.b.def_var(acc_var(), acc);
        let need = self.b.ins().iadd_imm(acc, i64::from(sb.n_guest));
        let limit = self
            .b
            .ins()
            .load(types::I64, MemFlags::trusted(), self.ctl, CTL_LIMIT as i32);
        let again = self
            .b
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, need, limit);
        let out = self.b.create_block();
        self.b.ins().brif(again, header, &[], out, &[]);
        self.b.seal_block(out);
        self.b.switch_to_block(out);
        let t = self.const32(target);
        self.leave(idx as u32, 0, t, None);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{assert_fallback, build_and_verify, toy_lower, verified};
    use super::*;
    use crate::cpu::Cpu;
    use crate::machine::sb::cache::tiny_trace;
    use crate::machine::sb::native::available;
    use crate::machine::sb::native::codegen::build_function_with;
    use crate::machine::sb::native::rt::Jit;
    use crate::machine::sb::uop::{Exit, Sb, Uop};
    use crate::mem::Mem;
    use cranelift_codegen::isa::CallConv;

    /// The loop of the tests: `r1 += 1; [guard r1 == 3]; r2 += 2; loop_back`, entered at 0x1000.
    fn loop_sb(guarded: bool) -> Sb {
        let add = |rd: u8, imm: u32, pc: u32| {
            let mut u = Uop::new(k::ADD_IMM, pc);
            (u.rd, u.rn, u.imm) = (rd, rd, imm);
            u
        };
        let mut uops = vec![add(1, 1, 0x1000)];
        if guarded {
            let mut g = Uop::new(k::GUARD, 0x1004);
            (g.rn, g.imm, g.n) = (1, 3, 0);
            uops.push(g);
        }
        uops.push(add(2, 2, 0x1004));
        let mut lb = Uop::new(k::LOOP_BACK, 0x1008);
        (lb.n, lb.rs) = (0, 0);
        uops.push(lb);
        let exits = vec![Exit::new(ExitKind::LoopBack, 2, 0x1004, 0x1000)];
        Sb::from_parts(0x1000, 2, uops, exits, vec![(0x1000, 0)]).unwrap()
    }

    #[test]
    fn a_trace_with_a_loop_back_gets_a_header_an_accumulator_and_stores_everything() {
        let sb = loop_sb(false);
        let func = build_and_verify(&sb, |cx| {
            assert!(cx.in_loop());
            assert_eq!((cx.dirty, cx.flags_def), (0x7FFF, 0xF));
            cx.set_lowerer(toy_lower);
            crate::machine::sb::native::codegen::lower_range_or_exit(cx, 0, 3);
            assert!(cx.is_terminated());
        });
        let text = func.display().to_string();
        assert!(text.contains("block1"), "a header block: {text}");
        assert_eq!(text.matches("brif").count(), 1, "the back-edge: {text}");
        assert_eq!(text.matches("istore8").count(), 4, "all four flags: {text}");
    }

    #[test]
    fn a_trace_without_one_has_no_header_and_loop_back_is_unsupported_and_emits_nothing() {
        let sb = tiny_trace(0x1000, &[(0x1000, 1)]);
        build_and_verify(&sb, |cx| {
            assert!(!cx.in_loop());
            assert_eq!((cx.dirty, cx.flags_def), (0, 0));
            let before = cx.b.func.dfg.num_insts();
            assert_eq!(cx.loop_back(0), Err(Unsupported));
            assert_eq!(cx.b.func.dfg.num_insts(), before);
            cx.exit_generic(0x1000);
        });
    }

    #[test]
    fn loop_back_through_an_exit_that_is_not_a_loop_back_is_unsupported() {
        let mut sb = loop_sb(false);
        sb.exits = vec![Exit::new(ExitKind::Direct, 2, 0x1004, 0x1008)].into_boxed_slice();
        build_and_verify(&sb, |cx| {
            let before = cx.b.func.dfg.num_insts();
            assert_eq!(cx.loop_back(0), Err(Unsupported));
            assert_eq!(cx.b.func.dfg.num_insts(), before);
            cx.exit_generic(0x1000);
        });
    }

    #[test]
    fn the_budget_check_is_the_executors_left_at_least_acc_plus_n_guest() {
        let sb = loop_sb(false);
        let func = build_function_with(&sb, CallConv::SystemV, types::I64, toy_lower).unwrap();
        let text = verified(func).display().to_string();
        assert!(
            text.contains("icmp ule"),
            "unsigned acc + n_guest <= limit: {text}"
        );
        if !available() {
            assert_fallback();
            return;
        }
        let mut jit = Jit::new().unwrap();
        let n = compile_with_toy(&mut jit, &sb).unwrap();
        let (mut counts, mut last) = (std::collections::BTreeSet::new(), 0);
        for budget in 2..=11u64 {
            let mut cpu = Cpu::new();
            cpu.insns = 100;
            // the model: iterate while `acc + n_guest <= limit - insns0`
            let (mut acc, mut iterations) = (0u64, 0u32);
            loop {
                iterations += 1;
                acc += 2;
                if acc + 2 > budget {
                    break;
                }
            }
            let out = jit.run(&n, &mut cpu, &mut Mem::new(), 100 + budget);
            assert_eq!(
                (out.exit, out.target, cpu.r[15], cpu.insns),
                (0, 0x1000, 0x1000, 100 + acc),
                "budget {budget}"
            );
            assert_eq!(
                (cpu.r[1], cpu.r[2]),
                (iterations, 2 * iterations),
                "budget {budget}"
            );
            counts.insert(iterations);
            last = iterations;
        }
        assert!(
            counts.len() >= 4 && last >= 5,
            "iteration counts seen: {counts:?}"
        );
    }

    #[test]
    fn a_side_exit_in_a_later_iteration_counts_the_earlier_ones_and_stores_every_register() {
        let sb = loop_sb(true);
        if !available() {
            assert_fallback();
            return;
        }
        let mut jit = Jit::new().unwrap();
        let n = compile_with_toy(&mut jit, &sb).unwrap();
        let mut cpu = Cpu::new();
        cpu.insns = 50;
        let out = jit.run(&n, &mut cpu, &mut Mem::new(), 1_000_000);
        // iterations 1 and 2 ran whole (r1 = 2, r2 = 4, 4 instructions); the third got as far as
        // the guard (r1 = 3, one instruction) and left for the interpreter at the guard's pc.
        // r2 is written *after* the guard in the body, so storing it needs the loop's
        // all-registers rule.
        assert!(out.is_generic());
        assert_eq!(
            (out.target, cpu.r[15], cpu.r[1], cpu.r[2], cpu.insns),
            (0x1004, 0x1004, 3, 4, 50 + 4 + 1)
        );
    }

    fn compile_with_toy(jit: &mut Jit, sb: &Sb) -> Option<crate::machine::sb::native::rt::Native> {
        let func = build_function_with(sb, jit.call_conv(), jit.ptr_type(), toy_lower)?;
        jit.define_native(func)
    }

    #[test]
    fn exit_through_a_loop_back_exit_is_the_back_edge_and_a_loopless_trace_leaves_plainly() {
        let sb = loop_sb(false);
        let text = build_and_verify(&sb, |cx| {
            assert_eq!(
                cx.exit(0),
                Ok(()),
                "the LoopBack exit of a trace with a loop"
            );
            assert!(cx.is_terminated());
        })
        .display()
        .to_string();
        assert!(text.contains("brif"), "{text}");
        let mut plain = tiny_trace(0x1000, &[(0x1000, 1)]);
        plain.exits = vec![Exit::new(ExitKind::LoopBack, 1, 0x1000, 0x1000)].into_boxed_slice();
        let text = build_and_verify(&plain, |cx| {
            assert_eq!(cx.exit(0), Ok(()));
        })
        .display()
        .to_string();
        assert!(!text.contains("brif"), "no header, no back-edge: {text}");
    }
}
