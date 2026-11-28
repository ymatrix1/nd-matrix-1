//! OWNER: W2-NDP. Native lowering of the data-processing uops (feature `sb-native`).
//!
//! Spec (design note 9, WP W2-NDP): [`lower_dp`] emits the Cranelift code of one uop of kind
//! `0..=107` through the `NCtx` vocabulary (`reg`/`set_reg`, `flag`/`set_flag`): the 96 generic
//! ids (`k::dp(form, opcode, s)`, shifter forms 0 to 2) and the 12 specialised ids. It equals
//! the tier-2 `exec_alu::exec_dp` on every register and flag: the result and flags of the
//! sixteen opcodes, the carry from the shifter, V from add and subtract, the S forms, the
//! carry-in of `adc`/`sbc`/`rsc`, and the register-shift amount rules (amount 0, 32, 33, >= 256).
//!
//! | File | What |
//! |---|---|
//! | `shift` | the second operand and the shifter carry (form 0, 1, 2) |
//! | `arith` | `x + y + cin` with N Z C V, `p - q`, the arithmetic opcodes |
//! | `special` | the ids `96..=107` |
//! | `cond` | [`cond_holds`], the condition of a `PRED` as an SSA value |
//!
//! Decisions (all before anything is emitted, from the uop alone, so `NCtx::body_lowerable`'s
//! dry run agrees with the real lowering):
//!
//! * a kind that is not `0..=107` (the multiplies `108..=111` of W2-MUL, anything unknown) is
//!   [`Unsupported`]: the trace leaves natively before it, through `exit_generic` at its pc;
//! * a register field the uop uses that is `r15` is [`Unsupported`] (I2: `NCtx::reg` has no
//!   `r15`; the trace builder never emits it, this is the second line of defence);
//! * a non-S `tst`/`teq`/`cmp`/`cmn` is a no-op (as in the executor) and emits nothing.
//!
//! Flags: `set_flag` records which flags the trace defined, an exit stores only those, and a
//! logic op with S leaves V (and C, when the shifter carry is the old C) untouched, so the
//! interpreter's "unchanged" is "never written". A flag value that a later uop overwrites
//! before any exit or read is a dead SSA value that Cranelift removes (the rule of
//! `flagopt.rs`, for free: `a_flag_overwritten_before_any_use_costs_no_code`).
//!
//! The compare-then-guard fusion of the design needs the `GUARD` and fused uops, which belong
//! to `lower_flow` (W2-NMEM): its guard lowering reads the flags this file leaves in the
//! variables, and Cranelift folds `icmp` -> flag -> `brif` chains.
//!
//! Mutation sites (layer L5, ids `800..=849` of `mutants.rs`, `cfg(test)` only): [`MUTANTS`]
//! lists them for W3-VERIF, `lower_dp/kill.rs` proves each is killed by this file's own
//! differential suite.

use super::emit::{NCtx, Unsupported};
use crate::machine::sb::uop::Uop;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, Value};

mod arith;
mod cond;
mod shift;
mod special;

#[cfg(test)]
mod diff;
#[cfg(test)]
mod harness;
#[cfg(test)]
mod kill;
#[cfg(test)]
mod sweeps;

pub use cond::cond_holds;

/// L5 mutation site: true only inside `mutants::with(id, ..)` of a test build.
#[cfg(test)]
#[inline(always)]
fn mutant(id: u32) -> bool {
    crate::machine::sb::mutants::is_active(id)
}

#[cfg(not(test))]
#[inline(always)]
const fn mutant(_id: u32) -> bool {
    false
}

/// `(id, name)` of the mutants planted in `native/lower_dp.rs` and its children (block
/// `800..=849`).
#[cfg(test)]
pub(crate) const MUTANTS: &[(u32, &str)] = &[
    (800, "native adc ignores the carry in"),
    (801, "native sbc inverts the carry in"),
    (802, "native rsb does not swap its operands"),
    (803, "native logic ops with S clear V"),
    (
        804,
        "native immediate with rotation 0 takes carry from bit 31",
    ),
    (805, "native register shift amount is 5 bits"),
    (806, "native lsr/asr #0 is no shift"),
    (807, "native test ops write the register"),
    (808, "native cmp inverts the carry"),
    (809, "native flags are written without S"),
    (810, "native rrx shifts in zero"),
    (811, "native cmp_imm adds"),
    (812, "native mvn_imm is not inverted"),
    (813, "native tst_reg clears C"),
    (814, "native subtract overflow sees b, not !b"),
    (815, "native lsl by 32 has carry 0"),
    (816, "native asr by 32 or more has carry 0"),
    (817, "native ror by a multiple of 32 keeps C"),
    (818, "native lsr by 32 does not clear the value"),
    (819, "native condition HI is c or !z"),
    (820, "native conditions GE and LT are swapped"),
    (821, "native sub and cmp add"),
    (822, "native sub_imm adds"),
    (823, "native orr_imm ands"),
    (824, "native add_reg adds rn twice"),
    (825, "native register shift amount is 9 bits"),
    (826, "native Z flag is inverted"),
    (827, "native adc carry out ignores the carry in"),
];

