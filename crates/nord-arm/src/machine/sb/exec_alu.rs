//! OWNER: W1-XALU. Executor arms of the data-processing uops, tier 2 (safe Rust).
//!
//! Spec (design note 3.1 and 4.3, WP W1-XALU): [`exec_dp`] runs one uop of kind `0..=107` on the
//! register file `r` and the flag nibble `f` (`n << 3 | z << 2 | c << 1 | v`, I4); `cpu` is not
//! touched, the executor loop unpacks `f` at every exit. Semantics are those of `Cpu::dp_exec`:
//!
//! * generic ids `0..=95` (`k::dp(form, opcode, s)`) run through one arm per `(OP, S, FORM)`
//!   const-generic combination; the shifter is `alu::shift` (form 1: `sh = type << 5 | amount`,
//!   LSR/ASR #0 = 32 and ROR #0 = RRX resolved by `imm = true`; form 2: `sh = type << 5`, amount
//!   from the low byte of `r[rs]`; form 0: `imm` is the rotated immediate, `sh` bit 0 set when the
//!   rotation was 0 so the shifter carry is C, else bit 31 of `imm`);
//! * arithmetic goes through `alu::add_c` (`sub`/`cmp` = `a + !b + 1`, `rsb` = `b + !a + 1`,
//!   `adc`/`sbc`/`rsc` use the pre-instruction C), logic ops take the shifter carry and keep V,
//!   test ops (`tst teq cmp cmn`) write no register, flags are written only when S;
//! * the specialised ids `96..=107` are the generic id they replace without the shifter: `MOV_REG`
//!   reads `rm`, `MOV_IMM`/`MVN_IMM` read `imm`, `ADD_IMM`/`SUB_IMM`/`ORR_IMM` read `rn` and `imm`,
//!   `ADD_REG`/`SUB_REG` read `rn` and `rm` (lsl #0), the compares and tests (always S) read `rn`
//!   and `imm` or `rm`; `TST_IMM` follows the form-0 `sh` bit 0 convention, `TST_REG` leaves C;
//! * `r15` is never read or written (I2): every register field is `0..=14` (`debug_assert!`).
//!
//! `exec_mul` (child module `mul`) is the MUL/MLA arm. No kind id exists for it yet (the design
//! keeps multiplies `GENERIC`, ids `108..=111` are reserved), so `exec_dp` does not dispatch to
//! it; it is complete and tested so that allocating the ids later is a one-line registration.
//!
//! Registered statically in `exec::run_sb` (family `k::Family::Dp`). The signature is frozen.
//!
//! Provenance: VERIFIED equal to `Cpu::dp_exec` / `Cpu::arm_mul` (the interpreter, the exactness
//! oracle of the design) by `diff::*`, `kill::*` and `mul::tests::*`: 2,000 random states per
//! `(op, S, form)`, every opcode on 13 x 13 edge operand pairs x 16 flag nibbles, the shifter on
//! every type x amount (and 17 register amounts) x 13 values x 16 nibbles, the 12 specialised ids
//! against the interpreter and their generic id; zero differences. `spec.rs` holds expectations
//! written from the ARM architecture rules, independent of the interpreter. That the interpreter
//! itself matches the AM1705 core stays ASSUMED, as in `cpu/`.
//!
//! Mutation sites (layer L5, `cfg(test)` only): ids `300..=349` are this file's (W1-XMEM takes
//! `350..=399` of the block shared by the two packages); `MUTANTS` lists them for W3-VERIF, and
//! `kill.rs` proves each one is killed by this file's differential suite.

use super::uop::{k, Uop};
use crate::cpu::alu::{add_c, shift};

mod mul;
pub use mul::exec_mul;

#[cfg(test)]
mod diff;
#[cfg(test)]
mod kill;
#[cfg(test)]
mod spec;

type Regs = [u32; 16];

/// L5 mutation site: true only inside `mutants::with(id, ..)` of a test build.
#[cfg(test)]
#[inline(always)]
fn mutant(id: u32) -> bool {
    super::mutants::is_active(id)
}

#[cfg(not(test))]
#[inline(always)]
const fn mutant(_id: u32) -> bool {
    false
}

