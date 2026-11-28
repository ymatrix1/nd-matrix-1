//! OWNER: W1-NCORE. Tests of the module wrapper [`Jit`] (handles, generations, the byte cap,
//! threads) and the hand-written IR functions the native tests of every package can reuse.
//!
//! Everything here is safe code: the unsafe call lives in `rt.rs` and is reached through
//! `Jit::run`. Where native code is unavailable each test asserts the fallback instead of being
//! skipped silently (design note 9).

use super::abi::{NativeCtl, CTL_EXIT, CTL_TARGET, OFF_INSNS, OFF_R};
use super::available;
use super::rt::Jit;
use crate::cpu::Cpu;
use crate::mem::raw::RawMem;
use crate::mem::Mem;
use cranelift_codegen::ir::{
    types, AbiParam, Function, InstBuilder, MemFlags, Signature, UserFuncName,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

/// `r1 = r0 + imm; insns += 3; ctl.exit = ctl.target = tag; return tag`, built by hand in the
/// C ABI of `jit`.
pub(crate) fn adder(jit: &Jit, imm: i64, tag: u32) -> Function {
    let mut func = jit.new_function();
    let mut fctx = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut func, &mut fctx);
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    b.seal_block(entry);
    let (cpu, ctl) = (b.block_params(entry)[0], b.block_params(entry)[1]);
    let m = MemFlags::trusted();
    let r0 = b.ins().load(types::I32, m, cpu, OFF_R as i32);
    let sum = b.ins().iadd_imm(r0, imm);
    b.ins().store(m, sum, cpu, (OFF_R + 4) as i32);
    let insns = b.ins().load(types::I64, m, cpu, OFF_INSNS as i32);
    let insns = b.ins().iadd_imm(insns, 3);
    b.ins().store(m, insns, cpu, OFF_INSNS as i32);
    let t = b.ins().iconst(types::I32, i64::from(tag as i32));
    b.ins().store(m, t, ctl, CTL_EXIT as i32);
    b.ins().store(m, t, ctl, CTL_TARGET as i32);
    b.ins().return_(&[t]);
    b.finalize();
    func
}

/// A `Cpu` and a control block with null memory pointers (for functions that touch no memory).
pub(crate) fn fresh() -> (Cpu, NativeCtl) {
    let raw = RawMem {
        table: std::ptr::null(),
        arena: std::ptr::null_mut(),
    };
    (Cpu::new(), NativeCtl::new(raw, 10))
}

/// Run `f`, true when it panicked (a refused handle).
fn panics(f: impl FnOnce()) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err()
}

#[test]
fn a_hand_written_function_runs_through_run_with_the_refreshed_memory_pointers() {
    if !available() {
        super::emit::testing::assert_fallback();
        return;
    }
    let mut jit = Jit::new().expect("available() implies a module");
    let n = jit.define_native(adder(&jit, 7, 0x1234)).unwrap();
    let (mut cpu, mut mem) = (Cpu::new(), Mem::new());
    cpu.r[0] = 100;
    cpu.insns = 5;
    let out = jit.run(&n, &mut cpu, &mut mem, 50);
    assert_eq!(
        (out.exit, out.target, cpu.r[1], cpu.insns, out.is_generic()),
        (0x1234, 0x1234, 107, 8, false)
    );
    assert_eq!(n.raw() as usize, n.addr());
}

#[test]
fn the_jit_and_its_handles_are_send() {
    fn is_send<T: Send>() {}
    is_send::<Jit>();
    is_send::<super::rt::Native>();
    is_send::<super::rt::SendCell<*mut u8>>();
}

#[test]
fn a_handle_of_another_module_or_a_reset_generation_is_refused_not_called() {
    if !available() {
        return;
    }
    let mut a = Jit::new().unwrap();
    let b = Jit::new().unwrap();
    let n = a.define_native(adder(&a, 1, 5)).unwrap();
    let mut mem = Mem::new();
    assert!(
        panics(|| {
            b.run(&n, &mut Cpu::new(), &mut Mem::new(), 0);
        }),
        "another module's handle"
    );
    a.free_all();
    assert!(
        panics(|| {
            a.run(&n, &mut Cpu::new(), &mut Mem::new(), 0);
        }),
        "a handle of the generation before free_all"
    );
    let n2 = a.define_native(adder(&a, 2, 6)).unwrap();
    let mut cpu = Cpu::new();
    assert_eq!(a.run(&n2, &mut cpu, &mut mem, 0).exit, 6);
    assert_eq!(cpu.r[1], 2, "the module works again after the reset");
}

