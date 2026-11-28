//! The handler body: conditions of both elements, the two effects, the bail on an arithmetic
//! exception, the commit.
//!
//! OWNER: W2-FIXED. One monomorphisation of [`handler`] per kind (`K`), so the effect of the kind
//! is inlined and its opcode folded into the code (the ALU kinds are one per opcode).
//!
//! Exactness rules of `Core::exec_cached` kept here:
//! * both conditions are evaluated before either element runs (`cond` of PEx, `cond_for(true)`
//!   of PEy when SIMD); a condition that does not hold changes nothing in that element;
//! * PEx runs, then PEy; the elements share nothing an effect reads or writes, so computing both
//!   effects from the entry state is the same as running them one after the other;
//! * an effect that would raise an arithmetic-exception interrupt (`Eff::sticky != 0`) makes the
//!   handler return `Exit::Slow` BEFORE anything is written (exit protocol rule 1: precondition
//!   before mutate), so `Core::step` runs the instruction and does the raise, the `irq.due` poke
//!   and the sequencer prologue itself.
use super::eff::{commit, view, Eff, View};
use super::kinds::{
    imm_of, op8, XALU_BASE, XDUAL_K, XMF2_K, XMF3_K, XMRT_K, XMULS_K, XMUL_K, XSHIFTI_K, XSHIFT_K,
};
use super::{alu, mult, multi, shift};
use crate::core::uop::types::{Exit, Uop, UopCtx, UF_SIMD};

/// The effect of kind `K` on one element.
#[inline(always)]
fn effect<const K: u16>(v: &View<'_>, u: &Uop) -> Eff {
    match K {
        XMUL_K => mult::product(v, u.a),
        XMULS_K => mult::clear_sat_round(v, u.a),
        XMRT_K => mult::transfer(v, u.a),
        XSHIFT_K => shift::effect(v, op8(u.a), u.a),
        XSHIFTI_K => shift::immediate(v, u.a, imm_of(u.b)),
        XDUAL_K => multi::dual(v, u.a),
        XMF2_K => multi::mf2(v, u.a),
        XMF3_K => multi::mf3(v, u.a),
        k => alu::effect(v, (k - XALU_BASE) as u8, u.a),
    }
}

/// The handler of kind `K` (ABI of `types::Handler`).
///
/// # Safety
/// `ctx` points to a live [`UopCtx`] whose `core` is the one `&mut Core` of the running block and
/// `u` to a uop of kind `K`; nothing else touches the core during the call. The driver and the
/// handler-level tests guarantee both.
pub(super) unsafe extern "C" fn handler<const K: u16>(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    // SAFETY: the contract is the caller's.
    unsafe { exec::<K, true>(ctx, u) }
}

/// [`handler`] without the bail (test build only): the effects are applied whatever they raise.
/// The differential tests use it to compare the arithmetic of the bailing cases (saturated
/// values, flags, spills) with the interpreter's; no handler registered for the engine is one.
///
/// # Safety
/// As [`handler`].
#[cfg(test)]
pub(super) unsafe extern "C" fn forced<const K: u16>(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    // SAFETY: the contract is the caller's.
    unsafe { exec::<K, false>(ctx, u) }
}