/// `(id, name)` of the mutants planted in `exec_alu.rs` and `exec_alu/mul.rs`.
#[cfg(test)]
pub(crate) const MUTANTS: &[(u32, &str)] = &[
    (300, "adc_ignores_carry_in"),
    (301, "sbc_inverts_carry_in"),
    (302, "rsb_operands_not_swapped"),
    (303, "logic_ops_clear_v"),
    (304, "immediate_rot0_carry_from_bit31"),
    (305, "register_shift_amount_5_bits"),
    (306, "lsr_asr_imm0_is_no_shift"),
    (307, "test_ops_write_back"),
    (308, "cmp_special_carry_inverted"),
    (309, "flags_written_without_s"),
    (310, "rrx_carry_in_is_zero"),
    (311, "cmp_imm_adds"),
    (312, "mvn_imm_not_inverted"),
    (313, "tst_reg_clears_c"),
    (314, "mla_drops_accumulator"),
    (315, "muls_flags_not_written"),
];

/// Register `i` (`0..=14`).
#[inline(always)]
fn reg(r: &Regs, i: u8) -> u32 {
    debug_assert!(i < 15, "I2: a data-processing uop never names r15 ({i})");
    r[(i & 15) as usize]
}

/// Write register `i` (`0..=14`).
#[inline(always)]
fn put(r: &mut Regs, i: u8, v: u32) {
    debug_assert!(i < 15, "I2: a data-processing uop never names r15 ({i})");
    r[(i & 15) as usize] = v;
}

/// N and Z of `res` in nibble position.
#[inline(always)]
const fn nz(res: u32) -> u8 {
    (((res >> 31) as u8) << 3) | (((res == 0) as u8) << 2)
}

/// Whole nibble of an arithmetic result.
#[inline(always)]
const fn nzcv(res: u32, carry: bool, ovf: bool) -> u8 {
    nz(res) | ((carry as u8) << 1) | ovf as u8
}

/// Nibble of `a - b` (a compare): carry is "no borrow".
#[inline(always)]
fn sub_flags(a: u32, b: u32) -> u8 {
    let (res, carry, ovf) = add_c(a, !b, true);
    nzcv(res, carry != mutant(308), ovf)
}

/// Second operand and shifter carry of operand form `FORM`; `c` is the pre-instruction C.
#[inline(always)]
fn operand<const FORM: u32>(r: &Regs, u: &Uop, c: bool) -> (u32, bool) {
    let ty = (u.sh >> 5 & 3) as u32;
    match FORM {
        0 => {
            let rot0 = u.sh & 1 != 0 && !mutant(304);
            (u.imm, if rot0 { c } else { u.imm >> 31 != 0 })
        }
        1 => {
            let amount = (u.sh & 31) as u32;
            if mutant(306) && amount == 0 && ty != 0 && ty != 3 {
                return (reg(r, u.rm), c);
            }
            shift(ty, amount, reg(r, u.rm), c && !mutant(310), true)
        }
        _ => {
            let amount = reg(r, u.rs) & if mutant(305) { 31 } else { u32::MAX };
            shift(ty, amount, reg(r, u.rm), c, false)
        }
    }
}

/// One generic data-processing arm: opcode `OP`, S bit `S`, operand form `FORM`.
#[inline(always)]
fn dp<const OP: u32, const S: bool, const FORM: u32>(r: &mut Regs, f: &mut u8, u: &Uop) {
    let fl = *f;
    let (c, v) = (fl & 2 != 0, fl & 1 != 0 && !mutant(303));
    let (b, sc) = operand::<FORM>(r, u, c);
    let a = if matches!(OP, 13 | 15) {
        0
    } else {
        reg(r, u.rn)
    };
    // (result, carry, overflow): logic ops pass the shifter carry and the old V through
    let (res, carry, ovf) = match OP {
        0 | 8 => (a & b, sc, v),
        1 | 9 => (a ^ b, sc, v),
        2 | 10 => add_c(a, !b, true),
        3 if mutant(302) => add_c(a, !b, true),
        3 => add_c(b, !a, true),
        4 | 11 => add_c(a, b, false),
        5 => add_c(a, b, c && !mutant(300)),
        6 => add_c(a, !b, c != mutant(301)),
        7 => add_c(b, !a, c),
        12 => (a | b, sc, v),
        13 => (b, sc, v),
        14 => (a & !b, sc, v),
        _ => (!b, sc, v),
    };
    if S || mutant(309) {
        *f = nzcv(res, carry, ovf);
    }
    if !matches!(OP, 8..=11) || mutant(307) {
        put(r, u.rd, res);
    }
}

