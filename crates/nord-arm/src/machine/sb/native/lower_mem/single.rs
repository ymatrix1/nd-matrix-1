//! OWNER: W2-NMEM. Native single transfers: `ldr/ldrb/str/strb` with an immediate or a shifted
//! register offset, `ldrh/ldrsh/ldrsb/strh`, `ldrd/strd` and the absolute literal load
//! (`LDR_ABS`). The rules are those of `exec_mem::single` and `exec_mem::half`.
//!
//! The decision (`plan`) is made before anything is emitted: a shape the executor arms refuse or
//! the contract keeps out of the family is `Err(Unsupported)` with an untouched function.

use super::access::{data_flags, host_addr};
use super::mutant;
use crate::machine::sb::exec_mem::{F_P, F_REG, F_U, F_WB};
use crate::machine::sb::native::emit::{NCtx, Unsupported};
use crate::machine::sb::uop::{k, Uop};
use cranelift_codegen::ir::{types, InstBuilder, Value};

/// What a single transfer moves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Word,
    Byte,
    Half,
    SignedByte,
    SignedHalf,
    Pair,
}

/// How the offset is given.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Off {
    Imm(u32),
    /// `Rm` plain (the half/double register form).
    Reg(u8),
    /// `Rm` shifted by `ty` (0 LSL, 1 LSR, 2 ASR, 3 ROR) and `amount` (never RRX).
    Shifted(u8, u8, u8),
}

/// A decoded single-transfer uop.
#[derive(Clone, Copy, Debug)]
struct Plan {
    load: bool,
    op: Op,
    off: Off,
}

/// Decide whether `u` is lowerable and how; `None` means [`Unsupported`].
fn plan(u: &Uop) -> Option<Plan> {
    let (load, op, shifted) = match u.kind {
        k::LDR_IMM => (true, Op::Word, false),
        k::LDRB_IMM => (true, Op::Byte, false),
        k::STR_IMM => (false, Op::Word, false),
        k::STRB_IMM => (false, Op::Byte, false),
        k::LDR_REG => (true, Op::Word, true),
        k::LDRB_REG => (true, Op::Byte, true),
        k::STR_REG => (false, Op::Word, true),
        k::STRB_REG => (false, Op::Byte, true),
        k::LDRH => (true, Op::Half, false),
        k::LDRSH => (true, Op::SignedHalf, false),
        k::LDRSB => (true, Op::SignedByte, false),
        k::STRH => (false, Op::Half, false),
        k::LDRD => (true, Op::Pair, false),
        k::STRD => (false, Op::Pair, false),
        _ => return None,
    };
    if u.rn >= 15 || u.rd >= 15 || (op == Op::Pair && u.rd >= 14) {
        return None;
    }
    let off = if shifted {
        let (ty, amount) = (u.sh >> 5 & 3, u.sh & 31);
        if u.rm >= 15 || (ty == 3 && amount == 0) {
            return None; // RRX needs the carry: the lowerer keeps it GENERIC
        }
        Off::Shifted(u.rm, ty, amount)
    } else if u.kind >= k::LDRH && u.rs & F_REG != 0 {
        if u.rm >= 15 {
            return None;
        }
        Off::Reg(u.rm)
    } else {
        Off::Imm(u.imm)
    };
    Some(Plan { load, op, off })
}

/// The offset as a value: shifted per `alu::shift(ty, amount, rm, _, imm = true)`.
fn offset_value(cx: &mut NCtx<'_>, off: Off) -> Value {
    match off {
        Off::Imm(i) => cx.const32(i),
        Off::Reg(rm) => {
            let v = cx.reg(rm);
            if mutant(879) {
                cx.b.ins().ishl_imm(v, 2)
            } else {
                v
            }
        }
        Off::Shifted(rm, ty, amount) => {
            let v = cx.reg(rm);
            let ty = if mutant(868) { 0 } else { ty };
            match (ty, amount) {
                (0, 0) => v,
                (0, n) => cx.b.ins().ishl_imm(v, i64::from(n)),
                (1, 0) if mutant(869) => v,
                (1, 0) => cx.const32(0),
                (1, n) => cx.b.ins().ushr_imm(v, i64::from(n)),
                (2, 0) if mutant(870) => v,
                (2, 0) => cx.b.ins().sshr_imm(v, 31),
                (2, n) => cx.b.ins().sshr_imm(v, i64::from(n)),
                (_, n) => cx.b.ins().rotr_imm(v, i64::from(n)),
            }
        }
    }
}

