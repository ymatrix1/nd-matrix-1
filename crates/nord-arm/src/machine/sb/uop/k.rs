//! OWNER: S0 (frozen). The kind ids of the micro-ops (`Uop::kind`), the static family registry
//! every dispatcher goes through, and the store predicate.
//!
//! Spec: `docs/research/arm0-jit-design.md` section 3.1. Ids are part of the frozen contract;
//! the gaps 129..=143 and 155..=159 are reserved, not allocated (108..=111 went to the 32-bit
//! multiplies, work package W2-MUL). The generic
//! data-processing ids equal the predecode handler ids minus `op::DP_BASE` (a test below).

/// First generic data-processing id.
pub const DP_BASE: u8 = 0;
/// Generic data-processing id: `form * 32 + opcode * 2 + s` (form 0 immediate, 1 register
/// shifted by an immediate, 2 register shifted by a register); ids `0..=95`.
pub const fn dp(form: u32, opcode: u32, s: bool) -> u8 {
    (form * 32 + opcode * 2 + s as u32) as u8
}

pub const MOV_IMM: u8 = 96;
pub const MOV_REG: u8 = 97;
pub const MVN_IMM: u8 = 98;
pub const ADD_IMM: u8 = 99;
pub const SUB_IMM: u8 = 100;
pub const ADD_REG: u8 = 101;
pub const SUB_REG: u8 = 102;
pub const CMP_IMM: u8 = 103;
pub const CMP_REG: u8 = 104;
pub const TST_IMM: u8 = 105;
pub const TST_REG: u8 = 106;
pub const ORR_IMM: u8 = 107;

/// `MUL`: `rd = rm * rs`, flags untouched. The four multiply ids are `MUL + acc * 2 + s`.
pub const MUL: u8 = 108;
/// `MULS`: `MUL` setting N and Z from the result (C and V unchanged on ARMv5).
pub const MULS: u8 = 109;
/// `MLA`: `rd = rm * rs + rn`.
pub const MLA: u8 = 110;
/// `MLAS`: `MLA` setting N and Z from the result.
pub const MLAS: u8 = 111;

/// Multiply id of `MUL` (`acc` false) or `MLA` (`acc` true), `S` set or not.
pub const fn mul(acc: bool, s: bool) -> u8 {
    MUL + (acc as u8) * 2 + s as u8
}

pub const LDR_IMM: u8 = 112;
pub const LDRB_IMM: u8 = 113;
pub const STR_IMM: u8 = 114;
pub const STRB_IMM: u8 = 115;
pub const LDR_REG: u8 = 116;
pub const LDRB_REG: u8 = 117;
pub const STR_REG: u8 = 118;
pub const STRB_REG: u8 = 119;
pub const LDRH: u8 = 120;
pub const LDRSH: u8 = 121;
pub const LDRSB: u8 = 122;
pub const STRH: u8 = 123;
pub const LDRD: u8 = 124;
pub const STRD: u8 = 125;
pub const LDM: u8 = 126;
pub const STM: u8 = 127;
pub const LDR_ABS: u8 = 128;

pub const PRED: u8 = 144;
pub const GUARD: u8 = 145;
pub const GUARD_REG_EQ: u8 = 146;
pub const LOOP_BACK: u8 = 147;
pub const EXIT: u8 = 148;
pub const EXIT_REG: u8 = 149;
pub const EXIT_REG_ARM: u8 = 150;
pub const GENERIC: u8 = 151;
pub const GENERIC_END: u8 = 152;
pub const MRS: u8 = 153;
pub const MSR_C: u8 = 154;

pub const CMP_GUARD_I: u8 = 160;
pub const CMP_GUARD_R: u8 = 161;
pub const TST_GUARD_I: u8 = 162;
pub const MOV_PAIR: u8 = 163;
pub const LDR_PAIR: u8 = 164;

/// One past the highest allocated id.
pub const COUNT: usize = 165;
/// Allocated ids (the rest of `0..COUNT` is reserved and `unreachable!` in an executor).
pub const DEFINED: usize = 145;

/// Which executor module (and native lowering module) owns a kind id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// `0..=111`: data processing, generic and specialised, and the 32-bit multiplies
    /// (`exec_alu`, `native::lower_dp`).
    Dp,
    /// `112..=128`: loads and stores (`exec_mem`, `native::lower_mem`).
    Mem,
    /// `144..=154`: predicates, guards, exits, `GENERIC*`, `MRS`, `MSR_C` (`exec_flow`).
    Flow,
    /// `160..=164`: fused uops, each equal to its unfused expansion (`exec_flow`).
    Fused,
    /// Not an allocated id.
    Invalid,
}

