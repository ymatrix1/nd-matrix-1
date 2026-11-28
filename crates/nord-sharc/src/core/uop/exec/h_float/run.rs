//! The shared skeleton of the float handlers: operand packing, the per-element enable (the
//! condition of `IF cond`), the in-place run on PEx and PEy and the exit after an arithmetic
//! exception.
//!
//! OWNER: W2-FLOAT. Every dedicated float handler is `run(ctx, u, pair, one)` with two closures:
//! `one` runs the op on one element, `pair` runs it on both (a paired kernel where one exists).
//! The skeleton does what `Core::run_compute_op` does around the op, in the same order:
//!
//! 1. both conditions are evaluated before anything is written (`cond_ok` for PEx, `cond_y` for
//!    PEy, which also needs SIMD mode: here the `UF_SIMD` flag the block was translated for);
//! 2. the op runs on every enabled element, PEx first;
//! 3. the arithmetic-exception bits of PEx are raised, then those of PEy (`Core::aot_pe_end`), after
//!    the borrow of the elements has ended, exactly as `exec_kernels` does.
//!
//! Exit after an exception. Raising a sticky bit latches its interrupt and pokes the controller
//! (`raise_interrupt` sets `irq.due = 0`) whether or not the interrupt is enabled, and the
//! interpreter's next step then runs its prologue (`sequencer_event`, which recomputes `due`). A
//! block that went on would skip that prologue, so a uop that moved `irq.due` hands over to the
//! driver once it has run (design 4.5 rule 4, [`poked`]). `exact_diff` compares `irq.due`, so
//! this is observable, not only a timing matter.
use crate::core::decode_cache::ComputeOp;
use crate::core::uop::exec::pe::{elems, Elem};
use crate::core::uop::types::{Exit, Uop, UopCtx, UF_FIN, UF_LAST, UF_SIMD};
use crate::core::Core;

/// Operands of a dedicated float uop, as [`pack`] stores them in `Uop::a`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::core) struct Args {
    /// Result register (`Fn`, or `Rn` for FIX and TRUNC).
    pub n: u8,
    /// First operand register.
    pub x: u8,
    /// Second operand register.
    pub y: u8,
    /// Kind-specific parameter: the variant of `FSEL` and `FABSNEG`, the opcode of `FCONV`.
    pub p: u32,
}

/// `Uop::a` of a dedicated float uop: `n` in bits 0..4, `x` in 4..8, `y` in 8..12 and the 20-bit
/// parameter `p` above. VERIFIED by `pack_and_unpack_are_inverse`.
pub(in crate::core) const fn pack(n: u8, x: u8, y: u8, p: u32) -> u32 {
    (n & 15) as u32 | ((x & 15) as u32) << 4 | ((y & 15) as u32) << 8 | (p & 0xF_FFFF) << 12
}

/// The inverse of [`pack`].
#[inline(always)]
pub(in crate::core) const fn unpack(a: u32) -> Args {
    Args {
        n: (a & 15) as u8,
        x: (a >> 4 & 15) as u8,
        y: (a >> 8 & 15) as u8,
        p: a >> 12,
    }
}

/// Whether `FGEN` runs `op` (through `Pe::exec`): the float ALU opcodes the exact kernels do not
/// cover (`RECIPS`, `RSQRTS`) and the float multiplier-plus-ALU multifunction forms `Pe` decodes.
/// VERIFIED equal to the interpreter, hidden extension included, by `fgen_ops_equal_interpreter`.
pub(in crate::core) fn fgen_supports(op: ComputeOp) -> bool {
    match op {
        ComputeOp::FloatAlu { op, .. } => matches!(op, 0xC4 | 0xC5),
        ComputeOp::Multifunction { .. } => crate::core::aot_pe::pe_supports(op),
        _ => false,
    }
}

/// `Uop::a` of an `FGEN` uop: the 23-bit compute field `ComputeOp::decode` reads, `None` when
/// [`fgen_supports`] refuses `op`. VERIFIED: `decode(word) == op` by `fgen_word_decodes_back`.
pub(in crate::core) fn fgen_word(op: ComputeOp) -> Option<u32> {
    if !fgen_supports(op) {
        return None;
    }
    match op {
        ComputeOp::FloatAlu { op, n, x, y } => Some(
            u32::from(op) << 12
                | u32::from(n & 15) << 8
                | u32::from(x & 15) << 4
                | u32::from(y & 15),
        ),
        ComputeOp::Multifunction { c } => Some((c & 0x7F_FFFF) as u32),
        _ => None,
    }
}

