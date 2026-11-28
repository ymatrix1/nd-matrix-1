//! Handlers of the float range `0x100..0x200`.
//!
//! OWNER: W2-FLOAT. Float compute on PEx and, in SIMD mode, PEy, in place on the core
//! (`exec::pe::Elem`), through the exact kernels of `kernels/` and, where one exists, the paired
//! kernel that does both elements at once. The kinds, their operands and what the lowering
//! (`lower/float`) emits for them:
//!
//! | Kind | Instruction | `a` | `b` |
//! |---|---|---|---|
//! | [`FGEN`] | `RECIPS`, `RSQRTS`, float multifunction (through `Pe::exec`) | the 23-bit compute field | condition |
//! | [`FADD`], [`FSUB`] | `Fn = Fx + Fy`, `Fx - Fy` | [`pack`]`(n, x, y, 0)` | condition |
//! | [`FMUL`] | `Fn = Fx * Fy` | `pack(n, x, y, 0)` | condition |
//! | [`FAVG`] | `Fn = (Fx + Fy) / 2` | `pack(n, x, y, 0)` | condition |
//! | [`FCMP`] | `COMP(Fx, Fy)` | `pack(0, x, y, 0)` | condition |
//! | [`FSEL`] | `COPYSIGN`, `MIN`, `MAX`, `CLIP` | `pack(n, x, y, 0..=3)` | condition |
//! | [`FABSNEG`] | `PASS`, `-`, `ABS` | `pack(n, x, 0, 0..=2)` | condition |
//! | [`FCONV`] | `RND`, `SCALB`, `FIX`, `TRUNC`, `FLOAT` | `pack(n, x, y, opcode)` | condition |
//!
//! `b` is the condition code of `IF cond` (31 = `TRUE`, and every unconditional compute). A uop
//! of a block translated for PEy on carries `UF_SIMD` and also runs on PEy, with the condition
//! evaluated on PEy's own flags (`Core::cond_for`). Every uop is one step and one instruction.
//!
//! Contract with the driver (design 4.5, 11.1):
//!
//! * A handler checks its operands before it writes and answers `Exit::Slow` on a bad one, with
//!   the core untouched. The lowering never emits one.
//! * A float op never faults: `Exit::Fault` is not returned.
//! * A uop that raised an arithmetic-exception bit has poked the interrupt controller
//!   (`irq.due = 0`): it returns `Exit::Jump` with `ctx.next_pc` the pc of the next uop (see
//!   `run::poked`), unless it is the last uop of its block or carries `UF_FIN`. The block entry
//!   guard keeps these blocks out while an arithmetic-exception interrupt is enabled
//!   (`GuardClass::Arith`, set for every uop of this range).
//! * The float extensions (tag and low byte) are left exactly as the interpreter leaves them: a
//!   float result writes tag and byte, an integer result (`FIX`, `TRUNC`) clears the byte and keeps
//!   the tag, an op that does not write a register does not touch them.
//!
//! Tests: `diff.rs` runs every kind against `Core::step` on random full states, `block.rs` runs
//! translated blocks of float instructions against the interpreter. Provenance: VERIFIED equal to
//! the interpreter by those tests; the operand layouts and kind numbers are this file's own.
mod handlers;
mod run;

#[cfg(test)]
mod block;
#[cfg(test)]
mod diff;
#[cfg(test)]
pub(in crate::core) mod support;

use crate::core::uop::types::{Handler, UopKind};

pub(in crate::core) use handlers::conv_opcode;
pub(in crate::core) use run::{fgen_supports, fgen_word, pack, unpack, Args};

