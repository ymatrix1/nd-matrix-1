//! OWNER: W2-NDP. The ARM condition codes as an SSA value: [`cond_holds`] is the native image of
//! `uop::holds(cond, f)` (the `COND_MASK` table of the predecoder), used by the `PRED` of a
//! conditional uop run through `NCtx::predicated`.
//!
//! | cond | Name | Expression (flags are `i32` 0 or 1) |
//! |---|---|---|
//! | 0, 1 | EQ, NE | `z`, `!z` |
//! | 2, 3 | CS, CC | `c`, `!c` |
//! | 4, 5 | MI, PL | `n`, `!n` |
//! | 6, 7 | VS, VC | `v`, `!v` |
//! | 8, 9 | HI, LS | `c & !z`, `!c \| z` |
//! | 10, 11 | GE, LT | `n == v`, `n != v` |
//! | 12, 13 | GT, LE | `!z & (n == v)`, `z \| (n != v)` |
//! | 14, 15 | AL | 1 (15 is the "never" slot of ARMv4 and always holds here, as in `holds`) |
//!
//! Provenance: VERIFIED against `uop::holds` for all 16 conditions x 16 flag nibbles by
//! `every_condition_equals_holds_on_every_flag_nibble` (native, where available) and by the
//! predicated differential tests of `lower_dp/diff.rs`; mutants 819 and 820 are killed by them.

use super::mutant;
use crate::machine::sb::native::emit::NCtx;
use cranelift_codegen::ir::{InstBuilder, Value};

/// An `i32` that is 1 when condition `cond` holds for the current flags and 0 otherwise. The
/// flags read are the variables of `cx`, so a flag-setting uop before it is seen.
pub fn cond_holds(cx: &mut NCtx<'_>, cond: u8) -> Value {
    let flag = |cx: &mut NCtx<'_>, i: u8| cx.flag(i);
    let not = |cx: &mut NCtx<'_>, v: Value| cx.b.ins().bxor_imm(v, 1);
    match cond & 15 {
        0 => flag(cx, 2),
        1 => {
            let z = flag(cx, 2);
            not(cx, z)
        }
        2 => flag(cx, 1),
        3 => {
            let c = flag(cx, 1);
            not(cx, c)
        }
        4 => flag(cx, 3),
        5 => {
            let n = flag(cx, 3);
            not(cx, n)
        }
        6 => flag(cx, 0),
        7 => {
            let v = flag(cx, 0);
            not(cx, v)
        }
        8 => {
            let (c, z) = (flag(cx, 1), flag(cx, 2));
            let nz = not(cx, z);
            if mutant(819) {
                cx.b.ins().bor(c, nz)
            } else {
                cx.b.ins().band(c, nz)
            }
        }
        9 => {
            let (c, z) = (flag(cx, 1), flag(cx, 2));
            let nc = not(cx, c);
            cx.b.ins().bor(nc, z)
        }
        10 | 11 => {
            let (n, v) = (flag(cx, 3), flag(cx, 0));
            let differ = cx.b.ins().bxor(n, v);
            if (cond & 15 == 11) != mutant(820) {
                differ
            } else {
                not(cx, differ)
            }
        }
        12 => {
            let (n, v, z) = (flag(cx, 3), flag(cx, 0), flag(cx, 2));
            let differ = cx.b.ins().bxor(n, v);
            let ge = not(cx, differ);
            let nz = not(cx, z);
            cx.b.ins().band(nz, ge)
        }
        13 => {
            let (n, v, z) = (flag(cx, 3), flag(cx, 0), flag(cx, 2));
            let differ = cx.b.ins().bxor(n, v);
            cx.b.ins().bor(z, differ)
        }
        _ => cx.const32(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::cache::tiny_trace;
    use crate::machine::sb::native::emit::testing::build_and_verify;
    use crate::machine::sb::native::lower_dp::harness::Rig;
    use crate::machine::sb::uop::{holds, unpack_flags, Uop};

    #[test]
    fn every_condition_builds_verified_ir_and_the_flags_it_reads_are_untouched() {
        let sb = tiny_trace(0x1000, &[(0x1000, 1)]);
        build_and_verify(&sb, |cx| {
            for cond in 0..16u8 {
                let _ = cond_holds(cx, cond);
            }
            assert_eq!((cx.dirty, cx.flags_def), (0, 0), "reading writes nothing");
            cx.exit_generic(0x1000);
        });
    }

    #[test]
    fn every_condition_equals_holds_on_every_flag_nibble() {
        let Some(mut rig) = Rig::new() else {
            return;
        };
        let mut seen = [[false; 2]; 16];
        for cond in 0..16u8 {
            // `if cond { r0 = 1 }` on r0 = 0: r0 afterwards is the condition
            let mut set = Uop::new(crate::machine::sb::uop::k::MOV_IMM, 0x1004);
            set.imm = 1;
            let mut pred = Uop::new(crate::machine::sb::uop::k::PRED, 0x1000);
            (pred.cond, pred.imm, pred.n) = (cond, 1, 0);
            let sb = Rig::trace(&[pred, set]);
            let native = rig.compile(&sb);
            for f in 0..16u8 {
                let mut cpu = crate::cpu::Cpu::new();
                unpack_flags(&mut cpu, f);
                cpu.r[15] = 0x1000;
                let got = rig.run_native(&sb, &native, &cpu);
                let want = holds(cond, f);
                assert_eq!(got.r[0], u32::from(want), "cond {cond} nibble {f:04b}");
                seen[cond as usize][usize::from(want)] = true;
            }
        }
        for (cond, s) in seen.iter().enumerate() {
            let want = if cond >= 14 {
                [false, true]
            } else {
                [true, true]
            };
            assert_eq!(*s, want, "cond {cond}: seen failing and holding");
        }
    }
}
