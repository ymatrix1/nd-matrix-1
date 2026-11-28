//! Operand layout of the flow uops: written by `lower/flow`, read by the handlers.
//!
//! OWNER: W2-FLOW. The layout is internal to the engine (no hardware meaning), so every field is
//! VERIFIED by the round-trip tests below; the instruction fields it carries come from the decode
//! (`FlowOp`, `decode_cache.rs`) and are checked against `Core::step` by `diff.rs`.
//!
//! | Kind | `a` | `b` |
//! |---|---|---|
//! | `JUMP` | target pc | branch word |
//! | `JUMP_I` | `reg_i \| reg_m << 4` (indexes into `I` and `M`, 0..=15) | branch word |
//! | `RET` | 0 | branch word |
//! | `IDLE` | 0 | branch word |
//! | `DO_COUNT` | pc of the last body instruction | count immediate, or `FROM_REG \| register code` |
//! | `DO_UNTIL` | low 32 bits of the 48-bit instruction word | the upper 16 bits |
//! | `SYS_LOAD` | universal register code | value |
//!
//! Branch word: bits 0..=4 condition code (31 = TRUE), bit 5 delayed, bit 6 call, bit 7 return
//! from interrupt, bits 8..=9 instruction length in parcels (1..=3), so the fall-through pc is
//! `u.pc + len` without a decode lookup.

/// Mask of the condition code in a branch word. VERIFIED: the condition field of the decode is 5
/// bits wide (0..=31), `a_branch_word_round_trips_every_field`.
pub(in crate::core) const COND_MASK: u32 = 0x1F;
/// Delayed branch (two delay slots follow in the same block). VERIFIED: internal layout, no
/// overlap with the other fields (`the_fields_do_not_overlap_and_stay_clear_of_the_register_flag`).
pub(in crate::core) const DELAYED: u32 = 1 << 5;
/// The branch pushes a return address. VERIFIED: internal layout, as `DELAYED`.
pub(in crate::core) const CALL: u32 = 1 << 6;
/// `RTI`: the return also ends the service of an interrupt. VERIFIED: internal layout, as `DELAYED`.
pub(in crate::core) const RTI: u32 = 1 << 7;
/// Shift of the instruction length. VERIFIED: internal layout, as `DELAYED`.
const LEN_SHIFT: u32 = 8;
/// Mask of the instruction length (after the shift). VERIFIED: lengths 1..=3 fit two bits (the
/// longest instruction is 3 parcels, `visa/classify.rs`), `a_branch_word_round_trips_every_field`.
const LEN_MASK: u32 = 0b11;
/// `DO_COUNT`: the count comes from a universal register. VERIFIED: bit 31 is above any 16-bit
/// immediate count (`the_fields_do_not_overlap_and_stay_clear_of_the_register_flag`).
pub(in crate::core) const FROM_REG: u32 = 1 << 31;

/// A branch word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::core) struct Branch(pub u32);

impl Branch {
    /// Packs the fields of one branch instruction. `len` is the length in parcels (1..=3).
    pub fn new(cond: u8, len: u32, delayed: bool, call: bool, rti: bool) -> Self {
        Branch(
            u32::from(cond) & COND_MASK
                | if delayed { DELAYED } else { 0 }
                | if call { CALL } else { 0 }
                | if rti { RTI } else { 0 }
                | (len & LEN_MASK) << LEN_SHIFT,
        )
    }

    /// Condition code, 31 for an unconditional instruction.
    #[inline(always)]
    pub fn cond(self) -> u64 {
        u64::from(self.0 & COND_MASK)
    }

    /// Instruction length in parcels.
    #[inline(always)]
    pub fn len(self) -> u32 {
        self.0 >> LEN_SHIFT & LEN_MASK
    }

    #[inline(always)]
    pub fn delayed(self) -> bool {
        self.0 & DELAYED != 0
    }

    #[inline(always)]
    pub fn call(self) -> bool {
        self.0 & CALL != 0
    }

    #[inline(always)]
    pub fn rti(self) -> bool {
        self.0 & RTI != 0
    }
}

/// `JUMP_I` operand `a`: the DAG2 index and modify registers (0..=15 each, as `I[reg_i]` and
/// `M[reg_m]`).
#[inline(always)]
pub(in crate::core) fn indirect(reg_i: u8, reg_m: u8) -> u32 {
    u32::from(reg_i & 15) | u32::from(reg_m & 15) << 4
}

/// `(reg_i, reg_m)` of a `JUMP_I` operand `a`.
#[inline(always)]
pub(in crate::core) fn indirect_regs(a: u32) -> (usize, usize) {
    ((a & 15) as usize, (a >> 4 & 15) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_branch_word_round_trips_every_field() {
        for cond in [0u8, 5, 15, 16, 30, 31] {
            for len in 1..=3u32 {
                for bits in 0..8u32 {
                    let (delayed, call, rti) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0);
                    let b = Branch::new(cond, len, delayed, call, rti);
                    assert_eq!(b.cond(), u64::from(cond));
                    assert_eq!(b.len(), len);
                    assert_eq!((b.delayed(), b.call(), b.rti()), (delayed, call, rti));
                }
            }
        }
    }

    #[test]
    fn the_fields_do_not_overlap_and_stay_clear_of_the_register_flag() {
        let all = Branch::new(31, 3, true, true, true).0;
        assert_eq!(all, COND_MASK | DELAYED | CALL | RTI | 3 << LEN_SHIFT);
        assert_eq!(all & FROM_REG, 0);
        let masks = [COND_MASK, DELAYED, CALL, RTI, LEN_MASK << LEN_SHIFT];
        for (i, a) in masks.iter().enumerate() {
            for b in &masks[i + 1..] {
                assert_eq!(a & b, 0);
            }
        }
        // a condition code wider than five bits cannot leak into the flags
        assert_eq!(
            Branch::new(0xFF, 1, false, false, false).0 & !COND_MASK,
            1 << 8
        );
    }

    #[test]
    fn the_indirect_registers_round_trip_and_are_masked_to_a_register_index() {
        for i in 0..16u8 {
            for m in 0..16u8 {
                assert_eq!(
                    indirect_regs(indirect(i, m)),
                    (usize::from(i), usize::from(m))
                );
            }
        }
        assert_eq!(indirect_regs(indirect(0x1F, 0x2A)), (15, 10));
    }
}
