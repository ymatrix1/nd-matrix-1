//! Handlers of the fixed-point range `0x200..0x300`.
//!
//! OWNER: W2-FIXED. One handler per kind (design P11: one concern per file):
//!
//! | File | Content |
//! |---|---|
//! | `kinds.rs` | the kind numbers, the operand layout of each uop and the field helpers |
//! | `eff.rs` | [`eff::Eff`], the effect of one fixed-point compute on one element, computed from a read-only view and applied by `commit` |
//! | `alu.rs` | the 21 ALU opcodes of `Core::fixed_alu` |
//! | `mult.rs` | the multiplier (products, `MRx = 0`, `SAT`, `RND`) and the MR register transfers |
//! | `shift.rs` | the 21 shifter opcodes that do not touch the bit FIFO, register and immediate form |
//! | `multi.rs` | the dual add/subtract and the multifunction forms |
//! | `run.rs` | the handler: conditions, effects of PEx and PEy, the bail, the commit |
//! | `fixture.rs`, `diff.rs`, `diff_sweep.rs`, `diff_ops.rs`, `diff_wide.rs`, `diff_sticky.rs` | test build only: the state filler and program generators, the handler-versus-`Core::step` rig, its sweeps and counters, and the acceptance tests per unit |
//!
//! Exactness: every handler computes the effects of both elements from the entry state and
//! returns `Exit::Slow` BEFORE writing anything when an effect would raise an arithmetic-exception
//! interrupt (STKY AOS, MOS, MVS, MUS), so the interpreter does the raise (`raise_interrupt`
//! pokes `irq.due`, which a handler must not). The lowering (`lower/fixed/`) declines every word
//! whose semantics are not reproduced here, so it stays an `Interp` uop. The tests run every kind
//! against `Core::step` on random full states, hidden state included, SIMD on and off, and run
//! each bailing case again through the same handler without the bail (`run::forced`, test build
//! only), which proves the saturated and wrapped values the interpreter produces for those cases.
//!
//! The bail is exact and not handled in the handler, also when the sticky bit is already latched:
//! the interpreter raises the interrupt on every event (`note_arith_exception_in`: it ORs the bit
//! into STKY and `raise_interrupt` sets the latch and pokes `irq.due` to 0 again, PRM A-21), so a
//! latched bit is not a no-op and `exact_diff` sees the poke (`diff_sticky.rs`,
//! `an_already_latched_sticky_bit_still_pokes_the_controller_so_it_is_not_a_no_op`). Frequency,
//! MEASURED by `diff_sticky.rs` on the census generator (`verify::gen::census_encodings` of the
//! ALU, multiplier and shifter forms, 2 seeds, both PEy modes, edge-biased random states and
//! random STKY): 3 of 480 cases bail (0.6%), 1 of those on bits STKY already held; on random ALU
//! programs of the same rig, which overflow far more often than audio code: 24 of 840 (2.9%), 13
//! on latched bits. No firmware share is known (none can be measured without firmware); the cost
//! of a bail is one `Slow` exit and one `Core::step`, counted by the G2 `Slow` row. If a real
//! pass shows many, the exact alternative is the float family's: raise through the interpreter's
//! own function in the handler and hand over (`h_float/run.rs`).
pub mod kinds;

mod alu;
mod eff;
mod mult;
mod multi;
mod run;
mod shift;

#[cfg(test)]
mod diff;
#[cfg(test)]
mod diff_ops;
#[cfg(test)]
mod diff_sticky;
#[cfg(test)]
mod diff_sweep;
#[cfg(test)]
mod diff_wide;
#[cfg(test)]
mod fixture;

pub use multi::{MF2_ALU, MF2_MULT, MF3_MULT};
pub use shift::SHIFT_OPS;

use super::super::types::{Handler, UopKind};
use kinds::{XALU_BASE, XDUAL_K, XMF2_K, XMF3_K, XMRT_K, XMULS_K, XMUL_K, XSHIFTI_K, XSHIFT_K};

