//! OWNER: W1-NCORE. Helpers of the native tests of every package: build a function with an
//! `NCtx` and verify its IR (`build_and_verify`, runs everywhere) or compile it through the
//! module wrapper (`compile`, then `Jit::run`; native is unavailable on some hosts, so the test
//! asserts the fallback there).

use super::super::rt::{Jit, Native};
use super::NCtx;
use crate::machine::sb::uop::Sb;
use cranelift_codegen::ir::{types, AbiParam, Function, Signature, UserFuncName};
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

fn verified(func: Function) -> Function {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::cache::tiny_trace;
    use crate::machine::sb::native::available;
    use crate::{cpu::Cpu, mem::Mem};

    #[test]
    fn the_helpers_verify_everywhere_and_compile_where_native_is_available() {
        let sb = tiny_trace(0x1000, &[(0x1000, 1)]);
        let func = build_and_verify(&sb, |cx| cx.exit_generic(0x1000));
        assert_eq!(func.signature.call_conv, CallConv::SystemV);
        if !available() {
            assert!(Jit::new().is_none() || !crate::machine::sb::native::rt::probe());
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
}
