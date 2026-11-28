//! Immediate loads and stores and the index modify instructions: `Un = <imm>` (type 17),
//! `DM/PM(Ii, Mm) = <imm>` (type 16), `MODIFY(Is, Mm)` (type 7) and `MODIFY` / `BITREV(Is, <imm>)`
//! (type 19a).
//!
//! OWNER: W2-MOVE. Each handler follows its interpreter arm (`exec_move_op`, `exec_move`,
//! `exec_modify_imm`):
//!
//! * the immediate load writes the register and, in SIMD mode, its PEy twin with the same value;
//! * the immediate store reads the address and the update before it writes, stores the value at
//!   `addr` and, in SIMD mode, at `addr + 1`, and updates the index last (post-modify, circular
//!   when enabled);
//! * the modifies always honour `L` and `B` (`modified_always`), and `BITREV` adds and reverses
//!   all 32 bits. The destination may differ from the source, which is why they are not a
//!   transfer.
use super::super::mem::Mem;
use super::layout::{
    a_ii, a_mi, a_ureg, load_imm_kind, BITREV_IMM, CLASS_OTHER, CLASS_R, CLASS_S, IMM_STORE,
    MODIFY, MODIFY_IMM,
};
use crate::core::uop::types::{Exit, Handler, Uop, UopCtx, UF_SIMD};

const OK: u32 = Exit::Ok as u32;
const FAULT: u32 = Exit::Fault as u32;
const SLOW: u32 = Exit::Slow as u32;

/// `Un = <imm>` for register class `D` (system registers are left to the interpreter).
///
/// # Safety
/// The handler ABI: `ctx` and `u` are valid and `ctx.core` is the live core.
#[inline(always)]
pub(super) unsafe fn load_imm<const D: u8>(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    let core = &mut *(*ctx).core;
    let u = &*u;
    let (ureg, v) = (a_ureg(u.a), u.b);
    let n = usize::from(ureg & 15);
    let simd = u.flags & UF_SIMD != 0;
    match D {
        CLASS_R => {
            core.r[n] = v;
            core.fx.clear(n);
            if simd {
                core.s[n] = v;
                core.fx_y.clear(n);
            }
        }
        CLASS_S => {
            core.s[n] = v;
            core.fx_y.clear(n);
            if simd {
                core.r[n] = v;
                core.fx.clear(n);
            }
        }
        _ => core.write_ureg(ureg, v),
    }
    OK
}

unsafe extern "C" fn load_imm_handler<const D: u8>(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    load_imm::<D>(ctx, u)
}

/// `DM/PM(Ii, Mm) = <imm>` with post-modify.
///
/// # Safety
/// The handler ABI: `ctx` and `u` are valid and `ctx.core` is the live core.
#[inline(always)]
pub(super) unsafe fn imm_store(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    let mut m = Mem::new(ctx, u);
    let (a, v) = (m.u.a, m.u.b);
    let simd = m.u.flags & UF_SIMD != 0;
    let (ii, mi) = (a_ii(a), a_mi(a));
    let addr = m.core.dag_out(ii, m.core.i[ii]);
    if m.fin() && !(m.can_write(addr) && (!simd || m.can_write(addr.wrapping_add(1)))) {
        return SLOW;
    }
    // after the preflight: a wrap of I7 / I15 is noted here, a `Slow` uop must not have noted it
    let next = m.core.modified(ii, mi);
    if m.wr(addr, v).is_err() {
        return FAULT;
    }
    if simd && m.wr(addr.wrapping_add(1), v).is_err() {
        return FAULT;
    }
    m.core.i[ii] = next;
    m.finish(true)
}

unsafe extern "C" fn imm_store_handler(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    imm_store(ctx, u)
}

/// `Id = MODIFY(Is, Mm)`: `src`, `dst` and `mi` from `a`.
unsafe extern "C" fn modify(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    let core = &mut *(*ctx).core;
    let a = (*u).a;
    let (src, dst, mi) = (a_ii(a), a_mi(a), (a >> 8 & 15) as usize);
    core.i[dst] = core.modified_always(src, core.m[mi]);
    OK
}

/// `Id = MODIFY(Is, <imm>)`.
unsafe extern "C" fn modify_imm(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    let core = &mut *(*ctx).core;
    let (a, data) = ((*u).a, (*u).b);
    let (src, dst) = (a_ii(a), a_mi(a));
    core.i[dst] = core.modified_always(src, data);
    OK
}

/// `Id = BITREV(Is, <imm>)`.
unsafe extern "C" fn bitrev_imm(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    let core = &mut *(*ctx).core;
    let (a, data) = ((*u).a, (*u).b);
    let (src, dst) = (a_ii(a), a_mi(a));
    core.i[dst] = core.i[src].wrapping_add(data).reverse_bits();
    OK
}

/// Fills the immediate and modify kinds of `t`.
pub(super) fn register(t: &mut [Handler]) {
    t[usize::from(load_imm_kind(CLASS_R).0)] = load_imm_handler::<CLASS_R>;
    t[usize::from(load_imm_kind(CLASS_S).0)] = load_imm_handler::<CLASS_S>;
    t[usize::from(load_imm_kind(CLASS_OTHER).0)] = load_imm_handler::<CLASS_OTHER>;
    t[usize::from(IMM_STORE.0)] = imm_store_handler;
    t[usize::from(MODIFY.0)] = modify;
    t[usize::from(MODIFY_IMM.0)] = modify_imm;
    t[usize::from(BITREV_IMM.0)] = bitrev_imm;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::bail_handler;

    #[test]
    fn the_immediate_and_modify_kinds_are_registered() {
        let marker: Handler = bail_handler;
        let mut t = vec![marker; 0x500];
        register(&mut t);
        let changed: Vec<usize> = (0..t.len())
            .filter(|&i| t[i] as usize != marker as usize)
            .collect();
        assert_eq!(
            changed,
            [0x320, 0x321, 0x322, 0x324, 0x325, 0x326, 0x327],
            "three register classes, the store and the three modifies"
        );
    }
}