/// Expand the 96 generic ids into `dp::<OP, S, FORM>` calls, keyed by `(form, opcode, s)`.
macro_rules! generic_arms {
    ($r:ident, $f:ident, $u:ident; $($op:literal)*) => {
        match (($u.kind >> 5) as u32, ($u.kind >> 1 & 15) as u32, $u.kind & 1 != 0) {
            $((0, $op, false) => dp::<$op, false, 0>($r, $f, $u),
              (0, $op, true) => dp::<$op, true, 0>($r, $f, $u),)*
            $((1, $op, false) => dp::<$op, false, 1>($r, $f, $u),
              (1, $op, true) => dp::<$op, true, 1>($r, $f, $u),)*
            $((2, $op, false) => dp::<$op, false, 2>($r, $f, $u),
              (2, $op, true) => dp::<$op, true, 2>($r, $f, $u),)*
            _ => unreachable!("generic data-processing id {} out of range", $u.kind),
        }
    };
}

/// The specialised ids `MOV_IMM..=ORR_IMM`: no shifter, no generic operand decoding.
#[inline(always)]
fn special(r: &mut Regs, f: &mut u8, u: &Uop) {
    let (rn, rm) = (u.rn, u.rm);
    match u.kind {
        k::MOV_IMM => put(r, u.rd, u.imm),
        k::MOV_REG => put(r, u.rd, reg(r, rm)),
        k::MVN_IMM => put(r, u.rd, if mutant(312) { u.imm } else { !u.imm }),
        k::ADD_IMM => put(r, u.rd, reg(r, rn).wrapping_add(u.imm)),
        k::SUB_IMM => put(r, u.rd, reg(r, rn).wrapping_sub(u.imm)),
        k::ADD_REG => put(r, u.rd, reg(r, rn).wrapping_add(reg(r, rm))),
        k::SUB_REG => put(r, u.rd, reg(r, rn).wrapping_sub(reg(r, rm))),
        k::ORR_IMM => put(r, u.rd, reg(r, rn) | u.imm),
        k::CMP_IMM if mutant(311) => {
            let (res, carry, ovf) = add_c(reg(r, rn), u.imm, false);
            *f = nzcv(res, carry, ovf);
        }
        k::CMP_IMM => *f = sub_flags(reg(r, rn), u.imm),
        k::CMP_REG => *f = sub_flags(reg(r, rn), reg(r, rm)),
        k::TST_IMM => {
            let carry = if u.sh & 1 != 0 {
                *f & 2 != 0
            } else {
                u.imm >> 31 != 0
            };
            *f = nz(reg(r, rn) & u.imm) | ((carry as u8) << 1) | (*f & 1);
        }
        k::TST_REG => {
            // lsl #0: the shifter carry is the old C
            let keep = if mutant(313) { 1 } else { 3 };
            *f = nz(reg(r, rn) & reg(r, rm)) | (*f & keep);
        }
        kind => unreachable!("kind {kind} is not a specialised data-processing id"),
    }
}

/// Execute one data-processing uop (kind `0..=107`) on the registers `r` and the flag nibble
/// `f`. Any other kind is a bug of the caller (`k::family`), not a guest condition.
#[inline]
pub fn exec_dp(r: &mut [u32; 16], f: &mut u8, u: &Uop) {
    match u.kind {
        0..=95 => generic_arms!(r, f, u; 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15),
        96..=107 => special(r, f, u),
        kind => unreachable!(
            "uop kind {kind} is not a data-processing uop (family {:?})",
            k::family(kind)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_data_processing_id_has_an_arm_and_nothing_else_does() {
        let mut ran = 0;
        for kind in 0..=255u8 {
            if k::family(kind) == k::Family::Dp {
                let (mut r, mut f) = ([0u32; 16], 0u8);
                exec_dp(&mut r, &mut f, &Uop::new(kind, 0x1000));
                ran += 1;
            }
        }
        assert_eq!(ran, 108, "96 generic ids + 12 specialised ids");
        for kind in [108u8, 111, 112, 144, 160, 255] {
            let caught = std::panic::catch_unwind(|| {
                exec_dp(&mut [0; 16], &mut 0, &Uop::new(kind, 0x1000));
            });
            assert!(caught.is_err(), "kind {kind} is not a data-processing id");
        }
    }

    #[test]
    fn the_nibble_helpers_pack_n_z_c_v_in_the_cpu_layout() {
        assert_eq!(nz(0), 4);
        assert_eq!(nz(0x8000_0000), 8);
        assert_eq!(nz(1), 0);
        assert_eq!(nzcv(0x8000_0000, true, true), 8 | 2 | 1);
        // the layout of `uop::pack_flags` (`Cpu::nzcv`)
        let mut c = crate::cpu::Cpu::new();
        c.n = true;
        c.c = true;
        assert_eq!(crate::machine::sb::uop::pack_flags(&c), 8 | 2);
    }
}
