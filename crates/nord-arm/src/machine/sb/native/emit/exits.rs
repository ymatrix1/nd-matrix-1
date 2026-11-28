//! OWNER: W1-NCORE. The epilogue half of [`NCtx`]: storing the dirty state, leaving through a
//! static, register-target or generic exit, and the conditional side exit of a memory fast path.
//!
//! In a trace with a loop (`NCtx::in_loop`) the instruction count a leaving path adds to
//! `cpu.insns` is the accumulator of completed iterations plus the static `done` of the path
//! (design note 4.3: `LOOP_BACK` adds the iteration length to a local accumulator); in a trace
//! without one it is `cpu.insns + done`, loaded at the exit.

use super::super::abi::{
    CTL_EXIT, CTL_TARGET, EXIT_GENERIC, OFF_FLAG, OFF_INSNS, OFF_R, OFF_THUMB,
};
use super::{acc_var, flag_var, reg_var, NCtx, Unsupported, NREGS};
use crate::machine::sb::uop::ExitKind;
use cranelift_codegen::ir::{types, InstBuilder, MemFlags, Value};

impl NCtx<'_> {
    /// Store every dirty register and defined flag back into the `Cpu`.
    fn store_state(&mut self) {
        let flags = MemFlags::trusted();
        for n in 0..NREGS {
            if self.dirty >> n & 1 != 0 {
                let v = self.b.use_var(reg_var(n));
                self.b
                    .ins()
                    .store(flags, v, self.cpu, (OFF_R + 4 * n as usize) as i32);
            }
        }
        for i in 0..4u8 {
            if self.flags_def >> i & 1 != 0 {
                let v = self.b.use_var(flag_var(i));
                self.b
                    .ins()
                    .istore8(flags, v, self.cpu, OFF_FLAG[i as usize] as i32);
            }
        }
    }

    /// Leave: state stored, `cpu.thumb` written when `thumb` is given, `insns += done`,
    /// `r15 = ctl.target = target`, `ctl.exit = id`, return `id`.
    pub(super) fn leave(&mut self, id: u32, done: u16, target: Value, thumb: Option<Value>) {
        self.store_state();
        let flags = MemFlags::trusted();
        let base = if self.in_loop() {
            self.b.use_var(acc_var())
        } else {
            self.b
                .ins()
                .load(types::I64, flags, self.cpu, OFF_INSNS as i32)
        };
        let insns = self.b.ins().iadd_imm(base, i64::from(done));
        self.b.ins().store(flags, insns, self.cpu, OFF_INSNS as i32);
        if let Some(bit) = thumb {
            self.b.ins().istore8(flags, bit, self.cpu, OFF_THUMB as i32);
        }
        self.b
            .ins()
            .store(flags, target, self.cpu, (OFF_R + 60) as i32);
        let e = self.const32(id);
        self.b.ins().store(flags, e, self.ctl, CTL_EXIT as i32);
        self.b
            .ins()
            .store(flags, target, self.ctl, CTL_TARGET as i32);
        self.b.ins().return_(&[e]);
    }

    /// Leave through exit `idx` of the trace: a static target, or the register target of an
    /// `Indirect` / `IndirectArm` exit. A `Dynamic` exit (its `GENERIC_END` is never native) and
    /// an indirect exit through `r15` are [`Unsupported`], and nothing is emitted for them. A
    /// `LoopBack` exit of a trace with a loop is the budget check of [`NCtx::loop_back`].
    pub fn exit(&mut self, idx: usize) -> Result<(), Unsupported> {
        let e = &self.sb().exits[idx];
        let (kind, reg, done, target) = (e.kind, e.reg, e.done, e.target);
        let id = idx as u32;
        match kind {
            ExitKind::LoopBack if self.in_loop() => return self.loop_back(idx),
            ExitKind::Direct
            | ExitKind::Fault
            | ExitKind::Stale
            | ExitKind::LoopBack
            | ExitKind::Watch => {
                let t = self.const32(target);
                self.leave(id, done, t, None);
            }
            ExitKind::Indirect if reg < NREGS => {
                let v = self.reg(reg);
                let bit0 = self.b.ins().band_imm(v, 1);
                let bit1 = self.b.ins().ishl_imm(bit0, 1);
                let mask = self.b.ins().bor_imm(bit1, -4);
                let t = self.b.ins().band(v, mask);
                self.leave(id, done, t, Some(bit0));
            }
            ExitKind::IndirectArm if reg < NREGS => {
                let v = self.reg(reg);
                let t = self.b.ins().band_imm(v, -4);
                self.leave(id, done, t, None);
            }
            ExitKind::Indirect | ExitKind::IndirectArm | ExitKind::Dynamic => {
                return Err(Unsupported)
            }
        }
        Ok(())
    }

    /// Leave with the state exact before the instruction at `pc`: the `retired` instructions
    /// before it are counted, `r15 = pc`, the driver runs one `Cpu::step` there.
    pub fn exit_generic(&mut self, pc: u32) {
        let t = self.const32(pc);
        self.leave(EXIT_GENERIC, self.retired, t, None);
    }

    /// `if cond != 0 { exit_generic(pc) }`; emission continues in the fall-through block.
    pub fn side_exit_generic_if(&mut self, cond: Value, pc: u32) {
        let (side, cont) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(cond, side, &[], cont, &[]);
        self.b.seal_block(side);
        self.b.switch_to_block(side);
        self.exit_generic(pc);
        self.b.seal_block(cont);
        self.b.switch_to_block(cont);
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::build_and_verify;
    use crate::machine::sb::cache::tiny_trace;
    use crate::machine::sb::uop::{Exit, ExitKind};

    fn trace(kinds: &[ExitKind]) -> crate::machine::sb::uop::Sb {
        let mut sb = tiny_trace(0x1000, &[(0x1000, 1)]);
        sb.exits = kinds
            .iter()
            .map(|k| Exit::new(*k, 2, 0x1004, 0x1008))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        sb
    }

    #[test]
    fn every_static_exit_kind_stores_the_target_and_returns_its_index() {
        let kinds = [
            ExitKind::Direct,
            ExitKind::Fault,
            ExitKind::Stale,
            ExitKind::LoopBack,
            ExitKind::Watch,
        ];
        let sb = trace(&kinds);
        for (idx, kind) in kinds.iter().enumerate() {
            let text = build_and_verify(&sb, |cx| {
                assert_eq!(cx.exit(idx), Ok(()), "{kind:?}");
                assert!(cx.is_terminated());
            })
            .display()
            .to_string();
            assert_eq!(text.matches("return").count(), 1, "{text}");
            assert!(text.contains(&format!("iconst.i32 {idx}")), "{text}");
            assert!(
                text.contains("iadd_imm") && text.contains(", 2"),
                "done: {text}"
            );
        }
    }

    #[test]
    fn a_generic_exit_leaves_with_the_retired_count_and_the_marker_index() {
        let sb = trace(&[ExitKind::Direct]);
        let text = build_and_verify(&sb, |cx| {
            cx.retired = 5;
            cx.exit_generic(0x1010);
        })
        .display()
        .to_string();
        assert!(text.contains(", 5"), "retired: {text}");
        assert!(text.contains("iconst.i32 -1"), "EXIT_GENERIC: {text}");
    }

    #[test]
    fn a_side_exit_branches_to_a_block_that_leaves_and_emission_resumes() {
        let sb = trace(&[ExitKind::Direct]);
        let text = build_and_verify(&sb, |cx| {
            let c = cx.reg(3);
            cx.side_exit_generic_if(c, 0x1004);
            assert!(!cx.is_terminated());
            cx.exit(0).unwrap();
        })
        .display()
        .to_string();
        assert_eq!(text.matches("brif").count(), 1, "{text}");
        assert_eq!(text.matches("return").count(), 2, "{text}");
    }
}