/// `Err(Unsupported)` when any of the registers a uop uses is `r15` (I2).
fn used(regs: &[u8]) -> Result<(), Unsupported> {
    if regs.iter().all(|r| *r < 15) {
        Ok(())
    } else {
        Err(Unsupported)
    }
}

/// An `i8` comparison result as the `i32` 0 or 1 of a flag.
fn b2i(cx: &mut NCtx<'_>, v: Value) -> Value {
    cx.b.ins().uextend(types::I32, v)
}

/// Bit `n` of `v` as an `i32` 0 or 1.
fn bit(cx: &mut NCtx<'_>, v: Value, n: u32) -> Value {
    let s = cx.b.ins().ushr_imm(v, i64::from(n));
    cx.b.ins().band_imm(s, 1)
}

/// Bit `k` of `v` as an `i32` 0 or 1, `k` a value (taken modulo 32 by the shift).
fn bit_dyn(cx: &mut NCtx<'_>, v: Value, k: Value) -> Value {
    let s = cx.b.ins().ushr(v, k);
    cx.b.ins().band_imm(s, 1)
}

/// Write N (bit 31) and Z (zero) of `res`.
fn set_nz(cx: &mut NCtx<'_>, res: Value) {
    let n = cx.b.ins().ushr_imm(res, 31);
    cx.set_flag(3, n);
    let cc = if mutant(826) {
        IntCC::NotEqual
    } else {
        IntCC::Equal
    };
    let z = cx.b.ins().icmp_imm(cc, res, 0);
    let z = b2i(cx, z);
    cx.set_flag(2, z);
}

/// Emit the native code of `u` (data processing, kinds `0..=107`). `Err(Unsupported)` for any
/// other kind and for an `r15` field, with nothing emitted.
pub fn lower_dp(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
    match u.kind {
        0..=95 => generic(cx, u),
        96..=107 => special::special(cx, u),
        _ => Err(Unsupported),
    }
}