#[test]
fn a_function_of_another_signature_is_defined_for_linking_but_never_runnable() {
    if !available() {
        return;
    }
    let mut jit = Jit::new().unwrap();
    let mut sig = Signature::new(jit.call_conv());
    sig.params.push(AbiParam::new(types::I32));
    sig.returns.push(AbiParam::new(types::I32));
    let mut func = Function::with_name_signature(UserFuncName::default(), sig);
    let mut fctx = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut func, &mut fctx);
    let e = b.create_block();
    b.append_block_params_for_function_params(e);
    b.switch_to_block(e);
    b.seal_block(e);
    let p = b.block_params(e)[0];
    b.ins().return_(&[p]);
    b.finalize();
    let n = jit.define_native(func).unwrap();
    assert_ne!(n.addr(), 0);
    assert!(panics(|| {
        jit.run(&n, &mut Cpu::new(), &mut Mem::new(), 0);
    }));
}

#[test]
fn a_full_module_refuses_to_define_until_it_is_reset() {
    if !available() {
        return;
    }
    let mut jit = Jit::new().unwrap();
    jit.set_code_cap(1);
    assert!(!jit.is_full());
    assert!(jit.define_native(adder(&jit, 1, 1)).is_some());
    assert!(jit.is_full() && jit.code_bytes() > 0);
    assert!(jit.define_native(adder(&jit, 1, 1)).is_none());
    jit.free_all();
    assert!(!jit.is_full() && jit.code_bytes() == 0);
    assert!(jit.define_native(adder(&jit, 1, 1)).is_some());
}

#[test]
fn a_jit_moves_to_another_thread_and_runs_there() {
    if !available() {
        return;
    }
    let mut jit = Jit::new().unwrap();
    let n = jit.define_native(adder(&jit, 5, 77)).unwrap();
    let out = std::thread::spawn(move || {
        let mut cpu = Cpu::new();
        cpu.r[0] = 1;
        let out = jit.run(&n, &mut cpu, &mut Mem::new(), 0);
        (out.exit, cpu.r[1])
    })
    .join()
    .unwrap();
    assert_eq!(out, (77, 6));
}

#[test]
fn the_default_cap_is_the_designs_32_mib_and_a_new_module_is_empty() {
    assert_eq!(super::rt::CODE_CAP, 32 << 20);
    if let Some(jit) = Jit::new() {
        assert_eq!((jit.code_bytes(), jit.is_full()), (0, false));
        assert_eq!(jit.ptr_type(), types::I64);
    }
}

#[test]
fn compile_builds_the_function_of_a_trace_and_a_trace_without_an_exit_stays_on_tier_2() {
    use crate::machine::sb::uop::{Exit, ExitKind, Sb, Uop};
    let exit = || vec![Exit::new(ExitKind::Direct, 2, 0x1004, 0x1008)];
    let words = vec![(0x1000, 0)];
    let generic = vec![Uop::generic(0xE3A0_0005, 0x1000, 0), Uop::exit(0, 0x1004)];
    let sb = Sb::from_parts(0x1000, 2, generic, exit(), words.clone()).unwrap();
    let empty = Sb::from_parts(0x1000, 2, vec![], exit(), words).unwrap();
    if !available() {
        return;
    }
    let mut jit = Jit::new().unwrap();
    assert!(jit.compile(&empty).is_none(), "no exit, no function");
    let n = jit.compile(&sb).expect("a terminated trace compiles");
    let (mut cpu, mut mem) = (Cpu::new(), Mem::new());
    cpu.insns = 40;
    let out = jit.run(&n, &mut cpu, &mut mem, 100);
    // `GENERIC` is never native: the trace leaves before its first instruction, counted 0
    assert!(out.is_generic());
    assert_eq!(
        (out.target, cpu.r[15], cpu.insns, cpu.r[0]),
        (0x1000, 0x1000, 40, 0)
    );
}
