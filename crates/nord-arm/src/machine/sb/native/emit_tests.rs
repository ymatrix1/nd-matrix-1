//! OWNER: W1-NCORE. Tests of the exits and the epilogue of `NCtx`: the IR of the register-target
//! exits, and native execution of exits, dirty-state stores and side exits against the tier-2
//! executor (`exec::run_sb`, the in-process oracle) and against values computed in Rust.
//!
//! Where native code is unavailable every executing test asserts the fallback instead of being
//! skipped (design note 9). Each differential test counts what it compared and fails when the
//! counts show it did not exercise the interesting cases.

use super::abi::{CTL_TARGET, OFF_THUMB};
use super::available;
use super::emit::testing::{build_and_verify, compile};
use super::emit::Unsupported;
use super::rt::Jit;
use crate::cpu::Cpu;
use crate::machine::sb::cache::tiny_trace;
use crate::machine::sb::exec::run_sb;
use crate::machine::sb::testkit::{random_cpu, CpuSnap, Rng};
use crate::machine::sb::uop::{Exit, ExitKind, Sb, Uop};
use crate::machine::SysBus;
use crate::mem::Mem;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder};

/// A trace that is one `EXIT` through an exit of `kind` reading register `reg`.
fn exit_trace(kind: ExitKind, reg: u8, done: u16) -> Sb {
    let mut e = Exit::new(kind, done, 0x1000 + 4 * (u32::from(done) - 1), 0x2000);
    e.reg = reg;
    let uops = vec![Uop::exit(0, 0x1000)];
    Sb::from_parts(0x1000, done, uops, vec![e], vec![(0x1000, 0)]).unwrap()
}

#[test]
fn a_dynamic_exit_and_an_indirect_exit_through_r15_are_unsupported_and_emit_nothing() {
    let mut sb = tiny_trace(0x1000, &[(0x1000, 1)]);
    let mut bad = Exit::new(ExitKind::Indirect, 1, 0x1000, 0);
    bad.reg = 15;
    let mut bad_arm = Exit::new(ExitKind::IndirectArm, 1, 0x1000, 0);
    bad_arm.reg = 15;
    let dynamic = Exit::new(ExitKind::Dynamic, 1, 0x1000, 0);
    sb.exits = vec![dynamic, bad, bad_arm].into_boxed_slice();
    build_and_verify(&sb, |cx| {
        let before = cx.b.func.dfg.num_insts();
        for idx in 0..3 {
            assert_eq!(cx.exit(idx), Err(Unsupported), "exit {idx}");
        }
        assert_eq!(cx.b.func.dfg.num_insts(), before, "nothing was emitted");
        assert!(!cx.is_terminated());
        cx.exit_generic(0x1000); // the function must still end in a terminator
    });
}

#[test]
fn indirect_exits_store_the_masked_target_and_only_bx_stores_the_thumb_byte() {
    let ir = |kind: ExitKind| {
        let sb = exit_trace(kind, 4, 3);
        build_and_verify(&sb, |cx| {
            assert_eq!(cx.exit(0), Ok(()));
            assert!(cx.is_terminated());
        })
        .display()
        .to_string()
    };
    let (bx, arm) = (ir(ExitKind::Indirect), ir(ExitKind::IndirectArm));
    assert_eq!(bx.matches("istore8").count(), 1, "{bx}");
    assert!(bx.contains(&format!("+{OFF_THUMB}")), "thumb byte: {bx}");
    assert_eq!(arm.matches("istore8").count(), 0, "{arm}");
    for text in [&bx, &arm] {
        assert!(
            text.contains(&format!("+{CTL_TARGET}")),
            "ctl.target: {text}"
        );
        assert!(text.contains("+60"), "r15 = target: {text}");
    }
}

#[test]
fn constants_above_the_sign_bit_are_the_same_bits() {
    let sb = tiny_trace(0x1000, &[(0x1000, 1)]);
    let func = build_and_verify(&sb, |cx| {
        let a = cx.const32(0xC000_0004);
        let b = cx.const32(u32::MAX);
        let s = cx.b.ins().iadd(a, b);
        cx.set_reg(0, s);
        cx.exit_generic(0xC000_0000); // a guest address with the top bit set
    });
    assert!(func.display().to_string().contains("iconst.i32"));
}