/// Which elements the uop runs on: PEx when its condition holds, PEy when SIMD mode is on and its
/// condition holds. `cond` 31 is `TRUE` and needs no flags (`Core::cond_for`). Both conditions
/// read the state before the op writes any of it, like the interpreter.
#[inline(always)]
pub(super) fn enabled(core: &Core, cond: u32, simd: bool) -> (bool, bool) {
    if cond == 31 {
        return (true, simd);
    }
    let c = u64::from(cond);
    (core.cond_for(false, c), simd && core.cond_for(true, c))
}

/// Runs a float uop: `one` on each enabled element, or `pair` when both are enabled, then the
/// sticky bits. Returns the [`Exit`] as `u32`: `Ok`, or `Jump` after an exception that poked the
/// interrupt controller ([`poked`]).
///
/// # Safety
/// `ctx` and `u` are valid for the call and `(*ctx).core` points to the core the block runs on
/// (the handler ABI); no other reference to the core is alive.
#[inline(always)]
pub(super) unsafe fn run(
    ctx: *mut UopCtx,
    up: *const Uop,
    pair: impl FnOnce(&mut Elem<'_>, &mut Elem<'_>, Args),
    one: impl Fn(&mut Elem<'_>, Args),
) -> u32 {
    let (ctx, u) = (&mut *ctx, &*up);
    let core = &mut *ctx.core;
    let args = unpack(u.a);
    let (x_on, y_on) = enabled(core, u.b, u.flags & UF_SIMD != 0);
    let due = core.irq.due;
    let (sx, sy) = {
        let (mut ex, mut ey) = elems(core);
        match (x_on, y_on) {
            (true, true) => pair(&mut ex, &mut ey, args),
            (true, false) => one(&mut ex, args),
            (false, true) => one(&mut ey, args),
            (false, false) => {}
        }
        (ex.sticky(), ey.sticky())
    };
    finish(ctx, up, core, due, sx, sy)
}

/// The end of a uop that collected the sticky bits `sx` (PEx) and `sy` (PEy): nothing to do when
/// none is set, otherwise the cold path.
///
/// # Safety
/// `up` is the pointer the handler was called with: it points into the uop slice of its block
/// (the handler ABI, `types::Handler`), so the uop after it is readable through it unless `up`
/// is the last uop. It must not be re-derived from a `&Uop`, whose provenance covers one element.
#[inline(always)]
pub(super) unsafe fn finish(
    ctx: &mut UopCtx,
    up: *const Uop,
    core: &mut Core,
    due: u64,
    sx: u32,
    sy: u32,
) -> u32 {
    if sx | sy == 0 {
        return Exit::Ok as u32;
    }
    raised(ctx, up, core, due, sx, sy)
}

/// Raises the arithmetic-exception bits (PEx first) and hands over to the driver when that
/// moved `irq.due`.
#[cold]
#[inline(never)]
unsafe fn raised(
    ctx: &mut UopCtx,
    up: *const Uop,
    core: &mut Core,
    due: u64,
    sx: u32,
    sy: u32,
) -> u32 {
    core.aot_pe_end(false, sx);
    core.aot_pe_end(true, sy);
    poked(ctx, up, core, due)
}

/// The exit of a uop that has run: `Ok` while `irq.due` is where the uop found it, else `Jump` to
/// the pc of the next uop, so the driver re-dispatches there, finds no room (`due` is 0) and lets
/// `Core::step` run the prologue (design 4.5 rule 4, P2: a `Jump` carries the steps of its uop).
/// The next pc is read through `up`, the slice pointer of the handler ABI (a pointer made from a
/// `&Uop` would cover one element only, so reading the next one through it would be unsound).
///
/// A uop that is the last of its block, or that finishes a step itself (`UF_FIN`: a delay slot or
/// a loop end), returns `Ok`: the block ends anyway or the driver still has to run `finish_step`,
/// and the next entry of the block finds no room. The lowering never emits `UF_FIN` on a uop that
/// is not the last of its block, because it declines a float instruction in the first delay slot
/// (`float::FloatFamily`). VERIFIED against the driver: its loop adds the uop's steps on `Jump`
/// and re-dispatches at `ctx.next_pc` (`dispatch.rs`, `block.rs`).
///
/// # Safety
/// `up` is valid and points into the uop slice of its block, as in [`finish`]; the next uop is read
/// only when `up` is neither `UF_LAST` (the last uop of every block, `BlockBuilder::finish`) nor
/// `UF_FIN`.
#[inline(always)]
pub(super) unsafe fn poked(ctx: &mut UopCtx, up: *const Uop, core: &Core, due: u64) -> u32 {
    if core.irq.due == due || (*up).flags & (UF_LAST | UF_FIN) != 0 {
        return Exit::Ok as u32;
    }
    // SAFETY: not the last uop of its block, so the next uop of the slice is the next uop of this
    // block; the pointer comes from the slice (see above).
    ctx.next_pc = unsafe { (*up.add(1)).pc };
    Exit::Jump as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::decode_cache::ComputeOp;
    use crate::core::support::core_with;

    #[test]
    fn pack_and_unpack_are_inverse() {
        for (n, x, y, p) in [(0, 0, 0, 0), (15, 7, 3, 0xA5), (1, 2, 15, 0xF_FFFF)] {
            assert_eq!(unpack(pack(n, x, y, p)), Args { n, x, y, p });
        }
        // out-of-range fields are masked, never spilled into a neighbour
        assert_eq!(unpack(pack(0x1F, 0x2F, 0x3F, 0x10_0001)).p, 1);
        assert_eq!(unpack(pack(0x1F, 0x2F, 0x3F, 0)).y, 15);
    }

    #[test]
    fn fgen_word_decodes_back() {
        for op in [0xC4u8, 0xC5] {
            for (n, x, y) in [(0, 0, 0), (15, 1, 14), (3, 9, 5)] {
                let alu = ComputeOp::FloatAlu { op, n, x, y };
                let w = fgen_word(alu).unwrap();
                assert_eq!(ComputeOp::decode(u64::from(w)), alu);
            }
        }
        // a float multiplier + float ALU multifunction word: bit 22, unit bits 21..19 = 0b011
        let c = 1u64 << 22 | 0b011 << 19 | 0b101 << 16 | 0xABCD;
        let multi = ComputeOp::Multifunction { c };
        assert_eq!(
            ComputeOp::decode(u64::from(fgen_word(multi).unwrap())),
            multi
        );
        // refused: ops with a dedicated kind, fixed-point multifunctions, everything else
        assert_eq!(fgen_word(ComputeOp::FloatMul { n: 1, x: 2, y: 3 }), None);
        let add = ComputeOp::FloatAlu {
            op: 0x81,
            n: 1,
            x: 2,
            y: 3,
        };
        assert_eq!(fgen_word(add), None);
        assert_eq!(fgen_word(ComputeOp::Multifunction { c: 1 << 22 }), None);
        assert_eq!(fgen_word(ComputeOp::Nop), None);
    }

    #[test]
    fn the_condition_of_each_element_comes_from_its_own_flags_and_needs_simd_for_pey() {
        let mut c = core_with(&[0x0001]);
        // condition 0 is EQ (AZ), 16 is NE
        c.flags.az = true;
        c.flags_y.az = false;
        assert_eq!(enabled(&c, 31, false), (true, false), "TRUE: PEx only");
        assert_eq!(enabled(&c, 31, true), (true, true), "TRUE with SIMD: both");
        assert_eq!(enabled(&c, 0, true), (true, false), "EQ: PEx only");
        assert_eq!(enabled(&c, 16, true), (false, true), "NE: PEy only");
        assert_eq!(enabled(&c, 16, false), (false, false), "no SIMD, no PEy");
    }

    #[test]
    fn a_poked_controller_hands_over_unless_the_uop_ends_the_step_or_the_block() {
        let mut core = core_with(&[0x0001]);
        let arena = [
            Uop::default(),
            Uop {
                pc: 0x12_0007,
                ..Uop::default()
            },
        ];
        let mut ctx = UopCtx::new(&mut core as *mut Core, 0, 8);
        // nothing moved `due`: carry on
        let first = arena.as_ptr();
        // SAFETY: `first` points into `arena`, which holds the uop and its successor.
        let r = unsafe { poked(&mut ctx, first, &core, core.irq.due) };
        assert_eq!(r, Exit::Ok as u32);
        // `due` moved: the next uop's pc is the re-dispatch point
        let r = unsafe { poked(&mut ctx, first, &core, core.irq.due ^ 1) };
        assert_eq!((r, ctx.next_pc), (Exit::Jump as u32, 0x12_0007));
        // the last uop and a uop that runs `finish_step` return Ok whatever `due` did
        for flags in [UF_LAST, UF_FIN] {
            let u = Uop { flags, ..arena[0] };
            // SAFETY: a last uop and a finishing uop do not read their successor.
            let r = unsafe { poked(&mut ctx, &raw const u, &core, core.irq.due ^ 1) };
            assert_eq!(r, Exit::Ok as u32);
        }
    }
}