/// Any float op [`fgen_supports`] lists, through `Pe::exec`. VERIFIED: equal to `Core::step`, hidden
/// extension included, by `fgen_ops_equal_interpreter` (PEx and PEy, `RECIPS`, `RSQRTS` and the
/// float multifunction forms), max err 0. The number is an internal table index (offset 0 of the
/// float range), checked by `every_kind_is_registered_once_inside_the_float_range`.
pub(in crate::core) const FGEN: UopKind = UopKind(UopKind::FLOAT_BASE);
/// `Fn = Fx + Fy`. VERIFIED: equal to `Core::step` by `float_kinds_equal_interpreter_on_random_states`
/// (value, hidden tag and lo, flags, sticky bits, PEx and PEy), max err 0. Index: offset 1.
pub(in crate::core) const FADD: UopKind = UopKind(UopKind::FLOAT_BASE + 1);
/// `Fn = Fx - Fy`. VERIFIED: as [`FADD`], by the same test. Index: offset 2.
pub(in crate::core) const FSUB: UopKind = UopKind(UopKind::FLOAT_BASE + 2);
/// `Fn = Fx * Fy`. VERIFIED: as [`FADD`], by the same test. Index: offset 3.
pub(in crate::core) const FMUL: UopKind = UopKind(UopKind::FLOAT_BASE + 3);
/// `Fn = (Fx + Fy) / 2`. VERIFIED: as [`FADD`], by the same test. Index: offset 4.
pub(in crate::core) const FAVG: UopKind = UopKind(UopKind::FLOAT_BASE + 4);
/// `COMP(Fx, Fy)`. VERIFIED: as [`FADD`], by the same test (flags and CACC). Index: offset 5.
pub(in crate::core) const FCMP: UopKind = UopKind(UopKind::FLOAT_BASE + 5);
/// `COPYSIGN`, `MIN`, `MAX`, `CLIP`. VERIFIED: as [`FADD`], by the same test (the four variants
/// of `p`). Index: offset 6.
pub(in crate::core) const FSEL: UopKind = UopKind(UopKind::FLOAT_BASE + 6);
/// `RND`, `SCALB`, `FIX`, `TRUNC`, `FLOAT`. VERIFIED: as [`FADD`], by the same test (the eight
/// opcodes of `conv_opcode`, `conversion_opcodes_match_the_kernels`). Index: offset 7.
pub(in crate::core) const FCONV: UopKind = UopKind(UopKind::FLOAT_BASE + 7);
/// `PASS`, `-`, `ABS`. VERIFIED: as [`FADD`], by the same test (the three variants of `p`). Index:
/// offset 8.
pub(in crate::core) const FABSNEG: UopKind = UopKind(UopKind::FLOAT_BASE + 8);

/// Every kind of this range, in numeric order.
pub(in crate::core) const KINDS: [UopKind; 9] =
    [FGEN, FADD, FSUB, FMUL, FAVG, FCMP, FSEL, FCONV, FABSNEG];

/// Fills the float range of `t`.
pub(crate) fn register(t: &mut [Handler]) {
    let at = |k: UopKind| usize::from(k.0);
    t[at(FGEN)] = handlers::fgen;
    t[at(FADD)] = handlers::addsub::<false>;
    t[at(FSUB)] = handlers::addsub::<true>;
    t[at(FMUL)] = handlers::mul;
    t[at(FAVG)] = handlers::avg;
    t[at(FCMP)] = handlers::cmp;
    t[at(FSEL)] = handlers::sel;
    t[at(FCONV)] = handlers::conv;
    t[at(FABSNEG)] = handlers::absneg;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::{bail_handler, KindFamily};

    #[test]
    fn every_kind_is_registered_once_inside_the_float_range() {
        let marker: Handler = bail_handler;
        let mut t = vec![marker; usize::from(UopKind::MAX)];
        register(&mut t);
        let range = KindFamily::Float.range();
        let mut seen = Vec::new();
        for (i, h) in t.iter().enumerate() {
            let set = *h as usize != marker as usize;
            assert_eq!(
                set,
                KINDS.iter().any(|k| usize::from(k.0) == i),
                "slot {i:#x}"
            );
            if set {
                assert!(range.contains(&i), "slot {i:#x} is outside the float range");
                seen.push(*h as usize);
            }
        }
        assert_eq!(seen.len(), KINDS.len());
        // ADD and SUB are two instances of one generic function: distinct code, distinct address
        // is not guaranteed (identical code folding), so only the count and the range are pinned
        assert!(KINDS.iter().all(|k| k.family() == Some(KindFamily::Float)));
        let mut ids: Vec<u16> = KINDS.iter().map(|k| k.0).collect();
        ids.dedup();
        assert_eq!(ids.len(), KINDS.len(), "kinds are distinct and ascending");
    }
}