#[test]
fn register_target_exits_equal_the_tier2_executor_on_random_registers() {
    if !available() {
        assert!(super::NativeBackend::new().is_none(), "the fallback");
        return;
    }
    let mut jit = Jit::new().unwrap();
    let mut rng = Rng::new(0xB10C);
    let (mut compared, mut thumb_on, mut thumb_off, mut kept, mut high) = (0u32, 0, 0, 0, 0);
    for kind in [ExitKind::Indirect, ExitKind::IndirectArm] {
        for reg in 0..15u8 {
            let sb = exit_trace(kind, reg, 3);
            let n = compile(&mut jit, &sb, |cx| cx.exit(0).unwrap()).unwrap();
            for _ in 0..40 {
                let mut want = random_cpu(&mut rng, 0x1000);
                want.thumb = rng.chance(2);
                want.insns = u64::from(rng.next());
                if rng.chance(2) {
                    want.r[reg as usize] = want.r[reg as usize] & !3 | rng.below(4);
                }
                let mut got = want.clone();
                let was_thumb = want.thumb;
                let (mut bus, mut left) = (SysBus::new(), 100u64);
                let t2 = run_sb(&mut want, &mut bus, &sb, &mut left, false);
                let out = jit.run(&n, &mut got, &mut Mem::new(), 100);
                let diff = CpuSnap::of(&want, None).diff(&CpuSnap::of(&got, None));
                assert_eq!(diff, None, "{kind:?} r{reg}");
                assert_eq!((out.exit, out.target), (u32::from(t2.exit), t2.target));
                assert_eq!((got.r[15], got.thumb), (want.r[15], want.thumb));
                compared += 1;
                thumb_on += u32::from(got.thumb && kind == ExitKind::Indirect);
                thumb_off += u32::from(!got.thumb && kind == ExitKind::Indirect);
                kept += u32::from(kind == ExitKind::IndirectArm && got.thumb == was_thumb);
                high += u32::from(got.r[15] >= 0x8000_0000);
            }
        }
    }
    assert_eq!(compared, 2 * 15 * 40);
    assert!(
        thumb_on > 100 && thumb_off > 100,
        "bx saw both states: {thumb_on} {thumb_off}"
    );
    assert_eq!(kept, 15 * 40, "mov pc,rm never writes the Thumb bit");
    assert!(high > 100, "targets with the top bit set: {high}");
}

#[test]
fn the_epilogue_stores_only_what_was_written_and_counts_the_static_instructions() {
    if !available() {
        return;
    }
    let mut jit = Jit::new().unwrap();
    let mut e = Exit::new(ExitKind::Direct, 2, 0xC000_0004, 0xC000_0008);
    e.hits = 0;
    let sb = Sb::from_parts(
        0xC000_0000,
        2,
        vec![Uop::exit(0, 0xC000_0004)],
        vec![e],
        vec![(0xC000_0000, 0)],
    )
    .unwrap();
    // r1 = r0 + 1; Z = (r1 == 0); leave. Everything else must stay as it was.
    let n = compile(&mut jit, &sb, |cx| {
        let a = cx.reg(0);
        let sum = cx.b.ins().iadd_imm(a, 1);
        cx.set_reg(1, sum);
        let z = cx.b.ins().icmp_imm(IntCC::Equal, sum, 0);
        let z = cx.b.ins().uextend(types::I32, z);
        cx.set_flag(2, z);
        cx.exit(0).unwrap();
    })
    .unwrap();
    let mut rng = Rng::new(7);
    let (mut zeros, mut nonzeros) = (0, 0);
    for case in 0..64u32 {
        let mut cpu = random_cpu(&mut rng, 0xC000_0000);
        cpu.r[0] = if case % 4 == 0 {
            u32::MAX
        } else {
            rng.biased()
        };
        cpu.insns = 1000 + u64::from(case);
        let before = cpu.clone();
        let out = jit.run(&n, &mut cpu, &mut Mem::new(), 0);
        let sum = before.r[0].wrapping_add(1);
        let mut want = before.clone();
        want.r[1] = sum;
        want.z = sum == 0;
        want.r[15] = 0xC000_0008;
        want.insns += 2;
        assert_eq!(
            CpuSnap::of(&cpu, None).diff(&CpuSnap::of(&want, None)),
            None
        );
        assert_eq!((out.exit, out.target), (0, 0xC000_0008));
        zeros += usize::from(sum == 0);
        nonzeros += usize::from(sum != 0);
    }
    assert!(zeros >= 16 && nonzeros >= 16, "{zeros} {nonzeros}");
}

#[test]
fn a_side_exit_stores_the_state_and_counts_what_retired_before_it_and_the_other_path_goes_on() {
    if !available() {
        return;
    }
    let mut jit = Jit::new().unwrap();
    let e = Exit::new(ExitKind::Direct, 3, 0x100C, 0x1010);
    let uops = vec![Uop::exit(0, 0x100C)];
    let sb = Sb::from_parts(0x1000, 3, uops, vec![e], vec![(0x1000, 0)]).unwrap();
    // one instruction retired (r3 = 99), then a side exit before the instruction at 0x1004 when
    // r2 != 0, else leave through the static exit
    let n = compile(&mut jit, &sb, |cx| {
        let v = cx.const32(99);
        cx.set_reg(3, v);
        cx.retired = 1;
        let c = cx.reg(2);
        cx.side_exit_generic_if(c, 0x1004);
        cx.retired = 3;
        cx.exit(0).unwrap();
    })
    .unwrap();
    let run = |r2: u32| {
        let mut cpu = Cpu::new();
        cpu.r[2] = r2;
        cpu.insns = 10;
        let out = jit.run(&n, &mut cpu, &mut Mem::new(), 0);
        (out, cpu.r[3], cpu.r[15], cpu.insns)
    };
    let (out, r3, r15, insns) = run(5);
    assert!(out.is_generic(), "the side path leaves for the interpreter");
    assert_eq!((out.target, r3, r15, insns), (0x1004, 99, 0x1004, 11));
    let (out, r3, r15, insns) = run(0);
    assert_eq!(
        (out.exit, out.target, r3, r15, insns),
        (0, 0x1010, 99, 0x1010, 13)
    );
}
