//! OWNER: W1-NCORE. The Cranelift emission context handed to the native lowerers.
//!
//! Spec (design note 9, WP W1-NCORE): [`NCtx`] owns the `FunctionBuilder` of one trace and gives
//! the lowerers a tiny vocabulary: `reg(n)` / `set_reg(n, v)` for `r0..r14` and `flag(i)` /
//! `set_flag(i, v)` for the four flags (`i` is the bit of the flag nibble: 3 N, 2 Z, 1 C, 0 V),
//! all backed by Cranelift `Variable`s loaded at entry. `r15` is never a variable (I2): lowerers
//! fold the pc into constants. The dirty masks record what was written, so an exit stores
//! back only the registers and flags that changed. `exit(idx)` leaves through a static exit of
//! the trace (applies `insns += done` and `r15 = target`, stores state, returns the index),
//! `exit_generic(pc)` leaves with the state exact before the instruction at `pc` (the driver runs
//! one `Cpu::step` there) and `side_exit_generic_if(cond, pc)` is the conditional form a
//! memory fast path uses. Registers are 32 bit, flags are `i32` holding 0 or 1.
//!
//! Unsupported shapes return [`Unsupported`]; codegen then ends the native trace with
//! `exit_generic`, so a missing native lowering costs speed, never exactness (I8).

use super::abi::{CTL_EXIT, CTL_TARGET, EXIT_GENERIC, OFF_FLAG, OFF_INSNS, OFF_R};
use crate::machine::sb::uop::{Exit, ExitKind, Sb};
use cranelift_codegen::ir::{types, Function, InstBuilder, MemFlags, Type, Value};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};

/// A uop shape the native lowerer does not handle (the trace falls back to a `Generic` exit).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unsupported;

const NREGS: u8 = 15;

fn reg_var(n: u8) -> Variable {
    Variable::from_u32(u32::from(n))
}

fn flag_var(i: u8) -> Variable {
    Variable::from_u32(u32::from(NREGS) + u32::from(i))
}

/// Emission state of one trace.
pub struct NCtx<'a> {
    pub b: FunctionBuilder<'a>,
    /// The `*mut Cpu` argument.
    pub cpu: Value,
    /// The `*mut NativeCtl` argument.
    pub ctl: Value,
    /// Bit `n` set: `r[n]` was written on the path emitted so far.
    pub dirty: u16,
    /// Bit `i` set: flag `i` (nibble bit) was written.
    pub flags_def: u8,
    /// Guest instructions retired on the path emitted so far (static, I3).
    pub retired: u16,
    exits: Vec<(ExitKind, u16, u32)>,
}

