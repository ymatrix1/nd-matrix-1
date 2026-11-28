//! The handler kinds of the fixed-point range `0x200..0x300` and the operand layout of their
//! uops.
//!
//! OWNER: W2-FIXED. Constants are added here, in this range only (`types.rs` reserves it).
//!
//! | Kind | Instruction | `a` | `b` |
//! |---|---|---|---|
//! | `XALU_BASE + op` | fixed ALU opcode `op` of `Core::fixed_alu` (one kind per opcode, so the hot add, subtract and logic ops are specialised) | the 23-bit compute word (`op:8 n:4 x:4 y:4` at bits 19..0) | `IF` condition code (low 5 bits, 31 = TRUE) |
//! | [`UopKind::XMUL`] | multiplier `Rn = [MRx +/-] Rx * Ry`, `MRx = ...` (opcode classes 1..3 of `Core::fixed_mult`) | compute word with unit 1 | condition |
//! | [`UopKind::XMULS`] | `MRx = 0`, `SAT MRx`, `RND MRx` (class 0) | compute word with unit 1 | condition |
//! | [`UopKind::XMRT`] | `Rn = MRxF/B` and `MRxF/B = Rn` (`Core::mr_transfer`) | the multifunction word | condition |
//! | [`UopKind::XSHIFT`] | register-form shifter op (`Core::shifter`, no bit FIFO) | compute word with unit 2 | condition |
//! | [`UopKind::XSHIFTI`] | shift-immediate (`Core::exec_shift_imm`, no bit FIFO) | `op:8 n:4 x:4` | condition (bits 4..0) and the 12-bit immediate (bits 16..5) |
//! | [`UopKind::XDUAL`] | fixed dual add/subtract `Ra = Rx + Ry, Rs = Rx - Ry` | the compute word | condition |
//! | [`UopKind::XMF2`] | fixed multiplier plus ALU multifunction | the multifunction word | condition |
//! | [`UopKind::XMF3`] | fixed multiplier, add and subtract (three operations) | the multifunction word | condition |
//!
//! Every uop accounts for exactly one step. The mode of PEy is the flag `UF_SIMD` of the uop.
use crate::core::uop::types::UopKind;

/// First kind of the fixed ALU: the kind of opcode `op` is `XALU_BASE + op`. VERIFIED: the
/// opcodes are those of the `match` in `Core::fixed_alu` (`fixed_alu.rs`), checked by
/// `the_alu_opcode_list_is_the_interpreters` in `lower/fixed/valid.rs`.
pub const XALU_BASE: u16 = 0x200;
/// Kind numbers of the other handlers (ASSUMED layout inside the reserved range: the range is a
/// partition of the handler table, not a hardware fact).
pub const XMUL_K: u16 = 0x270;
pub const XMULS_K: u16 = 0x271;
pub const XMRT_K: u16 = 0x272;
pub const XSHIFT_K: u16 = 0x280;
pub const XSHIFTI_K: u16 = 0x281;
pub const XDUAL_K: u16 = 0x290;
pub const XMF2_K: u16 = 0x291;
pub const XMF3_K: u16 = 0x292;

impl UopKind {
    /// Multiplier with a product (`Rn = ...`, `MRx = ...`, `MRx = MRx +/- ...`).
    pub const XMUL: UopKind = UopKind(XMUL_K);
    /// `MRx = 0`, `SAT MRx`, `RND MRx`.
    pub const XMULS: UopKind = UopKind(XMULS_K);
    /// Multiplier result register transfers.
    pub const XMRT: UopKind = UopKind(XMRT_K);
    /// Register-form shifter operation.
    pub const XSHIFT: UopKind = UopKind(XSHIFT_K);
    /// Shift-immediate operation.
    pub const XSHIFTI: UopKind = UopKind(XSHIFTI_K);
    /// Fixed dual add/subtract.
    pub const XDUAL: UopKind = UopKind(XDUAL_K);
    /// Multiplier and ALU in one instruction.
    pub const XMF2: UopKind = UopKind(XMF2_K);
    /// Multiplier, add and subtract in one instruction.
    pub const XMF3: UopKind = UopKind(XMF3_K);

    /// The kind of fixed ALU opcode `op` (no check that the opcode is modelled).
    pub const fn xalu(op: u8) -> UopKind {
        UopKind(XALU_BASE + op as u16)
    }
}

/// The fixed ALU opcodes of `Core::fixed_alu`, in ascending order. VERIFIED: `fixed_alu.rs`
/// (the arms of the `match`), checked against the interpreter for all 256 opcodes by
/// `the_alu_opcode_list_is_the_interpreters` (`lower/fixed/valid.rs`).
pub const ALU_OPS: [u8; 21] = [
    0x01, 0x02, 0x05, 0x06, 0x09, 0x0A, 0x0B, 0x21, 0x22, 0x25, 0x26, 0x29, 0x2A, 0x30, 0x40, 0x41,
    0x42, 0x43, 0x61, 0x62, 0x63,
];

