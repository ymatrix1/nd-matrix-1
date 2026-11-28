//! The transfer handlers: a universal register loaded from or stored to data memory through a
//! DAG index, a modify register, an immediate or an absolute address (types 3, 4, 14a, 15).
//!
//! OWNER: W2-MOVE. One body, [`xfer`], instantiated 26 times (`write`, address mode, register
//! class) by const generics, so the address mode and the register class cost no branch at run
//! time. The run-time inputs are the SIMD flag of the uop (`UF_SIMD`: the mode baked into the
//! block), `MODE1` (bit-reversed output of `I0`/`I8`, circular buffers, broadcast loads, which
//! the block cannot know) and the register values.
//!
//! The body follows `Core::exec_move_op` / `exec_move` / `mem_transfer_with` step by step, and
//! that order matters, so each step is named here and pinned by a test:
//!
//! 1. address and update first (`dag_out`, `modified`: the update is computed from the OLD index,
//!    and a wrap of I7 or I15 is noted in `cb_wrap` here);
//! 2. a store captures its register(s) before anything is written (`Core::capture`);
//! 3. the access: PEx word at `addr`, then in SIMD mode the PEy word at `addr + 1`: a load writes
//!    the register and then its twin, a store writes `addr` and then `addr + 1`, for every
//!    register class (a non-data register stores its own value twice, and loads once);
//! 4. the broadcast load of `I1` / `I9` (it copies the loaded register to its complement and
//!    overwrites the twin the SIMD access just wrote);
//! 5. the index update last, so a load into the index register itself loses to the update.
//!
//! Exits: `Ok`; `Smc` or `Jump` through `Mem::finish`; `Slow` before anything is mutated when the
//! uop carries `UF_FIN` and an access would not be plain (it cannot return `Smc` or `Jump`);
//! `Fault` when `dm_read` / `dm_write` fail on the first access (they never do today).
use super::super::mem::{Bad, Mem};
use super::layout::{
    a_ii, a_mi, a_ureg, xfer_kind, CLASS_OTHER, CLASS_R, CLASS_S, DIRECT, MODES, POST_IMM, POST_M,
    PRE_IMM, PRE_M,
};
use crate::core::uop::types::{Exit, Handler, Uop, UopCtx, UF_SIMD};
use crate::core::Core;

const FAULT: u32 = Exit::Fault as u32;
const SLOW: u32 = Exit::Slow as u32;

/// The address of the access.
#[inline(always)]
fn address<const A: u8>(core: &Core, a: u32, b: u32) -> u32 {
    let (ii, mi) = (a_ii(a), a_mi(a));
    match A {
        POST_M | POST_IMM => core.dag_out(ii, core.i[ii]),
        PRE_M => core.dag_out(ii, core.i[ii].wrapping_add(core.m[mi])),
        PRE_IMM => core.dag_out(ii, core.i[ii].wrapping_add(b)),
        _ => b,
    }
}

/// The value the index register takes after a post-modify access. A wrap of `I7` / `I15` is
/// noted in `cb_wrap` here, so this runs only once the uop is sure to run (after the `UF_FIN`
/// preflight: a uop that answers `Slow` leaves the core as it found it).
#[inline(always)]
fn update<const A: u8>(core: &Core, a: u32, b: u32) -> u32 {
    if A == POST_M {
        core.modified(a_ii(a), a_mi(a))
    } else {
        core.modified_by(a_ii(a), b)
    }
}

/// The values a store of `ureg` writes: PEx's and PEy's (`Core::capture` for a register below the
/// system registers).
#[inline(always)]
pub(super) fn capture<const D: u8>(core: &Core, ureg: u8) -> (u32, u32) {
    let n = usize::from(ureg & 15);
    match D {
        CLASS_R => (core.r[n], core.s[n]),
        CLASS_S => (core.s[n], core.r[n]),
        _ => {
            let v = core.read_ureg(ureg);
            (v, v)
        }
    }
}

/// Loads `v` into the register (`Core::write_ureg`).
#[inline(always)]
fn put<const D: u8>(core: &mut Core, ureg: u8, v: u32) {
    let n = usize::from(ureg & 15);
    match D {
        CLASS_R => {
            core.r[n] = v;
            core.fx.clear(n);
        }
        CLASS_S => {
            core.s[n] = v;
            core.fx_y.clear(n);
        }
        _ => core.write_ureg(ureg, v),
    }
}

/// Loads `v` into the PEy twin of the register.
#[inline(always)]
fn put_twin<const D: u8>(core: &mut Core, ureg: u8, v: u32) {
    let n = usize::from(ureg & 15);
    if D == CLASS_R {
        core.s[n] = v;
        core.fx_y.clear(n);
    } else {
        core.r[n] = v;
        core.fx.clear(n);
    }
}

