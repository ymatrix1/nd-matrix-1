//! The float handlers: one `extern "C"` function per kind of `h_float/mod.rs`.
//!
//! OWNER: W2-FLOAT. Each is a few lines around [`run`]: the op on one element through `Elem`
//! (the exact kernels of `kernels/`), and the same op on both through the paired kernel where one
//! exists (`add2`, `mul2`, `avg2`: add, subtract, multiply and average, the SIMD forms that halve
//! the cost) or through two single runs. A handler checks what can be wrong with its operands
//! before it writes anything and answers `Exit::Slow` then, so a bail leaves the core untouched
//! and the interpreter runs the instruction (design 4.5 rule 1). The lowering never produces such
//! an operand; the check is there so that no uop can make a handler panic or run a different op.
use super::run::{enabled, finish, run, unpack};
use crate::core::aot_pe::Pe;
use crate::core::decode_cache::ComputeOp;
use crate::core::uop::exec::h_float::fgen_supports;
use crate::core::uop::exec::pe::{add2, avg2, mul2};
use crate::core::uop::types::{Exit, Uop, UopCtx, UF_SIMD};

/// The bail exit: nothing was written.
const SLOW: u32 = Exit::Slow as u32;

/// `FGEN`: any float op `Pe::exec` implements and the kernels do not, with the compute field in
/// `a` (`RECIPS`, `RSQRTS`, the float multifunction forms). `Pe` is the v1 element, verified
/// equal to the interpreter by `fgen_ops_equal_interpreter`.
///
/// # Safety
/// The handler ABI: `ctx` and `u` valid, `(*ctx).core` the running core.
pub(super) unsafe extern "C" fn fgen(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    let (ctx, u) = (&mut *ctx, &*u);
    let op = ComputeOp::decode(u64::from(u.a));
    if !fgen_supports(op) {
        return SLOW;
    }
    let core = &mut *ctx.core;
    let (x_on, y_on) = enabled(core, u.b, u.flags & UF_SIMD != 0);
    let due = core.irq.due;
    let (mut sx, mut sy) = (0, 0);
    if x_on {
        let mut pe = Pe::load(core, false);
        pe.exec(op);
        sx = pe.sticky();
    }
    if y_on {
        let mut pe = Pe::load(core, true);
        pe.exec(op);
        sy = pe.sticky();
    }
    finish(ctx, u, core, due, sx, sy)
}

/// `FADD` (`SUB` false) and `FSUB`: `Fn = Fx + Fy` or `Fx - Fy`.
///
/// # Safety
/// As [`fgen`].
pub(super) unsafe extern "C" fn addsub<const SUB: bool>(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    run(
        ctx,
        u,
        |ex, ey, a| add2(ex, ey, a.n, a.x, a.y, SUB),
        |e, a| e.add(a.n, a.x, a.y, SUB),
    )
}

/// `FMUL`: `Fn = Fx * Fy`.
///
/// # Safety
/// As [`fgen`].
pub(super) unsafe extern "C" fn mul(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    run(
        ctx,
        u,
        |ex, ey, a| mul2(ex, ey, a.n, a.x, a.y),
        |e, a| e.mul(a.n, a.x, a.y),
    )
}

/// `FAVG`: `Fn = (Fx + Fy) / 2`.
///
/// # Safety
/// As [`fgen`].
pub(super) unsafe extern "C" fn avg(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    run(
        ctx,
        u,
        |ex, ey, a| avg2(ex, ey, a.n, a.x, a.y),
        |e, a| e.avg(a.n, a.x, a.y),
    )
}

/// `FCMP`: `COMP(Fx, Fy)`, the flags and CACC.
///
/// # Safety
/// As [`fgen`].
pub(super) unsafe extern "C" fn cmp(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    run(
        ctx,
        u,
        |ex, ey, a| {
            ex.cmp(a.x, a.y);
            ey.cmp(a.x, a.y);
        },
        |e, a| e.cmp(a.x, a.y),
    )
}

/// `FSEL`: `COPYSIGN`, `MIN`, `MAX` or `CLIP` (`p` 0..=3).
///
/// # Safety
/// As [`fgen`].
pub(super) unsafe extern "C" fn sel(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    if unpack((*u).a).p > 3 {
        return SLOW;
    }
    run(
        ctx,
        u,
        |ex, ey, a| {
            ex.select(a.n, a.x, a.y, a.p as u8);
            ey.select(a.n, a.x, a.y, a.p as u8);
        },
        |e, a| e.select(a.n, a.x, a.y, a.p as u8),
    )
}

/// `FABSNEG`: `PASS` (`p` 0), negate (1) or `ABS` (2) of `Fx`.
///
/// # Safety
/// As [`fgen`].
pub(super) unsafe extern "C" fn absneg(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    if unpack((*u).a).p > 2 {
        return SLOW;
    }
    run(
        ctx,
        u,
        |ex, ey, a| {
            ex.sign(a.n, a.x, a.p as u8);
            ey.sign(a.n, a.x, a.p as u8);
        },
        |e, a| e.sign(a.n, a.x, a.p as u8),
    )
}

/// Whether `op` is a float-to-integer, integer-to-float, round or scale opcode `FCONV` runs.
/// VERIFIED: the opcode numbers of `Core::float_alu`, by `conversion_opcodes_match_the_kernels`.
pub(in crate::core) const fn conv_opcode(op: u32) -> bool {
    matches!(op, 0xA5 | 0xBD | 0xC9 | 0xCA | 0xCD | 0xD9 | 0xDA | 0xDD)
}

/// `FCONV`: `RND`, `SCALB`, `FIX`, `TRUNC`, `FLOAT`; the ALU opcode is `p`.
///
/// # Safety
/// As [`fgen`].
pub(super) unsafe extern "C" fn conv(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    if !conv_opcode(unpack((*u).a).p) {
        return SLOW;
    }
    let op = |a: super::run::Args| ComputeOp::FloatAlu {
        op: a.p as u8,
        n: a.n,
        x: a.x,
        y: a.y,
    };
    run(
        ctx,
        u,
        |ex, ey, a| {
            ex.run(op(a));
            ey.run(op(a));
        },
        |e, a| e.run(op(a)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::exec::pe::kernels_support;

    #[test]
    fn conversion_opcodes_match_the_kernels() {
        for op in 0..=0xFFu32 {
            let float_alu = ComputeOp::FloatAlu {
                op: op as u8,
                n: 0,
                x: 0,
                y: 0,
            };
            if conv_opcode(op) {
                assert!(kernels_support(float_alu), "{op:#x}");
            }
        }
        // everything the kernels run that is not a conversion has another kind
        let others = [
            0x81u32, 0x82, 0x89, 0x8A, 0xA1, 0xA2, 0xB0, 0xE0, 0xE1, 0xE2, 0xE3,
        ];
        assert!(others.iter().all(|&op| !conv_opcode(op)));
        assert_eq!((0..=0xFFu32).filter(|&op| conv_opcode(op)).count(), 8);
    }
}
