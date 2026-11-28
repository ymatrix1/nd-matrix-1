//! OWNER: W1-NCORE. What a control-flow lowerer needs to see of its trace: the uop list and its
//! own position, a dry run of a body (`body_lowerable`) and the predicated block
//! (`predicated`) with its join.
//!
//! `codegen::build_function` walks the uops and tells the context the index of each one
//! (`begin`); a lowerer that also lowers the uops after its own (a `PRED` and its body) says how
//! many it took (`consume`, done by `predicated`) and the walk continues after them. The
//! design's obligation "a `PRED` is native only when its whole body is" is `body_lowerable`
//! (lower the body into a scratch function, emit nothing here) followed by `predicated`. If a
//! body uop fails after the dry run said yes (a lowerer whose answer depends on emission
//! state, which its contract forbids), the body block ends with a generic exit at that uop and
//! the join block is still reachable through the skip edge, so the function stays valid and
//! exact (risk R19: a join block left empty and unterminated).

use super::super::codegen::{lower_range, lower_range_or_exit};
use super::{NCtx, Unsupported};
use crate::machine::sb::uop::Uop;
use cranelift_codegen::ir::{Function, InstBuilder, UserFuncName, Value};
use cranelift_frontend::FunctionBuilderContext;

impl<'a> NCtx<'a> {
    /// Every uop of the trace.
    pub fn uops(&self) -> &'a [Uop] {
        &self.sb().uops
    }

    /// Index of the uop being lowered.
    pub fn index(&self) -> usize {
        self.pos
    }

    /// The uops after the one being lowered (a `PRED` finds its body at the start of it).
    pub fn rest(&self) -> &'a [Uop] {
        &self.uops()[self.pos + 1..]
    }

    /// Start lowering the uop at `index` (called by the walk before each lowerer).
    pub fn begin(&mut self, index: usize) {
        self.pos = index;
        self.consumed = 0;
    }

    /// The current lowerer also lowered the next `count` uops: the walk skips them.
    pub fn consume(&mut self, count: usize) {
        self.consumed += count;
    }

    /// Index of the uop the walk lowers next.
    pub fn next_index(&self) -> usize {
        self.pos + 1 + self.consumed
    }

    /// Whether every one of the `len` uops after the current one lowers natively. The answer is
    /// found by lowering them into a scratch function (this context is untouched), so it is
    /// exactly what `predicated` will get, given that a lowerer's `Ok`/`Err` depends only on the
    /// uop and the trace.
    pub fn body_lowerable(&self, len: usize) -> bool {
        let from = self.pos + 1;
        if self.uops().len() < from + len {
            return false;
        }
        let mut func =
            Function::with_name_signature(UserFuncName::default(), self.b.func.signature.clone());
        let mut fctx = FunctionBuilderContext::new();
        let mut scratch = NCtx::new(&mut func, &mut fctx, self.sb());
        scratch.lower = self.lower;
        lower_range(&mut scratch, from, from + len).is_ok()
    }

    /// Emit `if cond != 0 { the next len uops }` and consume them: a branch on the value `cond`
    /// computes, the body, and a join block where emission goes on. [`Unsupported`], with
    /// nothing emitted (not even `cond`'s code), when `body_lowerable(len)` is false. The
    /// instructions the body retires are counted on the join path whether or not the body ran
    /// (a conditional instruction whose condition fails still retires, design note 4.4).
    pub fn predicated(
        &mut self,
        len: usize,
        cond: impl FnOnce(&mut Self) -> Value,
    ) -> Result<(), Unsupported> {
        if !self.body_lowerable(len) {
            return Err(Unsupported);
        }
        let (pos, consumed, start) = (self.pos, self.consumed, self.retired);
        let from = pos + 1;
        let total: u16 = self.uops()[from..from + len]
            .iter()
            .map(|u| u16::from(u.n))
            .sum();
        let run = cond(self);
        let (body, join) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(run, body, &[], join, &[]);
        self.b.seal_block(body);
        self.b.switch_to_block(body);
        lower_range_or_exit(self, from, from + len);
        if !self.is_terminated() {
            self.b.ins().jump(join, &[]);
        }
        self.b.seal_block(join);
        self.b.switch_to_block(join);
        self.pos = pos;
        self.consumed = consumed + len;
        self.retired = start + total;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{assert_fallback, toy_lower, verified};
    use super::*;
    use crate::cpu::Cpu;
    use crate::machine::sb::native::available;
    use crate::machine::sb::native::codegen::build_function_with;
    use crate::machine::sb::native::rt::{Jit, Native};
    use crate::machine::sb::uop::{k, Exit, ExitKind, Sb};
    use crate::mem::Mem;
    use cranelift_codegen::ir::types;
    use cranelift_codegen::isa::CallConv;
    use std::cell::{Cell, RefCell};

    fn add(rd: u8, imm: u32, pc: u32) -> Uop {
        let mut u = Uop::new(k::ADD_IMM, pc);
        (u.rd, u.rn, u.imm) = (rd, rd, imm);
        u
    }

    /// `if r[rn] != 0 { next len uops }`, `n = 0` like the real `PRED`.
    fn pred(rn: u8, len: u32, pc: u32) -> Uop {
        let mut u = Uop::new(k::PRED, pc);
        (u.rn, u.imm, u.n) = (rn, len, 0);
        u
    }

    fn trace(uops: Vec<Uop>, done: u16) -> Sb {
        let exit = Exit::new(ExitKind::Direct, done, 0x1000 + 4 * u32::from(done), 0x2000);
        Sb::from_parts(0x1000, done, uops, vec![exit], vec![(0x1000, 0)]).unwrap()
    }

    fn native(jit: &mut Jit, sb: &Sb, lower: super::super::LowerFn) -> Native {
        let func = build_function_with(sb, jit.call_conv(), jit.ptr_type(), lower).unwrap();
        jit.define_native(verified(func)).unwrap()
    }

    /// Run `n` with `regs` set, `insns = 10`; `(exit, is_generic, target, regs, insns)`.
    fn run(jit: &Jit, n: &Native, regs: &[(usize, u32)]) -> (u32, bool, u32, [u32; 4], u64) {
        let mut cpu = Cpu::new();
        for (r, v) in regs {
            cpu.r[*r] = *v;
        }
        cpu.insns = 10;
        let out = jit.run(n, &mut cpu, &mut Mem::new(), 1000);
        let regs = [cpu.r[0], cpu.r[1], cpu.r[2], cpu.r[3]];
        (out.exit, out.is_generic(), out.target, regs, cpu.insns)
    }

    thread_local! {
        static SEEN: RefCell<Vec<(usize, usize, usize, usize)>> = const { RefCell::new(Vec::new()) };
        static CALLS: Cell<u32> = const { Cell::new(0) };
    }

    fn recording(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
        SEEN.with(|s| {
            s.borrow_mut().push((
                cx.index(),
                cx.uops().len(),
                cx.rest().len(),
                cx.next_index(),
            ));
        });
        toy_lower(cx, u)
    }

    /// Lowers like the toy, except that `r2 += 0x666` fails from its second lowering on: the
    /// dry run says yes, the real lowering says no.
    fn flaky(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
        if u.kind == k::ADD_IMM && u.imm == 0x666 {
            let calls = CALLS.with(|c| {
                c.set(c.get() + 1);
                c.get()
            });
            if calls >= 2 {
                return Err(Unsupported);
            }
        }
        toy_lower(cx, u)
    }

    #[test]
    fn each_lowerer_sees_the_trace_its_index_and_the_uops_after_it() {
        SEEN.with(|s| s.borrow_mut().clear());
        let sb = trace(
            vec![
                add(1, 1, 0x1000),
                pred(0, 1, 0x1004),
                add(2, 1, 0x1004),
                Uop::exit(0, 0x1008),
            ],
            2,
        );
        build_function_with(&sb, CallConv::SystemV, types::I64, recording).unwrap();
        let seen = SEEN.with(|s| s.borrow().clone());
        // the dry run of the PRED's body is a second visit of uop 2 (index 2, 1 uop after it)
        assert_eq!(
            seen,
            vec![
                (0, 4, 3, 1),
                (1, 4, 2, 2),
                (2, 4, 1, 3),
                (2, 4, 1, 3),
                (3, 4, 0, 4)
            ]
        );
    }

    #[test]
    fn a_predicated_body_runs_only_when_the_condition_holds_and_both_paths_join() {
        let sb = trace(
            vec![
                pred(0, 2, 0x1000),
                add(1, 10, 0x1000),
                add(2, 20, 0x1004),
                Uop::exit(0, 0x1008),
            ],
            2,
        );
        let func = build_function_with(&sb, CallConv::SystemV, types::I64, toy_lower).unwrap();
        verified(func.clone());
        let text = func.display().to_string();
        assert_eq!(text.matches("brif").count(), 1, "{text}");
        if !available() {
            assert_fallback();
            return;
        }
        let mut jit = Jit::new().unwrap();
        let n = native(&mut jit, &sb, toy_lower);
        // exit 0, not generic, target 0x2000, both paths retire the 2 instructions
        assert_eq!(
            run(&jit, &n, &[(0, 0)]),
            (0, false, 0x2000, [0, 0, 0, 0], 12)
        );
        assert_eq!(
            run(&jit, &n, &[(0, 7)]),
            (0, false, 0x2000, [7, 10, 20, 0], 12)
        );
    }

    #[test]
    fn nested_predicates_lower_each_body_uop_once() {
        // r0 guards [add r1, PRED r3 [add r2]]: a body walk that ignored the inner PRED's
        // `consume` would lower `add r2` twice (r2 += 2)
        let sb = trace(
            vec![
                pred(0, 3, 0x1000),
                add(1, 1, 0x1000),
                pred(3, 1, 0x1004),
                add(2, 1, 0x1004),
                Uop::exit(0, 0x1008),
            ],
            2,
        );
        if !available() {
            assert_fallback();
            return;
        }
        let mut jit = Jit::new().unwrap();
        let n = native(&mut jit, &sb, toy_lower);
        let mut seen = std::collections::BTreeSet::new();
        for (r0, r3) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            let want = [r0, u32::from(r0 != 0), u32::from(r0 != 0 && r3 != 0), r3];
            let got = run(&jit, &n, &[(0, r0), (3, r3)]);
            assert_eq!(
                (got.0, got.1, got.3, got.4),
                (0, false, want, 12),
                "{r0} {r3}"
            );
            seen.insert(want);
        }
        assert_eq!(seen.len(), 4, "every combination was distinct");
    }

    #[test]
    fn a_body_that_cannot_be_lowered_makes_the_pred_unsupported_and_emits_nothing() {
        let mut ldr = Uop::new(k::LDR_IMM, 0x1004);
        ldr.rd = 3;
        let sb = trace(
            vec![
                add(1, 5, 0x1000),
                pred(0, 2, 0x1004),
                add(2, 1, 0x1004),
                ldr,
                Uop::exit(0, 0x1008),
            ],
            3,
        );
        crate::machine::sb::native::emit::testing::build_and_verify(&sb, |cx| {
            cx.set_lowerer(toy_lower);
            cx.begin(1);
            assert!(!cx.body_lowerable(2), "the load is not lowerable");
            assert!(cx.body_lowerable(1), "the add alone is");
            assert!(!cx.body_lowerable(9), "past the end of the trace");
            let before = cx.b.func.dfg.num_insts();
            let called = Cell::new(false);
            let r = cx.predicated(2, |cx| {
                called.set(true);
                cx.reg(0)
            });
            assert_eq!(r, Err(Unsupported));
            assert!(!called.get(), "the condition is not even computed");
            assert_eq!(cx.b.func.dfg.num_insts(), before, "nothing was emitted");
            cx.exit_generic(0x1004);
        });
        // through the walk: native up to the PRED, then a generic exit at the PRED's pc
        let func = build_function_with(&sb, CallConv::SystemV, types::I64, toy_lower).unwrap();
        verified(func.clone());
        if !available() {
            assert_fallback();
            return;
        }
        let mut jit = Jit::new().unwrap();
        let n = native(&mut jit, &sb, toy_lower);
        assert_eq!(
            run(&jit, &n, &[(0, 1)]),
            (u32::MAX, true, 0x1004, [1, 5, 0, 0], 11),
            "add r1 ran, one instruction retired, the PRED is for the interpreter"
        );
    }

    #[test]
    fn a_body_uop_that_fails_after_the_dry_run_ends_its_block_and_the_join_stays_valid() {
        CALLS.with(|c| c.set(0));
        let sb = trace(
            vec![
                pred(0, 2, 0x1000),
                add(1, 1, 0x1000),
                add(2, 0x666, 0x1004),
                Uop::exit(0, 0x1008),
            ],
            2,
        );
        let func = build_function_with(&sb, CallConv::SystemV, types::I64, flaky).unwrap();
        verified(func.clone());
        assert_eq!(
            CALLS.with(Cell::get),
            2,
            "once in the dry run, once for real"
        );
        CALLS.with(|c| c.set(0));
        if !available() {
            assert_fallback();
            return;
        }
        let mut jit = Jit::new().unwrap();
        let n = native(&mut jit, &sb, flaky);
        // condition false: the skip edge reaches the join and the exit
        assert_eq!(
            run(&jit, &n, &[(0, 0)]),
            (0, false, 0x2000, [0, 0, 0, 0], 12)
        );
        // condition true: the first add ran (1 retired), the second leaves for the interpreter
        assert_eq!(
            run(&jit, &n, &[(0, 3)]),
            (u32::MAX, true, 0x1004, [3, 1, 0, 0], 11)
        );
    }
}