/// Whether every access of the transfer is plain (fast path, no hit in translated code): what a
/// `UF_FIN` uop requires before it mutates anything.
#[inline(always)]
pub(super) fn plain<const W: bool, const D: u8>(m: &Mem<'_>, addr: u32, simd: bool) -> bool {
    let second = simd && (W || D != CLASS_OTHER);
    if W {
        m.can_write(addr) && (!second || m.can_write(addr.wrapping_add(1)))
    } else {
        m.can_read(addr) && (!second || m.can_read(addr.wrapping_add(1)))
    }
}

/// The access of `mem_transfer_with`: store the captured values (both PEs in SIMD mode) or load
/// the register and, in SIMD mode, its twin.
#[inline(always)]
pub(super) fn access<const W: bool, const D: u8>(
    m: &mut Mem<'_>,
    addr: u32,
    ureg: u8,
    simd: bool,
    cap: (u32, u32),
) -> Result<(), Bad> {
    if W {
        m.wr(addr, cap.0)?;
        if simd {
            m.wr(addr.wrapping_add(1), cap.1)?;
        }
    } else {
        let v = m.rd(addr)?;
        put::<D>(m.core, ureg, v);
        if simd && D != CLASS_OTHER {
            let v = m.rd(addr.wrapping_add(1))?;
            put_twin::<D>(m.core, ureg, v);
        }
    }
    Ok(())
}

/// The transfer uop: see the module doc. `#[inline(always)]` so the instruction-count probes
/// (`probe.rs`) measure the body a handler really runs.
///
/// # Safety
/// The handler ABI: `ctx` and `u` are valid and `ctx.core` is the live core.
#[inline(always)]
pub(super) unsafe fn xfer<const W: bool, const A: u8, const D: u8>(
    ctx: *mut UopCtx,
    u: *const Uop,
) -> u32 {
    let mut m = Mem::new(ctx, u);
    let (a, b) = (m.u.a, m.u.b);
    let simd = m.u.flags & UF_SIMD != 0;
    let (ii, ureg) = (a_ii(a), a_ureg(a));
    let addr = address::<A>(m.core, a, b);
    if m.fin() && !plain::<W, D>(&m, addr, simd) {
        return SLOW;
    }
    let next = if A <= POST_IMM {
        update::<A>(m.core, a, b)
    } else {
        0
    };
    let cap = if W {
        capture::<D>(m.core, ureg)
    } else {
        (0, 0)
    };
    if access::<W, D>(&mut m, addr, ureg, simd, cap).is_err() {
        return FAULT;
    }
    if !W && A != DIRECT && D != CLASS_OTHER && (ii == 1 || ii == 9) {
        m.core.broadcast_load(ii, ureg, false);
    }
    if A <= POST_IMM {
        m.core.i[ii] = next;
    }
    m.finish(W)
}

/// The table entry of one transfer kind.
unsafe extern "C" fn handler<const W: bool, const A: u8, const D: u8>(
    ctx: *mut UopCtx,
    u: *const Uop,
) -> u32 {
    xfer::<W, A, D>(ctx, u)
}

/// Registers one transfer kind.
macro_rules! kind {
    ($t:ident, $w:expr, $a:expr, $d:expr) => {
        $t[usize::from(xfer_kind($w, $a, $d).0)] = handler::<{ $w }, { $a }, { $d }>;
    };
}

/// Registers the three register classes of one (`write`, mode).
macro_rules! row {
    ($t:ident, $w:expr, $a:expr) => {
        kind!($t, $w, $a, CLASS_R);
        kind!($t, $w, $a, CLASS_S);
        kind!($t, $w, $a, CLASS_OTHER);
    };
}