/// A generic id: `form * 32 + opcode * 2 + s`.
fn generic(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
    let (form, op, s) = (
        u32::from(u.kind >> 5),
        u32::from(u.kind >> 1 & 15),
        u.kind & 1 != 0,
    );
    let tests = matches!(op, 8..=11);
    let writes = !tests || mutant(807);
    let reads_rn = !matches!(op, 13 | 15);
    if writes {
        used(&[u.rd])?;
    }
    if reads_rn {
        used(&[u.rn])?;
    }
    if form >= 1 {
        used(&[u.rm])?;
    }
    if form == 2 {
        used(&[u.rs])?;
    }
    let flags = s || mutant(809);
    if tests && !flags {
        return Ok(()); // not a data-processing instruction without S; the executor does nothing
    }
    let logic = !matches!(op, 2..=7 | 10 | 11);
    let operand = shift::operand(cx, u, form, flags && logic);
    let b = operand.value;
    let a = if reads_rn { cx.reg(u.rn) } else { b };
    let res = if logic {
        let res = match op {
            0 | 8 => cx.b.ins().band(a, b),
            1 | 9 => cx.b.ins().bxor(a, b),
            12 => cx.b.ins().bor(a, b),
            13 => b,
            14 => cx.b.ins().band_not(a, b),
            _ => cx.b.ins().bnot(b),
        };
        if flags {
            set_nz(cx, res);
            if let Some(carry) = operand.carry {
                cx.set_flag(1, carry);
            }
            if mutant(803) {
                let zero = cx.const32(0);
                cx.set_flag(0, zero);
            }
        }
        res
    } else {
        arith::arith(cx, op, a, b, flags)
    };
    if writes {
        cx.set_reg(u.rd, res);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::cache::tiny_trace;
    use crate::machine::sb::native::emit::testing::build_and_verify;
    use crate::machine::sb::uop::k;

    /// Registers the uop of `kind` uses: `[rd, rn, rm, rs]` as read from the executor's rules
    /// (`exec_alu`): the generic opcode decides `rd` and `rn`, the form `rm` and `rs`.
    fn used_fields(kind: u8) -> [bool; 4] {
        if kind >= 96 {
            return match kind {
                k::MOV_IMM | k::MVN_IMM => [true, false, false, false],
                k::MOV_REG => [true, false, true, false],
                k::ADD_IMM | k::SUB_IMM | k::ORR_IMM => [true, true, false, false],
                k::ADD_REG | k::SUB_REG => [true, true, true, false],
                k::CMP_IMM | k::TST_IMM => [false, true, false, false],
                _ => [false, true, true, false],
            };
        }
        let (form, op) = (kind >> 5, kind >> 1 & 15);
        [
            !matches!(op, 8..=11),
            !matches!(op, 13 | 15),
            form >= 1,
            form == 2,
        ]
    }

    /// `(registers written, flags written)` of the uop of `kind` with `sh` = 0 (form 0: the
    /// rotation is not 0, so the immediate carry is a constant).
    fn effects(kind: u8) -> (u16, u8) {
        let rd = 1 << 3;
        match kind {
            96..=107 => match kind {
                k::CMP_IMM | k::CMP_REG => (0, 0b1111),
                k::TST_IMM => (0, 0b1110),
                k::TST_REG => (0, 0b1100),
                _ => (rd, 0),
            },
            _ => {
                let (form, op, s) = (kind >> 5, kind >> 1 & 15, kind & 1 != 0);
                let regs = if matches!(op, 8..=11) { 0 } else { rd };
                let flags = match (s, op) {
                    (false, _) => 0,
                    (true, 2..=7 | 10 | 11) => 0b1111,
                    // logic: N Z, and C unless the shifter keeps it (register form, lsl #0)
                    (true, _) => 0b1100 | if form == 1 { 0 } else { 0b10 },
                };
                (regs, flags)
            }
        }
    }

    fn uop(kind: u8) -> Uop {
        let mut u = Uop::new(kind, 0x1000);
        (u.rd, u.rn, u.rm, u.rs, u.imm) = (3, 1, 2, 4, 0x8000_00FF);
        u
    }

    #[test]
    fn every_data_processing_kind_lowers_with_the_effects_of_its_row_and_all_else_declines() {
        let sb = tiny_trace(0x1000, &[(0x1000, 1)]);
        let mut lowered = 0;
        for kind in 0..=255u8 {
            build_and_verify(&sb, |cx| {
                let before = cx.b.func.dfg.num_insts();
                let r = lower_dp(cx, &uop(kind));
                if kind <= 107 {
                    assert_eq!(r, Ok(()), "kind {kind}");
                    lowered += 1;
                    let (regs, flags) = effects(kind);
                    // form 1 with sh = 0 is lsl #0: the shifter carry is C
                    assert_eq!((cx.dirty, cx.flags_def), (regs, flags), "kind {kind}");
                } else {
                    assert_eq!(r, Err(Unsupported), "kind {kind}");
                    assert_eq!(cx.b.func.dfg.num_insts(), before, "kind {kind}: emitted");
                    assert_eq!((cx.dirty, cx.flags_def), (0, 0), "kind {kind}");
                }
                cx.exit_generic(0x1000);
            });
        }
        assert_eq!(lowered, 108, "96 generic ids and 12 specialised ids");
    }

    #[test]
    fn the_multiply_ids_and_other_unknown_kinds_are_unsupported_never_guessed() {
        let sb = tiny_trace(0x1000, &[(0x1000, 1)]);
        build_and_verify(&sb, |cx| {
            let before = cx.b.func.dfg.num_insts();
            for kind in [
                108, 109, 110, 111, 112, 129, 144, 151, 160, 164, 165, 200, 255,
            ] {
                assert_eq!(lower_dp(cx, &uop(kind)), Err(Unsupported), "kind {kind}");
            }
            assert_eq!(cx.b.func.dfg.num_insts(), before);
            cx.exit_generic(0x1000);
        });
    }

    #[test]
    fn an_r15_field_is_unsupported_exactly_when_the_uop_uses_it() {
        let sb = tiny_trace(0x1000, &[(0x1000, 1)]);
        let mut declined = 0;
        for kind in 0..=107u8 {
            let used = used_fields(kind);
            for (i, used) in used.iter().enumerate() {
                build_and_verify(&sb, |cx| {
                    let mut u = uop(kind);
                    match i {
                        0 => u.rd = 15,
                        1 => u.rn = 15,
                        2 => u.rm = 15,
                        _ => u.rs = 15,
                    }
                    let before = cx.b.func.dfg.num_insts();
                    let r = lower_dp(cx, &u);
                    assert_eq!(r.is_err(), *used, "kind {kind} field {i}");
                    if r.is_err() {
                        declined += 1;
                        assert_eq!(cx.b.func.dfg.num_insts(), before, "nothing emitted");
                    }
                    cx.exit_generic(0x1000);
                });
            }
        }
        assert!(declined > 200, "{declined} declined cases");
    }

    #[test]
    fn a_non_s_test_opcode_is_a_no_op_that_emits_nothing() {
        let sb = tiny_trace(0x1000, &[(0x1000, 1)]);
        build_and_verify(&sb, |cx| {
            let before = cx.b.func.dfg.num_insts();
            for op in 8..=11 {
                for form in 0..3 {
                    assert_eq!(lower_dp(cx, &uop(k::dp(form, op, false))), Ok(()));
                }
            }
            assert_eq!(cx.b.func.dfg.num_insts(), before);
            assert_eq!((cx.dirty, cx.flags_def), (0, 0));
            cx.exit_generic(0x1000);
        });
    }

    #[test]
    fn the_mutant_ids_are_unique_and_inside_the_native_dp_block() {
        let mut ids: Vec<u32> = MUTANTS.iter().map(|m| m.0).collect();
        assert!(ids.iter().all(|id| (800..=849).contains(id)));
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MUTANTS.len());
    }
}