/// `base +/- offset` as the U flag says.
fn step_base(cx: &mut NCtx<'_>, base: Value, off: Off, flags: u8) -> Value {
    let up = (flags & F_U != 0) != mutant(858);
    match off {
        Off::Imm(i) => {
            let i = i64::from(i);
            cx.b.ins().iadd_imm(base, if up { i } else { -i })
        }
        _ => {
            let o = offset_value(cx, off);
            if up || mutant(878) {
                cx.b.ins().iadd(base, o)
            } else {
                cx.b.ins().isub(base, o)
            }
        }
    }
}

/// Finish a load: the base is written back first (skipped when it is `rd`), then `rd`.
fn finish_load(cx: &mut NCtx<'_>, u: &Uop, moved: Value, v: Value) {
    let wb = u.rs & F_WB != 0 && !mutant(857);
    if mutant(860) {
        cx.set_reg(u.rd, v);
        if wb {
            cx.set_reg(u.rn, moved);
        }
        return;
    }
    if wb && u.rn != u.rd {
        cx.set_reg(u.rn, moved);
    }
    cx.set_reg(u.rd, v);
}

/// `ldr`, `ldrb`, `str`, `strb`, `ldrh`, `ldrsh`, `ldrsb`, `strh`, `ldrd` and `strd`.
pub(super) fn lower_single(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
    let Some(Plan { load, op, off }) = plan(u) else {
        return Err(Unsupported);
    };
    let base = cx.reg(u.rn);
    let moved = step_base(cx, base, off, u.rs);
    let addr = if u.rs & F_P != 0 || mutant(859) {
        moved
    } else {
        base
    };
    let wb = u.rs & F_WB != 0 && !mutant(857);
    let fl = data_flags();
    match (load, op) {
        (true, Op::Word) => {
            let aligned = if mutant(853) {
                addr
            } else {
                cx.b.ins().band_imm(addr, -4)
            };
            let host = host_addr(cx, aligned, false, None, u.pc);
            let v = cx.b.ins().load(types::I32, fl, host, 0);
            let rot = cx.b.ins().band_imm(addr, 3);
            let rot = cx.b.ins().ishl_imm(rot, 3);
            let v = if mutant(852) {
                v
            } else {
                cx.b.ins().rotr(v, rot)
            };
            finish_load(cx, u, moved, v);
        }
        (true, Op::Byte | Op::SignedByte) => {
            let host = host_addr(cx, addr, false, None, u.pc);
            let v = if op == Op::Byte || mutant(855) {
                cx.b.ins().uload8(types::I32, fl, host, 0)
            } else {
                cx.b.ins().sload8(types::I32, fl, host, 0)
            };
            finish_load(cx, u, moved, v);
        }
        (true, Op::Half | Op::SignedHalf) => {
            let aligned = half_addr(cx, addr);
            let host = host_addr(cx, aligned, false, None, u.pc);
            let v = if op == Op::Half || mutant(856) {
                cx.b.ins().uload16(types::I32, fl, host, 0)
            } else {
                cx.b.ins().sload16(types::I32, fl, host, 0)
            };
            finish_load(cx, u, moved, v);
        }
        (true, Op::Pair) => {
            let aligned = cx.b.ins().band_imm(addr, -4);
            let span = (!mutant(863)).then_some(8);
            let host = host_addr(cx, aligned, false, span, u.pc);
            let lo = cx.b.ins().load(types::I32, fl, host, 0);
            let hi = cx.b.ins().load(types::I32, fl, host, 4);
            let (lo, hi) = if mutant(862) { (hi, lo) } else { (lo, hi) };
            // the pair lands after the write-back, so it wins when `rn` is `rd` or `rd + 1`
            if wb {
                cx.set_reg(u.rn, moved);
            }
            cx.set_reg(u.rd, lo);
            cx.set_reg(u.rd + 1, hi);
        }
        (false, Op::Word) => {
            let aligned = cx.b.ins().band_imm(addr, -4);
            let host = host_addr(cx, aligned, true, None, u.pc);
            let v = store_value(cx, u, moved, wb);
            cx.b.ins().store(fl, v, host, 0);
            write_back(cx, u, moved, wb);
        }
        (false, Op::Byte) => {
            let host = host_addr(cx, addr, true, None, u.pc);
            let v = store_value(cx, u, moved, wb);
            if mutant(874) {
                cx.b.ins().istore16(fl, v, host, 0);
            } else {
                cx.b.ins().istore8(fl, v, host, 0);
            }
            write_back(cx, u, moved, wb);
        }
        (false, Op::Half) => {
            let aligned = half_addr(cx, addr);
            let host = host_addr(cx, aligned, true, None, u.pc);
            let v = store_value(cx, u, moved, wb);
            if mutant(875) {
                cx.b.ins().istore8(fl, v, host, 0);
            } else {
                cx.b.ins().istore16(fl, v, host, 0);
            }
            write_back(cx, u, moved, wb);
        }
        (false, Op::Pair) => {
            let aligned = cx.b.ins().band_imm(addr, -4);
            let span = (!mutant(863)).then_some(8);
            let host = host_addr(cx, aligned, true, span, u.pc);
            let (lo, hi) = (cx.reg(u.rd), cx.reg(u.rd + 1));
            cx.b.ins().store(fl, lo, host, 0);
            cx.b.ins().store(fl, hi, host, 4);
            write_back(cx, u, moved, wb);
        }
        (false, Op::SignedByte | Op::SignedHalf) => return Err(Unsupported),
    }
    Ok(())
}