/// Fills the transfer kinds of `t`: 26 of the 30 numbers of `layout`. The post-modify with an
/// immediate exists only in type 4, whose register field has 4 bits (`R0..R15`), so its `S` and
/// other-register kinds cannot be made by any instruction and have no handler.
pub(super) fn register(t: &mut [Handler]) {
    row!(t, false, POST_M);
    kind!(t, false, POST_IMM, CLASS_R);
    row!(t, false, PRE_M);
    row!(t, false, PRE_IMM);
    row!(t, false, DIRECT);
    row!(t, true, POST_M);
    kind!(t, true, POST_IMM, CLASS_R);
    row!(t, true, PRE_M);
    row!(t, true, PRE_IMM);
    row!(t, true, DIRECT);
    const _: () = assert!(MODES == 5);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::uop::types::bail_handler;

    #[test]
    fn every_reachable_transfer_kind_gets_a_handler_and_nothing_else_changes() {
        let marker: Handler = bail_handler;
        let mut t = vec![marker; 0x500];
        register(&mut t);
        let changed: Vec<usize> = (0..t.len())
            .filter(|&i| t[i] as usize != marker as usize)
            .collect();
        // the numbers 0x300..0x31E, but not the post-modify with an immediate in the S and other
        // classes (modes are numbered 3 apart: write 0, mode 1 is 0x303.., write 1 is 0x312..)
        let unreachable = [0x304, 0x305, 0x313, 0x314];
        let want: Vec<usize> = (0..30)
            .map(|k| 0x300 + k)
            .filter(|k| !unreachable.contains(k))
            .collect();
        assert_eq!(changed, want);
        for (write, class) in [
            (false, CLASS_S),
            (false, CLASS_OTHER),
            (true, CLASS_S),
            (true, CLASS_OTHER),
        ] {
            let k = usize::from(xfer_kind(write, POST_IMM, class).0);
            assert!(unreachable.contains(&k), "{k:#x}");
        }
    }

    /// The address and update of every mode on a hand-made core (the formulas of `exec_move`).
    #[test]
    fn address_modes_follow_the_interpreter() {
        let mut c = core_with(&[0x0001]);
        (c.i[3], c.m[5], c.l[3], c.b[3]) = (0x9_1000, 4, 0, 0);
        let a = super::super::layout::xfer_a(3, 5, 0, 1);
        assert_eq!(address::<POST_M>(&c, a, 0), 0x9_1000);
        assert_eq!(update::<POST_M>(&c, a, 0), 0x9_1004);
        assert_eq!(address::<POST_IMM>(&c, a, 0xFFFF_FFFE), 0x9_1000);
        assert_eq!(update::<POST_IMM>(&c, a, 0xFFFF_FFFE), 0x9_0FFE);
        assert_eq!(address::<PRE_M>(&c, a, 0), 0x9_1004);
        assert_eq!(address::<PRE_IMM>(&c, a, 7), 0x9_1007);
        assert_eq!(address::<DIRECT>(&c, a, 0x1234), 0x1234);
        // bit-reversed output: only I0 with BR0 (MODE1 bit 1) and I8 with BR8 (bit 0)
        c.mode1 = 0b11;
        c.i[0] = 0x8_3000;
        assert_eq!(address::<POST_M>(&c, 0, 0), 0x8_3000u32.reverse_bits());
        c.i[8] = 1;
        assert_eq!(address::<PRE_IMM>(&c, 8, 0), 1u32.reverse_bits());
        assert_eq!(
            address::<PRE_IMM>(&c, 3, 0),
            0x9_1000,
            "other indices are never reversed"
        );
        // only the update notes a wrap of I7 (L7 = 2 at B7: 1 + 1 leaves the buffer)
        (c.i[7], c.l[7], c.b[7], c.mode1, c.m[7]) = (0x9_1001, 2, 0x9_1000, 1 << 24, 1);
        let a7 = super::super::layout::xfer_a(7, 7, 0, 1);
        assert_eq!(address::<POST_M>(&c, a7, 0), 0x9_1001);
        assert_eq!(c.cb_wrap.get(), 0, "the address alone notes nothing");
        assert_eq!(update::<POST_M>(&c, a7, 0), 0x9_1000);
        assert_eq!(c.cb_wrap.get(), 1);
    }

    #[test]
    fn capture_and_put_match_the_interpreter_helpers() {
        let mut c = core_with(&[0x0001]);
        (c.r[3], c.s[3], c.m[2]) = (11, 22, 33);
        assert_eq!(capture::<CLASS_R>(&c, 3), c.capture(3));
        assert_eq!(capture::<CLASS_S>(&c, 0x53), c.capture(0x53));
        assert_eq!(capture::<CLASS_OTHER>(&c, 0x22), c.capture(0x22));
        let mut d = core_with(&[0x0001]);
        c.fx.lo[3] = 9;
        d.fx.lo[3] = 9;
        put::<CLASS_R>(&mut c, 3, 5);
        d.write_ureg(3, 5);
        put_twin::<CLASS_R>(&mut c, 3, 6);
        d.write_ureg(0x53, 6);
        put::<CLASS_OTHER>(&mut c, 0x41, 7);
        d.write_ureg(0x41, 7);
        assert_eq!((c.r, c.s, c.i, c.b), (d.r, d.s, d.i, d.b));
        assert_eq!((c.fx.lo, c.fx_y.lo), (d.fx.lo, d.fx_y.lo));
        assert_eq!(c.i[1], 7, "loading B1 loads I1 as well");
    }
}
