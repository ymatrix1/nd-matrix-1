//! Dual data moves (types 1a and 1b without a compute): one DM and one PM access in one
//! instruction, both post-modified.
//!
//! OWNER: W2-MOVE, from `Core::exec_dual_move_op`. Both accesses use the indices as they were
//! before the instruction, a store reads its register before either access writes (so
//! `R0 = DM(..), PM(..) = R0` stores the old R0), the DM access runs before the PM access, the
//! broadcast loads follow both accesses (DM first), and both indices are updated last. The data
//! registers are `R0..R15`, so a SIMD access moves the `S` twin to `addr + 1`. A dual move with a
//! compute stays an `Interp` uop (it needs the compute family's handlers, see `lower/moves`).
use super::layout::{dual_sides, Side, CLASS_R, DUAL};
use super::xfer::{access, capture, plain};
use crate::core::uop::exec::mem::Mem;
use crate::core::uop::types::{Exit, Handler, Uop, UopCtx, UF_SIMD};

const FAULT: u32 = Exit::Fault as u32;
const SLOW: u32 = Exit::Slow as u32;

/// The access of one side: `access` with the direction chosen at run time.
#[inline(always)]
fn side_access(
    m: &mut Mem<'_>,
    s: Side,
    addr: u32,
    simd: bool,
    cap: (u32, u32),
) -> Result<(), crate::core::uop::exec::mem::Bad> {
    if s.write {
        access::<true, CLASS_R>(m, addr, s.reg, simd, cap)
    } else {
        access::<false, CLASS_R>(m, addr, s.reg, simd, cap)
    }
}

/// Whether every access of one side is plain.
#[inline(always)]
fn side_plain(m: &Mem<'_>, s: Side, addr: u32, simd: bool) -> bool {
    if s.write {
        plain::<true, CLASS_R>(m, addr, simd)
    } else {
        plain::<false, CLASS_R>(m, addr, simd)
    }
}

/// The dual move uop (kind [`DUAL`]).
///
/// # Safety
/// The handler ABI: `ctx` and `u` are valid and `ctx.core` is the live core.
#[inline(always)]
pub(super) unsafe fn dual(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    let mut m = Mem::new(ctx, u);
    let simd = m.u.flags & UF_SIMD != 0;
    let (dm, pm) = dual_sides(m.u.a);
    let (di, pi) = (usize::from(dm.i), usize::from(pm.i));
    let (dm_addr, pm_addr) = (
        m.core.dag_out(di, m.core.i[di]),
        m.core.dag_out(pi, m.core.i[pi]),
    );
    if m.fin() && !(side_plain(&m, dm, dm_addr, simd) && side_plain(&m, pm, pm_addr, simd)) {
        return SLOW;
    }
    // after the preflight: a wrap of I7 / I15 is noted here, a `Slow` uop must not have noted it
    let (dm_next, pm_next) = (
        m.core.modified(di, usize::from(dm.m)),
        m.core.modified(pi, usize::from(pm.m)),
    );
    let (dm_cap, pm_cap) = (
        capture::<CLASS_R>(m.core, dm.reg),
        capture::<CLASS_R>(m.core, pm.reg),
    );
    if side_access(&mut m, dm, dm_addr, simd, dm_cap).is_err()
        || side_access(&mut m, pm, pm_addr, simd, pm_cap).is_err()
    {
        return FAULT;
    }
    m.core.broadcast_load(di, dm.reg, dm.write);
    m.core.broadcast_load(pi, pm.reg, pm.write);
    m.core.i[di] = dm_next;
    m.core.i[pi] = pm_next;
    m.finish(dm.write || pm.write)
}

unsafe extern "C" fn handler(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    dual(ctx, u)
}

/// Fills the dual move kind of `t`.
pub(super) fn register(t: &mut [Handler]) {
    t[usize::from(DUAL.0)] = handler;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::bail_handler;

    #[test]
    fn the_dual_move_kind_is_registered() {
        let marker: Handler = bail_handler;
        let mut t = vec![marker; 0x500];
        register(&mut t);
        let changed: Vec<usize> = (0..t.len())
            .filter(|&i| t[i] as usize != marker as usize)
            .collect();
        assert_eq!(changed, [0x32A]);
    }
}