impl<'a> NCtx<'a> {
    /// Start the function of `sb` in `func`: entry block, the 15 registers and 4 flags loaded
    /// into variables. `func` must have the signature `(ptr, ptr) -> i32`.
    pub fn new(func: &'a mut Function, fctx: &'a mut FunctionBuilderContext, sb: &Sb) -> Self {
        let mut b = FunctionBuilder::new(func, fctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        b.seal_block(entry);
        let (cpu, ctl) = (b.block_params(entry)[0], b.block_params(entry)[1]);
        let flags = MemFlags::trusted();
        for n in 0..NREGS {
            b.declare_var(reg_var(n), types::I32);
            let v = b
                .ins()
                .load(types::I32, flags, cpu, (OFF_R + 4 * n as usize) as i32);
            b.def_var(reg_var(n), v);
        }
        for i in 0..4u8 {
            b.declare_var(flag_var(i), types::I32);
            let v = b
                .ins()
                .uload8(types::I32, flags, cpu, OFF_FLAG[i as usize] as i32);
            b.def_var(flag_var(i), v);
        }
        let exits = sb
            .exits
            .iter()
            .map(|e: &Exit| (e.kind, e.done, e.target))
            .collect();
        Self {
            b,
            cpu,
            ctl,
            dirty: 0,
            flags_def: 0,
            retired: 0,
            exits,
        }
    }

    /// The pointer type of the function (the `cpu` argument's type).
    pub fn ptr(&self) -> Type {
        self.b.func.dfg.value_type(self.cpu)
    }

    /// Current value of `r[n]`, `n < 15`.
    pub fn reg(&mut self, n: u8) -> Value {
        assert!(n < NREGS, "r15 is never a variable (I2)");
        self.b.use_var(reg_var(n))
    }

    /// Write `r[n]`, `n < 15`.
    pub fn set_reg(&mut self, n: u8, v: Value) {
        assert!(n < NREGS, "r15 is never a variable (I2)");
        self.b.def_var(reg_var(n), v);
        self.dirty |= 1 << n;
    }

    /// Current value (0 or 1, `i32`) of flag `i`.
    pub fn flag(&mut self, i: u8) -> Value {
        self.b.use_var(flag_var(i))
    }

    /// Write flag `i` (`v` is an `i32` holding 0 or 1).
    pub fn set_flag(&mut self, i: u8, v: Value) {
        self.b.def_var(flag_var(i), v);
        self.flags_def |= 1 << i;
    }

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

    /// Leave: state stored, `insns += done`, `r15 = target`, `ctl.exit/target` set, return `id`.
    fn leave(&mut self, id: u32, done: u16, target: u32) {
        self.store_state();
        let flags = MemFlags::trusted();
        let insns = self
            .b
            .ins()
            .load(types::I64, flags, self.cpu, OFF_INSNS as i32);
        let insns = self.b.ins().iadd_imm(insns, i64::from(done));
        self.b.ins().store(flags, insns, self.cpu, OFF_INSNS as i32);
        let t = self.b.ins().iconst(types::I32, i64::from(target));
        self.b.ins().store(flags, t, self.cpu, (OFF_R + 60) as i32);
        let e = self.b.ins().iconst(types::I32, i64::from(id));
        self.b.ins().store(flags, e, self.ctl, CTL_EXIT as i32);
        self.b.ins().store(flags, t, self.ctl, CTL_TARGET as i32);
        self.b.ins().return_(&[e]);
    }

    /// Leave through exit `idx` of the trace. Only static targets are handled here; an
    /// `Indirect*` or `Dynamic` exit is [`Unsupported`] until the native register-target work.
    pub fn exit(&mut self, idx: usize) -> Result<(), Unsupported> {
        let (kind, done, target) = self.exits[idx];
        match kind {
            ExitKind::Direct
            | ExitKind::Fault
            | ExitKind::Stale
            | ExitKind::LoopBack
            | ExitKind::Watch => {
                self.leave(idx as u32, done, target);
                Ok(())
            }
            ExitKind::Indirect | ExitKind::IndirectArm | ExitKind::Dynamic => Err(Unsupported),
        }
    }

    /// Leave with the state exact before the instruction at `pc`: the `retired` instructions
    /// before it are counted, `r15 = pc`, the driver runs one `Cpu::step` there.
    pub fn exit_generic(&mut self, pc: u32) {
        self.leave(EXIT_GENERIC, self.retired, pc);
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

    /// True when the current block already ends in a terminator (a lowered exit): nothing may be
    /// emitted after it. (`FunctionBuilder::is_filled` is private in cranelift-frontend 0.116.)
    pub fn is_terminated(&self) -> bool {
        let Some(block) = self.b.current_block() else {
            return true;
        };
        self.b
            .func
            .layout
            .last_inst(block)
            .is_some_and(|inst| self.b.func.dfg.insts[inst].opcode().is_terminator())
    }

    /// Finish the function (all blocks sealed, builder finalised).
    pub fn finish(mut self) {
        self.b.seal_all_blocks();
        self.b.finalize();
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use cranelift_codegen::ir::{AbiParam, Signature, UserFuncName};
    use cranelift_codegen::isa::CallConv;
    use cranelift_codegen::settings;
    use cranelift_codegen::verify_function;

    /// A function with the native signature `(i64, i64) -> i32`.
    pub fn new_function() -> Function {
        let mut sig = Signature::new(CallConv::SystemV);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        Function::with_name_signature(UserFuncName::default(), sig)
    }

    /// Build a function for `sb` with `emit`, finish it and verify the IR (no executable memory).
    pub fn build_and_verify(sb: &Sb, emit: impl FnOnce(&mut NCtx<'_>)) -> Function {
        let mut func = new_function();
        let mut fctx = FunctionBuilderContext::new();
        let mut cx = NCtx::new(&mut func, &mut fctx, sb);
        emit(&mut cx);
        cx.finish();
        let flags = settings::Flags::new(settings::builder());
        if let Err(e) = verify_function(&func, &flags) {
            panic!("{e}\n{func}");
        }
        func
    }
}

#[cfg(test)]
mod tests {
    use super::testing::build_and_verify;
    use super::*;
    use crate::machine::sb::cache::tiny_trace;

    fn trace() -> Sb {
        let mut sb = tiny_trace(0x1000, &[(0x1000, 1)]);
        sb.exits = vec![
            Exit::new(ExitKind::Direct, 2, 0x1004, 0x1008),
            Exit::new(ExitKind::Dynamic, 1, 0x1000, 0),
        ]
        .into_boxed_slice();
        sb
    }

    #[test]
    fn variables_registers_flags_and_a_static_exit_verify() {
        let sb = trace();
        let func = build_and_verify(&sb, |cx| {
            let a = cx.reg(0);
            let one = cx.b.ins().iconst(types::I32, 1);
            let sum = cx.b.ins().iadd(a, one);
            cx.set_reg(1, sum);
            let z =
                cx.b.ins()
                    .icmp_imm(cranelift_codegen::ir::condcodes::IntCC::Equal, sum, 0);
            let z = cx.b.ins().uextend(types::I32, z);
            cx.set_flag(2, z);
            assert_eq!((cx.dirty, cx.flags_def), (0b10, 0b100));
            assert_eq!(cx.exit(0), Ok(()));
        });
        let text = func.display().to_string();
        assert!(text.contains("store") && text.contains("return"), "{text}");
        assert_eq!(
            text.matches("istore8").count(),
            1,
            "only the defined flag is stored"
        );
    }

    #[test]
    fn a_block_is_terminated_exactly_after_an_exit() {
        let sb = trace();
        build_and_verify(&sb, |cx| {
            assert!(!cx.is_terminated());
            let a = cx.reg(2);
            cx.side_exit_generic_if(a, 0x1004);
            assert!(!cx.is_terminated(), "emission continues after a side exit");
            cx.exit(0).unwrap();
            assert!(cx.is_terminated());
        });
    }

    #[test]
    fn a_dynamic_exit_is_unsupported_until_the_register_target_work() {
        let sb = trace();
        build_and_verify(&sb, |cx| {
            assert_eq!(cx.exit(1), Err(Unsupported));
            cx.exit_generic(0x1000); // the function must still end in a terminator
        });
    }

    #[test]
    fn a_generic_exit_counts_the_retired_instructions_and_a_side_exit_continues() {
        let sb = trace();
        let func = build_and_verify(&sb, |cx| {
            cx.retired = 3;
            let a = cx.reg(2);
            cx.side_exit_generic_if(a, 0x1004);
            cx.exit(0).unwrap();
        });
        let text = func.display().to_string();
        assert!(text.contains("brif"), "{text}");
        assert_eq!(
            text.matches("return").count(),
            2,
            "the side exit and the final exit"
        );
    }

    #[test]
    fn the_pointer_type_is_the_argument_type_and_r15_is_not_a_variable() {
        let sb = trace();
        build_and_verify(&sb, |cx| {
            assert_eq!(cx.ptr(), types::I64);
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cx.reg(15)));
            assert!(r.is_err());
            cx.exit_generic(0x1000);
        });
    }
}
