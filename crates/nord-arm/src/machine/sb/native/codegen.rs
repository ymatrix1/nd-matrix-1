//! OWNER: W1-NCORE. From one `Sb` to one Cranelift function (IR only: no ISA, no executable
//! memory, so the tests run everywhere).
//!
//! Spec (design note 9, WP W1-NCORE): walk the uops; each family goes through the static registry
//! `uop::k::family` to `native::lower_dp`, `lower_mem` or `lower_flow` (so no work package edits
//! this file to register itself). A lowerer that returns [`Unsupported`] ends the native trace:
//! everything before it ran natively and the trace leaves with `exit_generic(u.pc)`, so the
//! driver runs the rest through the interpreter. `retired` is advanced by `Uop::n` after each
//! uop that lowered. The skeleton handles no uop natively, so every trace is one `Generic` exit
//! at its first instruction. A lowerer must return `Unsupported` before it emits anything, and a
//! `PRED` whose body is not entirely lowerable is `Unsupported` itself (its `pc` is the first
//! instruction of the body, so the state before it is exact); an instruction that lowers to
//! several uops carries `n = 1` on the first and `n = 0` on the others.

use super::emit::{NCtx, Unsupported};
use super::{lower_dp, lower_flow, lower_mem};
use crate::machine::sb::uop::{k, Sb, Uop};
use cranelift_codegen::ir::{types, AbiParam, Function, Signature, UserFuncName};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilderContext;

/// Lower one uop through the static family registry.
pub fn lower_uop(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
    match k::family(u.kind) {
        k::Family::Dp => lower_dp::lower_dp(cx, u),
        k::Family::Mem => lower_mem::lower_mem(cx, u),
        k::Family::Flow | k::Family::Fused => lower_flow::lower_flow(cx, u),
        k::Family::Invalid => unreachable!("unknown uop kind {}", u.kind),
    }
}

/// The signature of a compiled trace: `(cpu, ctl) -> exit index`.
pub fn trace_signature(call_conv: CallConv, ptr: cranelift_codegen::ir::Type) -> Signature {
    let mut sig = Signature::new(call_conv);
    sig.params.push(AbiParam::new(ptr));
    sig.params.push(AbiParam::new(ptr));
    sig.returns.push(AbiParam::new(types::I32));
    sig
}

/// Build the Cranelift function of `sb` for a pointer type `ptr`, or `None` when the uops do not
/// end in an exit (a trace-builder bug: the backend then leaves the trace on T2, which is
/// exact). A lowerer returns [`Unsupported`] before it emits anything, so the exit at `u.pc`
/// always starts from a block that is still open; after a lowered exit the block is filled and
/// nothing more is emitted.
pub fn build_function(
    sb: &Sb,
    call_conv: CallConv,
    ptr: cranelift_codegen::ir::Type,
) -> Option<Function> {
    let mut func = Function::with_name_signature(
        UserFuncName::user(0, sb.entry),
        trace_signature(call_conv, ptr),
    );
    let mut fctx = FunctionBuilderContext::new();
    let mut cx = NCtx::new(&mut func, &mut fctx, sb);
    for u in sb.uops.iter() {
        if cx.is_terminated() {
            break;
        }
        if lower_uop(&mut cx, u).is_err() {
            cx.exit_generic(u.pc);
            break;
        }
        cx.retired += u16::from(u.n);
    }
    let terminated = cx.is_terminated();
    if terminated {
        cx.finish();
        Some(func)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::uop::{Exit, ExitKind};
    use cranelift_codegen::settings;
    use cranelift_codegen::verify_function;

    fn sb(uops: Vec<Uop>) -> Sb {
        let exits = vec![Exit::new(ExitKind::Direct, 2, 0x1004, 0x1008)];
        Sb::from_parts(0x1000, 2, uops, exits, vec![(0x1000, 0)]).unwrap()
    }

    fn verify(func: &Function) {
        let flags = settings::Flags::new(settings::builder());
        if let Err(e) = verify_function(func, &flags) {
            panic!("{e}\n{func}");
        }
    }

    #[test]
    fn the_skeleton_ends_every_trace_with_a_generic_exit_at_its_first_unsupported_uop() {
        let t = sb(vec![
            Uop::generic(0xE3A0_0005, 0x1000, 0),
            Uop::generic(0xE280_0002, 0x1004, 0),
            Uop::exit(0, 0x1004),
        ]);
        let func = build_function(&t, CallConv::SystemV, types::I64).unwrap();
        verify(&func);
        let text = func.display().to_string();
        assert_eq!(text.matches("return").count(), 1, "{text}");
        assert!(
            text.contains("4096") || text.contains("0x1000"),
            "r15 = 0x1000: {text}"
        );
    }

    #[test]
    fn a_trace_that_does_not_end_in_an_exit_is_not_compiled() {
        let t = sb(vec![]);
        assert!(build_function(&t, CallConv::SystemV, types::I64).is_none());
    }

    #[test]
    fn every_family_is_routed_to_its_native_lowerer_and_an_invalid_kind_is_unreachable() {
        let t = sb(vec![Uop::exit(0, 0x1000)]);
        let mut func = Function::with_name_signature(
            UserFuncName::user(0, 1),
            trace_signature(CallConv::SystemV, types::I64),
        );
        let mut fctx = FunctionBuilderContext::new();
        let mut cx = NCtx::new(&mut func, &mut fctx, &t);
        for kind in [k::ADD_IMM, k::LDR_IMM, k::EXIT, k::CMP_GUARD_I] {
            assert_eq!(
                lower_uop(&mut cx, &Uop::new(kind, 0x1000)),
                Err(Unsupported),
                "kind {kind}"
            );
        }
        cx.exit_generic(0x1000);
        cx.finish();
        verify(&func);
        let bad = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut f2 = Function::with_name_signature(
                UserFuncName::user(0, 2),
                trace_signature(CallConv::SystemV, types::I64),
            );
            let mut c2 = FunctionBuilderContext::new();
            let mut cx2 = NCtx::new(&mut f2, &mut c2, &t);
            let _ = lower_uop(&mut cx2, &Uop::new(200, 0));
        }));
        assert!(bad.is_err());
    }

    #[test]
    fn the_signature_is_cpu_and_ctl_in_and_the_exit_index_out() {
        let s = trace_signature(CallConv::SystemV, types::I64);
        assert_eq!((s.params.len(), s.returns.len()), (2, 1));
        assert_eq!(s.returns[0].value_type, types::I32);
    }
}