/// The halfword address rule: bit 0 is ignored.
fn half_addr(cx: &mut NCtx<'_>, addr: Value) -> Value {
    if mutant(854) {
        addr
    } else {
        cx.b.ins().band_imm(addr, -2)
    }
}

/// The value a store writes: `rd` as it is before the write-back.
fn store_value(cx: &mut NCtx<'_>, u: &Uop, moved: Value, wb: bool) -> Value {
    if mutant(861) && wb {
        cx.set_reg(u.rn, moved);
    }
    cx.reg(u.rd)
}

/// The write-back of a store.
fn write_back(cx: &mut NCtx<'_>, u: &Uop, moved: Value, wb: bool) {
    if wb {
        cx.set_reg(u.rn, moved);
    }
}

/// `LDR_ABS`: a word load of a constant address, rotated by `8 * (imm & 3)`.
pub(super) fn lower_abs(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
    if u.rd >= 15 {
        return Err(Unsupported);
    }
    let aligned = cx.const32(u.imm & !3);
    let host = host_addr(cx, aligned, false, None, u.pc);
    let v = cx.b.ins().load(types::I32, data_flags(), host, 0);
    let rot = 8 * (u.imm & 3);
    let v = if rot == 0 || mutant(871) {
        v
    } else {
        cx.b.ins().rotr_imm(v, i64::from(rot))
    };
    cx.set_reg(u.rd, v);
    Ok(())
}

#[cfg(test)]
mod unit {
    use super::*;
    use crate::machine::sb::native::emit::testing::build_and_verify;
    use crate::machine::sb::native::lower_mem::tests::one;

    #[test]
    fn r15_anywhere_is_declined_before_anything_is_emitted() {
        let plain = Uop::new(k::LDR_IMM, 0x1000);
        assert!(plan(&plain).is_some());
        let sb = one(plain);
        build_and_verify(&sb, |cx| {
            let before = cx.b.func.dfg.num_insts();
            for (rd, rn) in [(15u8, 0u8), (0, 15)] {
                let mut u = plain;
                (u.rd, u.rn) = (rd, rn);
                assert!(plan(&u).is_none(), "rd {rd} rn {rn}");
                assert_eq!(lower_single(cx, &u), Err(Unsupported));
            }
            let mut abs = Uop::new(k::LDR_ABS, 0x1000);
            abs.rd = 15;
            assert_eq!(lower_abs(cx, &abs), Err(Unsupported));
            assert_eq!(cx.b.func.dfg.num_insts(), before);
            assert_eq!(lower_single(cx, &plain), Ok(()));
            cx.exit_generic(0x1000);
        });
    }
}