/// Stores `run::$h::<K>` in slot `K` for each kind of the list.
macro_rules! register_kinds {
    ($t:expr, $h:ident; $($k:expr),+ $(,)?) => {
        $( $t[usize::from($k)] = run::$h::<{ $k }> as Handler; )+
    };
}

/// Registers the handler function `$h` of `run` for every kind of the family: the ALU opcodes of
/// `kinds::ALU_OPS`, one handler each, and the eight named kinds. The test
/// `registration_covers_exactly_the_kinds` ties this list to `kinds::all_kinds`.
macro_rules! install {
    ($t:expr, $h:ident) => {
        register_kinds!(
            $t, $h;
            XALU_BASE + 0x01, XALU_BASE + 0x02, XALU_BASE + 0x05, XALU_BASE + 0x06,
            XALU_BASE + 0x09, XALU_BASE + 0x0A, XALU_BASE + 0x0B, XALU_BASE + 0x21,
            XALU_BASE + 0x22, XALU_BASE + 0x25, XALU_BASE + 0x26, XALU_BASE + 0x29,
            XALU_BASE + 0x2A, XALU_BASE + 0x30, XALU_BASE + 0x40, XALU_BASE + 0x41,
            XALU_BASE + 0x42, XALU_BASE + 0x43, XALU_BASE + 0x61, XALU_BASE + 0x62,
            XALU_BASE + 0x63, XMUL_K, XMULS_K, XMRT_K, XSHIFT_K, XSHIFTI_K, XDUAL_K, XMF2_K,
            XMF3_K,
        )
    };
}

/// Fills the fixed-point range of `t`.
pub(crate) fn register(t: &mut [Handler]) {
    install!(t, handler);
}

/// The table of the handlers without the bail (test build only, see `run::forced`): the same
/// kinds in the same slots, every other slot the bail handler.
#[cfg(test)]
fn forced_table() -> Vec<Handler> {
    let mut t = vec![crate::core::uop::types::bail_handler as Handler; usize::from(UopKind::MAX)];
    install!(t, forced);
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::{Exit, Uop, UopCtx, UopKind};

    /// Distinct body so the linker cannot fold it with a handler of this family.
    unsafe extern "C" fn marker_handler(_: *mut UopCtx, _: *const Uop) -> u32 {
        0xFEED
    }

    #[test]
    fn registration_covers_exactly_the_kinds() {
        let marker: Handler = marker_handler;
        let mut t = vec![marker; usize::from(UopKind::MAX)];
        register(&mut t);
        let touched: Vec<u16> = t
            .iter()
            .enumerate()
            .filter(|(_, h)| **h as usize != marker as usize)
            .map(|(i, _)| i as u16)
            .collect();
        let mut kinds = kinds::all_kinds();
        kinds.sort_unstable();
        assert_eq!(
            touched, kinds,
            "the fixed range holds the kinds, and only them"
        );
        assert!(touched
            .iter()
            .all(|&k| UopKind(k).family() == Some(crate::core::uop::types::KindFamily::Fixed)));
    }

    #[test]
    fn a_registered_handler_answers_with_an_exit_code() {
        let table = crate::core::uop::hot::UopHot::handler_table();
        let mut c = crate::core::support::core_with(&[0x0001]);
        c.r[1] = 2;
        c.r[2] = 3;
        let u = Uop::new(UopKind::xalu(0x01), 1, c.pc, kinds::word(0x01, 3, 1, 2), 31);
        let steps = c.steps;
        let mut ctx = UopCtx::new(&mut c as *mut crate::core::Core, steps, 4);
        // SAFETY: `ctx.core` is the only live reference to the core during the call.
        let raw = unsafe { table[usize::from(u.h)](&mut ctx, &u) };
        assert_eq!(Exit::from_u32(raw), Some(Exit::Ok));
        assert_eq!(c.r[3], 5);
    }
}