/// Every kind this package registers: the ALU kinds and the eight named ones, in table order.
pub fn all_kinds() -> Vec<u16> {
    let mut v: Vec<u16> = ALU_OPS
        .iter()
        .map(|&op| XALU_BASE + u16::from(op))
        .collect();
    v.extend([
        XMUL_K, XMULS_K, XMRT_K, XSHIFT_K, XSHIFTI_K, XDUAL_K, XMF2_K, XMF3_K,
    ]);
    v
}

/// The ALU opcode of kind `k`, when `k` is one of the ALU kinds.
pub const fn alu_op_of(k: u16) -> Option<u8> {
    if k >= XALU_BASE && k < XALU_BASE + 0x70 {
        Some((k - XALU_BASE) as u8)
    } else {
        None
    }
}

/// The compute-word fields `n`, `x`, `y` (register numbers, bits 11..8, 7..4, 3..0).
#[inline(always)]
pub const fn nxy(a: u32) -> (u8, u8, u8) {
    (((a >> 8) & 15) as u8, ((a >> 4) & 15) as u8, (a & 15) as u8)
}

/// The 8-bit operation code of a compute word (bits 19..12).
#[inline(always)]
pub const fn op8(a: u32) -> u8 {
    ((a >> 12) & 0xFF) as u8
}

/// The compute word `op:8 n:4 x:4 y:4` (unit bits 22..20 not included).
pub const fn word(op: u8, n: u8, x: u8, y: u8) -> u32 {
    (op as u32) << 12 | (n as u32 & 15) << 8 | (x as u32 & 15) << 4 | (y as u32 & 15)
}

/// The 12-bit immediate of a shift-immediate instruction word `w`: bits 30..27 are its high
/// nibble, bits 15..8 its low byte (`shift_imm_params`).
#[inline(always)]
pub const fn imm_data(w: u64) -> u32 {
    ((((w >> 27) & 15) << 8) | ((w >> 8) & 0xFF)) as u32
}

/// Operand `b` of an [`UopKind::XSHIFTI`] uop: the condition code in bits 4..0 and the 12-bit
/// immediate in bits 16..5.
#[inline(always)]
pub const fn imm_b(cond: u32, data: u32) -> u32 {
    (cond & 31) | (data & 0xFFF) << 5
}

/// The immediate of operand `b` of an [`UopKind::XSHIFTI`] uop.
#[inline(always)]
pub const fn imm_of(b: u32) -> u32 {
    (b >> 5) & 0xFFF
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::KindFamily;

    #[test]
    fn the_immediate_travels_through_the_uop_operand() {
        // r1 = ashift r1 by -1: 023e 7801 ff11
        let w = 0x023e_7801_ff11u64;
        assert_eq!(imm_data(w), 0xFFF);
        let b = imm_b(31, imm_data(w));
        assert_eq!((b & 31, imm_of(b)), (31, 0xFFF));
        assert_eq!(
            imm_b(0x3F, 0x1FFF),
            31 | 0xFFF << 5,
            "fields are masked, not merged"
        );
    }

    #[test]
    fn every_kind_is_in_the_fixed_range_and_distinct() {
        let kinds = all_kinds();
        let mut sorted = kinds.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), kinds.len(), "no kind twice");
        assert_eq!(kinds.len(), ALU_OPS.len() + 8);
        for k in kinds {
            assert_eq!(UopKind(k).family(), Some(KindFamily::Fixed), "{k:#x}");
        }
        assert_eq!(UopKind::xalu(0x63).0, 0x263);
        assert_eq!(UopKind::XMUL.0, XMUL_K);
    }

    #[test]
    fn the_named_kinds_do_not_collide_with_the_alu_block() {
        let last_alu = XALU_BASE + u16::from(*ALU_OPS.last().unwrap());
        assert!(XMUL_K > last_alu);
        assert_eq!(alu_op_of(last_alu), Some(0x63));
        assert_eq!(alu_op_of(XMUL_K), None);
        assert_eq!(alu_op_of(XALU_BASE - 1), None);
    }

    #[test]
    fn the_compute_word_fields_round_trip() {
        let a = word(0x78, 3, 1, 2) | 1 << 20;
        assert_eq!((op8(a), nxy(a)), (0x78, (3, 1, 2)));
        assert_eq!(word(0xFF, 0x1F, 0x2F, 0x3F), 0xFFFFF);
        assert!(ALU_OPS.windows(2).all(|w| w[0] < w[1]), "ascending");
    }
}
