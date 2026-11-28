//! OWNER: W1-NCORE. Helpers of the native tests of every package: build a function with an
//! `NCtx` and verify its IR (`build_and_verify`, runs everywhere) or compile it through the
//! module wrapper (`compile`, then `Jit::run`; native is unavailable on some hosts, so the test
//! asserts the fallback there with `assert_fallback`), and `toy_lower`, a stand-in lowerer for
//! the tests of the walk, the predicated block and the loop (the real ones are W2-NDP and
//! W2-NMEM; pass it to `codegen::build_function_with`).

use super::super::rt::{Jit, Native};
use super::super::{available, host_runs_native, tail_calls_available, NativeBackend};
use super::{NCtx, Unsupported};
use crate::machine::sb::uop::{k, Sb, Uop};
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, AbiParam, Function, InstBuilder, Signature, UserFuncName};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings;
use cranelift_codegen::verify_function;
use cranelift_frontend::FunctionBuilderContext;

/// A function with the native signature `(i64, i64) -> i32` in the SystemV convention: for
/// IR-level tests that never execute (use [`compile`] to run).
pub fn new_function() -> Function {
    let mut sig = Signature::new(CallConv::SystemV);
    sig.params.push(AbiParam::new(types::I64));
    sig.params.push(AbiParam::new(types::I64));
    sig.returns.push(AbiParam::new(types::I32));
    Function::with_name_signature(UserFuncName::default(), sig)
}

/// Verify the IR of `func` (panics with the function text on a defect) and return it.
pub(crate) fn verified(func: Function) -> Function {
    let flags = settings::Flags::new(settings::builder());
    if let Err(e) = verify_function(&func, &flags) {
        panic!("{e}\n{func}");
    }
    func
}

/// Build a function for `sb` with `emit`, finish it and verify the IR (no executable memory).
pub fn build_and_verify(sb: &Sb, emit: impl FnOnce(&mut NCtx<'_>)) -> Function {
    let mut func = new_function();
    let mut fctx = FunctionBuilderContext::new();
    let mut cx = NCtx::new(&mut func, &mut fctx, sb);
    emit(&mut cx);
    cx.finish();
    verified(func)
}

/// Build the function of `sb` in the C ABI of `jit` with `emit`, verify the IR and compile it:
/// the entry point of every native test (`jit.run(&native, cpu, mem, limit)` runs it). `None`
/// when the module refuses the function.
pub fn compile(jit: &mut Jit, sb: &Sb, emit: impl FnOnce(&mut NCtx<'_>)) -> Option<Native> {
    let mut func = jit.new_function();
    let mut fctx = FunctionBuilderContext::new();
    let mut cx = NCtx::new(&mut func, &mut fctx, sb);
    emit(&mut cx);
    cx.finish();
    jit.define_native(verified(func))
}

/// What a native test asserts where `available()` is false instead of skipping (design note 9):
/// there is no backend and no chaining, and the answer is false for exactly one of two reasons,
/// an unproven target (`NATIVE_TARGET_PROVEN`) or a host that cannot run native code.
pub(crate) fn assert_fallback() {
    assert!(!available());
    assert!(
        NativeBackend::new().is_none(),
        "no backend without native code"
    );
    assert!(!tail_calls_available(), "no chaining without native code");
    assert!(
        !super::super::NATIVE_TARGET_PROVEN || !host_runs_native(),
        "false only for an unproven target or a host without executable memory"
    );
}

/// A tiny lowerer with the shapes the walk tests need, by kind: `ADD_IMM` is `r[rd] =
/// r[rn] + imm`; `PRED` runs the next `imm` uops when `r[rn] != 0`; `GUARD` leaves for the
/// interpreter at its pc when `r[rn] == imm`; `EXIT` and `LOOP_BACK` take their exit; every
/// other kind is [`Unsupported`].
pub(crate) fn toy_lower(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
    match u.kind {
        k::ADD_IMM => {
            let a = cx.reg(u.rn);
            let sum = cx.b.ins().iadd_imm(a, i64::from(u.imm as i32));
            cx.set_reg(u.rd, sum);
            Ok(())
        }
        k::PRED => {
            let rn = u.rn;
            cx.predicated(u.imm as usize, |cx| cx.reg(rn))
        }
        k::GUARD => {
            let a = cx.reg(u.rn);
            let hit =
                cx.b.ins()
                    .icmp_imm(IntCC::Equal, a, i64::from(u.imm as i32));
            cx.side_exit_generic_if(hit, u.pc);
            Ok(())
        }
        k::EXIT => cx.exit(u.exit_idx()),
        k::LOOP_BACK => cx.loop_back(u.exit_idx()),
        _ => Err(Unsupported),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::cache::tiny_trace;
    use crate::{cpu::Cpu, mem::Mem};

    #[test]
    fn the_helpers_verify_everywhere_and_compile_where_native_is_available() {
        let sb = tiny_trace(0x1000, &[(0x1000, 1)]);
        let func = build_and_verify(&sb, |cx| cx.exit_generic(0x1000));
        assert_eq!(func.signature.call_conv, CallConv::SystemV);
        if !available() {
            assert_fallback();
            return;
        }
        let mut jit = Jit::new().unwrap();
        let n = compile(&mut jit, &sb, |cx| cx.exit_generic(0xC000_1000)).unwrap();
        let mut cpu = Cpu::new();
        let out = jit.run(&n, &mut cpu, &mut Mem::new(), 0);
        assert!(out.is_generic());
        assert_eq!(
            (out.target, cpu.r[15], cpu.insns),
            (0xC000_1000, 0xC000_1000, 0)
        );
    }

    #[test]
    fn the_toy_lowerer_handles_its_kinds_and_declines_the_rest() {
        let sb = tiny_trace(0x1000, &[(0x1000, 1)]);
        build_and_verify(&sb, |cx| {
            let mut add = Uop::new(k::ADD_IMM, 0x1000);
            (add.rd, add.rn, add.imm) = (1, 0, 5);
            assert_eq!(toy_lower(cx, &add), Ok(()));
            assert_eq!(cx.dirty, 0b10);
            let mut guard = Uop::new(k::GUARD, 0x1004);
            guard.rn = 1;
            assert_eq!(toy_lower(cx, &guard), Ok(()));
            assert_eq!(
                toy_lower(cx, &Uop::new(k::LDR_IMM, 0x1000)),
                Err(Unsupported)
            );
            cx.exit_generic(0x1008);
        });
    }
}