/// Static registry: the family of a kind id. Dispatchers match on this and call the owner's
/// function, so a work package never edits another one's file to register its arms.
pub const fn family(kind: u8) -> Family {
    match kind {
        0..=111 => Family::Dp,
        112..=128 => Family::Mem,
        144..=154 => Family::Flow,
        160..=164 => Family::Fused,
        _ => Family::Invalid,
    }
}

/// True for the memory uops that write guest memory (they may leave `stale_code` non-empty,
/// I6, so each one needs an early exit).
pub const fn is_store(kind: u8) -> bool {
    matches!(
        kind,
        STR_IMM | STRB_IMM | STR_REG | STRB_REG | STRH | STRD | STM
    )
}

#[cfg(test)]
mod tests {
    use crate::cpu::predecode::op;
    use crate::machine::sb::uop::k;

    #[test]
    fn every_defined_kind_is_unique_below_count_and_has_a_family() {
        let defined: Vec<u8> = (0..=255u8)
            .filter(|k| k::family(*k) != k::Family::Invalid)
            .collect();
        assert_eq!(defined.len(), k::DEFINED);
        assert!(defined.iter().all(|k| (*k as usize) < k::COUNT));
        // every named constant is in the table, none appears twice
        let named = [
            k::MOV_IMM,
            k::MOV_REG,
            k::MVN_IMM,
            k::ADD_IMM,
            k::SUB_IMM,
            k::ADD_REG,
            k::SUB_REG,
            k::CMP_IMM,
            k::CMP_REG,
            k::TST_IMM,
            k::TST_REG,
            k::ORR_IMM,
            k::MUL,
            k::MULS,
            k::MLA,
            k::MLAS,
            k::LDR_IMM,
            k::LDRB_IMM,
            k::STR_IMM,
            k::STRB_IMM,
            k::LDR_REG,
            k::LDRB_REG,
            k::STR_REG,
            k::STRB_REG,
            k::LDRH,
            k::LDRSH,
            k::LDRSB,
            k::STRH,
            k::LDRD,
            k::STRD,
            k::LDM,
            k::STM,
            k::LDR_ABS,
            k::PRED,
            k::GUARD,
            k::GUARD_REG_EQ,
            k::LOOP_BACK,
            k::EXIT,
            k::EXIT_REG,
            k::EXIT_REG_ARM,
            k::GENERIC,
            k::GENERIC_END,
            k::MRS,
            k::MSR_C,
            k::CMP_GUARD_I,
            k::CMP_GUARD_R,
            k::TST_GUARD_I,
            k::MOV_PAIR,
            k::LDR_PAIR,
        ];
        let mut seen = std::collections::BTreeSet::new();
        for id in named {
            assert!(seen.insert(id), "kind id {id} used twice");
            assert!(defined.contains(&id), "kind id {id} has no family");
        }
        assert_eq!(
            seen.len() + 96,
            k::DEFINED,
            "96 generic DP ids + the named ones"
        );
        assert_eq!(*defined.last().unwrap() as usize, k::COUNT - 1);
    }

    #[test]
    fn the_generic_dp_ids_match_the_predecode_handler_ids() {
        for form in 0..3 {
            for opcode in 0..16 {
                for s in [false, true] {
                    assert_eq!(
                        k::dp(form, opcode, s),
                        op::dp(form, opcode, s) - op::DP_BASE,
                        "{form} {opcode} {s}"
                    );
                    assert_eq!(k::family(k::dp(form, opcode, s)), k::Family::Dp);
                }
            }
        }
        assert_eq!(k::dp(2, 15, true), 95);
    }

    #[test]
    fn the_multiply_ids_are_mul_plus_acc_times_two_plus_s_and_are_data_processing() {
        assert_eq!(
            [k::mul(false, false), k::mul(false, true)],
            [k::MUL, k::MULS]
        );
        assert_eq!([k::mul(true, false), k::mul(true, true)], [k::MLA, k::MLAS]);
        assert_eq!([k::MUL, k::MULS, k::MLA, k::MLAS], [108, 109, 110, 111]);
        for id in k::MUL..=k::MLAS {
            assert_eq!(k::family(id), k::Family::Dp, "{id}");
        }
        assert_eq!(
            k::family(129),
            k::Family::Invalid,
            "the other gaps stay reserved"
        );
    }

    #[test]
    fn stores_are_exactly_the_memory_uops_that_write() {
        let stores: Vec<u8> = (112..=128).filter(|k| k::is_store(*k)).collect();
        assert_eq!(stores, [114, 115, 118, 119, 123, 125, 127]);
    }
}
