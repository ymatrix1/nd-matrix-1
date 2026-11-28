//! Lowering of fixed-point instructions to uops.
//!
//! OWNER: W2-FIXED. Fixed-point compute (ALU, multiplier, shifter, MR registers, the dual
//! add/subtract and the multifunction forms) of the conditional compute (`T2a`), the unconditional
//! compute (`T2b`, `T2c`) and the shift immediate (`T6n`). One uop per instruction, one step.
//!
//! Which words are taken is decided by [`valid`], each list tied to the interpreter by a test.
//! Everything else (the bit FIFO shifter forms, the float forms, unmodelled codes, the compute
//! field of a move or flow instruction, `T6a` shift-and-move) stays an `Interp` uop, which is exact
//! by construction. The handlers are in `exec/h_fixed/` (see its `kinds.rs` for the operand
//! layout).
//!
//! PEy: the uop carries `UF_SIMD` when the block was translated with PEy on; the handler then runs
//! the compute on both elements, each under its own condition, exactly as `Core::exec_cached` does.
pub mod valid;

use super::builder::BlockBuilder;
use super::{Family, LowerCtx};
use crate::core::decode_cache::{CachedInsn, ComputeOp, DecodedOp};
use crate::core::shifter::shift_imm_params;
use crate::core::uop::exec::h_fixed::kinds::{imm_b, imm_data, word};
use crate::core::uop::types::{Uop, UopKind, UF_SIMD};
use crate::visa::bits;
use valid::{alu_kind, mult_kind, multi_kind, shift_modelled};

/// The fixed-point family.
pub(in crate::core) struct FixedFamily;

impl Family for FixedFamily {
    fn lower(cx: &LowerCtx<'_>, c: &CachedInsn, out: &mut BlockBuilder) -> bool {
        let Some((kind, a, b)) = plan(c) else {
            return false;
        };
        let mut u = Uop::new(kind, 1, c.pc, a, b);
        if cx.simd {
            u.flags |= UF_SIMD;
        }
        out.push(u);
        true
    }
}

/// The kind and the operands `(a, b)` of the uop for `c`, `None` when the interpreter keeps it.
fn plan(c: &CachedInsn) -> Option<(UopKind, u32, u32)> {
    match c.op {
        DecodedOp::ComputeCond { cond, op } => compute(op).map(|(k, a)| (k, a, u32::from(cond))),
        DecodedOp::ComputeBoth { op } => compute(op).map(|(k, a)| (k, a, 31)),
        DecodedOp::ShiftImm => shift_imm(c),
        _ => None,
    }
}

/// The kind and the compute word of a decoded compute operation.
fn compute(op: ComputeOp) -> Option<(UopKind, u32)> {
    match op {
        ComputeOp::FixedAlu { op, n, x, y } => Some((alu_kind(op)?, word(op, n, x, y))),
        ComputeOp::FixedMult { op, n, x, y } => Some((mult_kind(op)?, 1 << 20 | word(op, n, x, y))),
        ComputeOp::Shifter { op, n, x, y } => {
            shift_modelled(op).then_some((UopKind::XSHIFT, 2 << 20 | word(op, n, x, y)))
        }
        // the fixed dual add/subtract has the opcode nibble 7 (the float one is 0xF)
        ComputeOp::DualAddSub { c } => (bits(c, 19, 16) == 7).then_some((UopKind::XDUAL, c as u32)),
        ComputeOp::Multifunction { c } => Some((multi_kind(c as u32)?, c as u32)),
        _ => None,
    }
}