/// The body of [`handler`] and [`forced`]: with `BAIL`, an effect that would raise an exception
/// makes the call return `Exit::Slow` before anything is written.
///
/// # Safety
/// As [`handler`].
#[inline(always)]
unsafe fn exec<const K: u16, const BAIL: bool>(ctx: *mut UopCtx, u: *const Uop) -> u32 {
    // SAFETY: see the contract of `handler`.
    let (core, u) = unsafe { (&mut *(*ctx).core, &*u) };
    let cond = u64::from(u.b & 31);
    let ok_x = core.cond(cond);
    let ok_y = u.flags & UF_SIMD != 0 && core.cond_for(true, cond);
    let ex = if ok_x {
        effect::<K>(&view(core, false), u)
    } else {
        Eff::NONE
    };
    let ey = if ok_y {
        effect::<K>(&view(core, true), u)
    } else {
        Eff::NONE
    };
    if BAIL && (ex.sticky | ey.sticky) != 0 {
        return Exit::Slow as u32;
    }
    if ok_x {
        commit(core, false, &ex);
    }
    if ok_y {
        commit(core, true, &ey);
    }
    Exit::Ok as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::uop::exec::h_fixed::kinds::{word, ALU_OPS};
    use crate::core::uop::types::UopKind;
    use crate::core::{Core, MODE1_PEYEN};

    fn call(core: &mut Core, u: &Uop) -> Exit {
        let steps = core.steps;
        let mut ctx = UopCtx::new(core as *mut Core, steps, 4);
        // SAFETY: `ctx.core` is the only live reference to the core during the call.
        let raw = unsafe { super::handler::<{ XALU_BASE + 0x01 }>(&mut ctx, u) };
        Exit::from_u32(raw).expect("a handler returns an exit code")
    }

    #[test]
    fn a_handler_runs_the_effect_of_its_kind_on_the_elements_whose_condition_holds() {
        // R3 = R1 + R2 on PEx and PEy; IF EQ (cond 0) holds on PEx only
        let mut c = core_with(&[0x0001]);
        c.mode1 |= MODE1_PEYEN;
        (c.r[1], c.r[2], c.s[1], c.s[2]) = (4, 5, 6, 7);
        (c.flags.az, c.flags_y.az) = (true, false);
        let mut u = Uop::new(UopKind::xalu(0x01), 1, c.pc, word(0x01, 3, 1, 2), 0);
        u.flags = UF_SIMD;
        assert_eq!(call(&mut c, &u), Exit::Ok);
        assert_eq!((c.r[3], c.s[3]), (9, 0), "PEy's condition does not hold");
        // TRUE on both
        u.b = 31;
        assert_eq!(call(&mut c, &u), Exit::Ok);
        assert_eq!((c.r[3], c.s[3]), (9, 13));
        // without UF_SIMD PEy is left alone even when SIMD is on in the core
        c.s[3] = 0;
        u.flags = 0;
        assert_eq!(call(&mut c, &u), Exit::Ok);
        assert_eq!(c.s[3], 0);
    }

    #[test]
    fn an_exception_on_either_element_bails_before_writing_anything() {
        let mut c = core_with(&[0x0001]);
        c.mode1 |= MODE1_PEYEN;
        // PEx adds fine, PEy overflows: AOS on PEy
        (c.r[1], c.r[2], c.s[1], c.s[2]) = (1, 2, 0x7FFF_FFFF, 1);
        let mut u = Uop::new(UopKind::xalu(0x01), 1, c.pc, word(0x01, 3, 1, 2), 31);
        u.flags = UF_SIMD;
        let r_before = (c.r, c.s, c.flags, c.flags_y, c.sys.astat_extra, c.sys.stky);
        assert_eq!(call(&mut c, &u), Exit::Slow);
        assert_eq!(
            r_before,
            (c.r, c.s, c.flags, c.flags_y, c.sys.astat_extra, c.sys.stky),
            "PEx was not written either"
        );
        assert_eq!((c.irq.irptl, c.irq.lirptl), (0, 0), "nothing raised");
    }

    #[test]
    fn every_kind_has_a_monomorphisation_that_does_not_panic_on_a_zero_uop() {
        let mut c = core_with(&[0x0001]);
        let table = crate::core::uop::hot::UopHot::handler_table();
        for k in super::super::kinds::all_kinds() {
            let u = Uop::new(UopKind(k), 1, c.pc, 0, 31);
            let steps = c.steps;
            let mut ctx = UopCtx::new(&mut c as *mut Core, steps, 4);
            // SAFETY: as in `call`.
            let raw = unsafe { table[usize::from(k)](&mut ctx, &u) };
            assert!(
                matches!(Exit::from_u32(raw), Some(Exit::Ok | Exit::Slow)),
                "kind {k:#x}"
            );
        }
        assert_eq!(ALU_OPS.len() + 8, super::super::kinds::all_kinds().len());
    }
}