/// `[IF cond] Rn = <shifter op> Rx BY <imm12>` (`T6n`) without the bit FIFO forms.
fn shift_imm(c: &CachedInsn) -> Option<(UopKind, u32, u32)> {
    let (op, n, x, _) = shift_imm_params(c.w)?;
    let op = u8::try_from(op).ok().filter(|&o| shift_modelled(o))?;
    Some((
        UopKind::XSHIFTI,
        u32::from(op) | (n as u32) << 8 | (x as u32) << 12,
        imm_b(u32::from(c.cond), imm_data(c.w)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::config::UopConfig;
    use crate::core::uop::lower::{decode::decode_at, lower_first};

    /// The uop `F` emits for the first instruction of `program`, `None` when it declines.
    fn lowered(program: &[u16], simd: bool) -> Option<Uop> {
        let mut core = crate::core::support::core_with(program);
        let pc = core.pc;
        let insn = decode_at(&mut core, pc);
        let cfg = UopConfig::default();
        let cx = LowerCtx {
            cfg: &cfg,
            stop_pc: None,
            simd,
            isa: false,
        };
        let mut b = BlockBuilder::new(pc, simd, false);
        let taken = FixedFamily::lower(&cx, &insn, &mut b);
        assert_eq!(
            taken,
            b.len() == 1,
            "taken exactly when one uop was emitted"
        );
        assert_eq!(b.steps(), u32::from(taken), "one step, or nothing");
        b.uops().first().copied()
    }

    #[test]
    fn an_add_becomes_one_alu_uop_with_its_word_and_condition() {
        // R3 = R1 + R2, IF TRUE: parcels 013e 0000 1312
        let u = lowered(&[0x013e, 0x0000, 0x1312], false).expect("taken");
        assert_eq!((u.h, u.steps, u.flags), (0x201, 1, 0));
        assert_eq!(u.a, 0x1312, "op 0x01, n 3, x 1, y 2");
        assert_eq!((u.b, u.pc), (31, crate::core::support::CODE));
        // the same instruction in a block translated with PEy on
        let u = lowered(&[0x013e, 0x0000, 0x1312], true).unwrap();
        assert_eq!(u.flags, UF_SIMD);
        // IF EQ: the condition code goes into `b`
        let u = lowered(&[0x0100, 0x0000, 0x1312], false).unwrap();
        assert_eq!(u.b, 0);
        // the 16-bit R1 = R1 - R2 (T2c) runs on both elements without a condition
        let u = lowered(&[0xc112], true).unwrap();
        assert_eq!((u.h, u.a, u.b), (0x202, 0x2112, 31));
    }

    #[test]
    fn every_unit_gets_its_kind() {
        let kinds = |program: &[u16]| lowered(program, false).map(|u| u.h);
        // R3 = R1 * R2 (SSI, T2c op 7 is Rn = Rn * Rx), MRF = 0, R3 = MR0F, shifter LSHIFT
        assert_eq!(kinds(&[0xc712]), Some(UopKind::XMUL.0));
        assert_eq!(kinds(&t2a(0x14 << 12 | 1 << 20)), Some(UopKind::XMULS.0));
        assert_eq!(kinds(&t2a(1 << 22 | 3 << 8)), Some(UopKind::XMRT.0));
        assert_eq!(
            kinds(&t2a(2 << 20 | 0x04 << 12 | 3 << 8 | 1 << 4 | 2)),
            Some(UopKind::XSHIFT.0)
        );
        assert_eq!(
            kinds(&t2a(0x7 << 16 | 2 << 12 | 3 << 8 | 1)),
            Some(UopKind::XDUAL.0)
        );
        assert_eq!(
            kinds(&t2a(1 << 22 | 0b001000 << 16 | 9 << 8)),
            Some(UopKind::XMF2.0)
        );
        assert_eq!(
            kinds(&t2a(1 << 22 | 1 << 21 | 3 << 16 | 5 << 12 | 6 << 8)),
            Some(UopKind::XMF3.0)
        );
    }

    /// The parcels of `IF TRUE <compute c>` (T2a).
    fn t2a(c: u32) -> [u16; 3] {
        [0x013e, (c >> 16) as u16, c as u16]
    }

    #[test]
    fn the_shift_immediate_keeps_the_register_op_the_registers_and_the_immediate() {
        // r1 = ashift r1 by -1: 023e 7801 ff11 (ADI elfdump, `shifter.rs` tests)
        let u = lowered(&[0x023e, 0x7801, 0xff11], false).expect("taken");
        assert_eq!((u.h, u.steps), (UopKind::XSHIFTI.0, 1));
        assert_eq!(u.a, 0x04 | 1 << 8 | 1 << 12, "ASHIFT, n 1, x 1");
        assert_eq!(u.b, 31 | 0xFFF << 5, "IF TRUE, the 12-bit immediate");
        // r4 = btgl r4 by 0
        let u = lowered(&[0x023e, 0x0032, 0x0044], false).unwrap();
        assert_eq!(u.a, 0xC8 | 4 << 8 | 4 << 12);
    }

    #[test]
    fn what_the_family_does_not_own_stays_with_the_interpreter() {
        let none = |program: &[u16]| lowered(program, false).is_none();
        assert!(none(&[0x0001]), "NOP");
        assert!(none(&t2a(0x81 << 12 | 3 << 8 | 1 << 4 | 2)), "float add");
        assert!(none(&t2a(0xF1 << 12)), "float dual add/subtract");
        assert!(
            none(&t2a(1 << 22 | 1 << 21 | 1 << 20 | 3 << 16)),
            "float triple form"
        );
        assert!(
            none(&t2a(2 << 20 | 0x74 << 12 | 3 << 8)),
            "BITDEP is a bit FIFO form"
        );
        assert!(none(&t2a(1 << 20 | 0x20 << 12)), "no multiplier operation");
        assert!(none(&t2a(3 << 20 | 0x01 << 12)), "no compute unit 3");
        assert!(
            none(&t2a(0x03 << 12 | 3 << 8)),
            "ALU op 0x03 does not exist"
        );
        // r3 = bitext 0x5: a bit FIFO immediate, and FPACK has no immediate form
        assert!(none(&[0x023e, 0x0014, 0x0535]));
        // T6a shift and move: the compute is not alone
        assert!(none(&[0x8b7e, 0x3011, 0x4650]));
        assert_eq!(lower_first::<FixedFamily>(&[0x0001]), (false, 0));
    }
}
