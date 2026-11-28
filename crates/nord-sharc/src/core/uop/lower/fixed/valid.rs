//! Which fixed-point compute words the family lowers: the opcode lists and the multifunction
//! decision, each tied to the interpreter by a test that asks `Core::compute` about every
//! opcode.
//!
//! OWNER: W2-FIXED. A word outside these lists stays an `Interp` uop (exact by construction):
//! the bit FIFO shifter forms, the multiplier codes that are no operation, the float
//! multifunction forms (the float family's) and every code the interpreter itself does not model.
use crate::core::uop::exec::h_fixed::kinds::{ALU_OPS, XALU_BASE};
use crate::core::uop::exec::h_fixed::SHIFT_OPS;
use crate::core::uop::types::UopKind;

/// Whether fixed ALU opcode `op` (`< 0x70`) is modelled. VERIFIED: equal to the set of opcodes
/// for which `Core::compute` returns true, by `the_alu_opcode_list_is_the_interpreters`.
pub fn alu_modelled(op: u8) -> bool {
    ALU_OPS.contains(&op)
}

/// The kind of multiplier opcode `op` (unit 1), `None` when the interpreter does not model it or
/// the float family owns it (`0x30` is the float multiply). VERIFIED against `Core::compute` for
/// all 256 codes by `the_multiplier_lists_are_the_interpreters`.
///
/// Classes 1..3 (`op >= 0x40`) are products; class 0 holds `MRx = 0` (`0x14`, `0x16`), `SAT`
/// (`0x00..=0x0F`) and `RND` (`0x18..=0x1F`).
pub fn mult_kind(op: u8) -> Option<UopKind> {
    match op {
        0x40..=0xFF => Some(UopKind::XMUL),
        0x00..=0x0F | 0x14 | 0x16 | 0x18..=0x1F => Some(UopKind::XMULS),
        _ => None,
    }
}

/// Whether shifter opcode `op` (unit 2) is lowered: modelled by the interpreter and not a bit
/// FIFO form. VERIFIED against `Core::compute` by `the_shifter_list_is_the_interpreters`.
pub fn shift_modelled(op: u8) -> bool {
    SHIFT_OPS.contains(&op)
}

/// The kind of the compute word `c` (23 bits) when bit 22 is set (the multifunction and the MR
/// transfer forms), `None` for the float forms and the codes that are no operation.
/// VERIFIED against `Core::compute` by `the_multifunction_forms_are_the_interpreters`.
///
/// * bits 21..17 clear: `Rn = MRxF/B` or `MRxF/B = Rn` when the operation code (bits 19..12) is
///   `0..=3`, `4..=7` (select the background register) with a field other than 3, optionally
///   with bit 4 (write): `op & !0x17 == 0` and `op & 3 != 3`;
/// * bit 21 set: bit 20 clear is the fixed triple form (`Rm = Rxm*Rym, Ra = Rxa+Rya, Rs =
///   Rxa-Rya`), bit 20 set the float one;
/// * bit 21 clear: `op6 = bits 21..16`; `op6 >> 3 == 3` is a float multiplier, groups 1..=5 with
///   a low pair of 0..=2 are the fixed multiplier plus ALU forms.
pub fn multi_kind(c: u32) -> Option<UopKind> {
    if c >> 22 & 1 == 0 {
        return None;
    }
    if (c >> 17) & 0x1F == 0 {
        let op = (c >> 12) & 0xFF;
        return (op & 3 != 3 && op & !0x17 == 0).then_some(UopKind::XMRT);
    }
    if (c >> 21) & 1 == 1 {
        return ((c >> 20) & 1 == 0).then_some(UopKind::XMF3);
    }
    let op6 = (c >> 16) & 63;
    (op6 >> 3 != 3 && (1..=5).contains(&(op6 >> 2)) && op6 & 3 <= 2).then_some(UopKind::XMF2)
}

/// Whether `c` (bit 22 set) is a float multifunction form: the float family lowers it, this one
/// declines it.
pub fn float_multi(c: u32) -> bool {
    c >> 22 & 1 == 1
        && (c >> 17) & 0x1F != 0
        && if (c >> 21) & 1 == 1 {
            (c >> 20) & 1 == 1
        } else {
            (c >> 19) & 7 == 3
        }
}

/// The ALU kind of opcode `op`, `None` when it is not modelled.
pub fn alu_kind(op: u8) -> Option<UopKind> {
    alu_modelled(op).then_some(UopKind(XALU_BASE + u16::from(op)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::Core;

    /// `Core::compute` of the word `c` on `core`. Whether an operation is modelled does not
    /// depend on the state, so one core serves a whole sweep (a fresh one costs a 4 MiB image).
    fn modelled(core: &mut Core, c: u32) -> bool {
        core.compute(u64::from(c))
    }

    fn word(unit: u32, op: u32) -> u32 {
        unit << 20 | op << 12 | 3 << 8 | 1 << 4 | 2
    }

    #[test]
    fn the_alu_opcode_list_is_the_interpreters() {
        let mut core = core_with(&[0x0001]);
        for op in 0..=0x6Fu8 {
            assert_eq!(
                modelled(&mut core, word(0, u32::from(op))),
                alu_modelled(op),
                "fixed ALU op {op:#x}"
            );
            assert_eq!(alu_kind(op).is_some(), alu_modelled(op));
        }
        assert_eq!(
            ALU_OPS.len(),
            (0..=0x6Fu8).filter(|&o| alu_modelled(o)).count()
        );
        // the dual add/subtract opcodes 0x70..0x7F are all modelled (their own kind)
        for op in 0x70..=0x7F {
            assert!(modelled(&mut core, word(0, op)), "dual op {op:#x}");
        }
        assert_eq!(alu_kind(0x01), Some(UopKind(0x201)));
        assert_eq!(alu_kind(0x03), None);
    }

    #[test]
    fn the_multiplier_lists_are_the_interpreters() {
        let mut core = core_with(&[0x0001]);
        for op in (0..=0xFFu32).filter(|&o| o != 0x30) {
            assert_eq!(
                modelled(&mut core, word(1, op)),
                mult_kind(op as u8).is_some(),
                "multiplier op {op:#x}"
            );
        }
        assert_eq!(
            mult_kind(0x30),
            None,
            "the float multiply is the float family's"
        );
        assert_eq!(mult_kind(0x78), Some(UopKind::XMUL));
        assert_eq!(mult_kind(0x14), Some(UopKind::XMULS));
        assert_eq!(mult_kind(0x13), None, "integer RND is no operation");
        assert_eq!(mult_kind(0x20), None);
    }

    #[test]
    fn the_shifter_list_is_the_interpreters() {
        const FIFO: [u32; 5] = [0x50, 0x58, 0x70, 0x74, 0x7C];
        let mut core = core_with(&[0x0001]);
        for op in 0..=0xFFu32 {
            let lowered = shift_modelled(op as u8);
            assert_eq!(
                modelled(&mut core, word(2, op)),
                lowered || FIFO.contains(&op),
                "shifter op {op:#x}"
            );
            assert!(!(lowered && FIFO.contains(&op)));
        }
        assert_eq!(SHIFT_OPS.iter().filter(|&&o| shift_modelled(o)).count(), 21);
    }

    #[test]
    fn the_multifunction_forms_are_the_interpreters() {
        let mut kinds = [0usize; 3];
        let mut core = core_with(&[0x0001]);
        for bits2120 in 0..4u32 {
            for op in 0..=0xFFu32 {
                for low in [0x000u32, 0xFFF, 0x2A5, 0x4C3] {
                    let c = 1 << 22 | bits2120 << 20 | op << 12 | low;
                    let kind = multi_kind(c);
                    assert_eq!(
                        modelled(&mut core, c),
                        kind.is_some() || float_multi(c),
                        "word {c:#x}"
                    );
                    assert!(
                        kind.is_none() || !float_multi(c),
                        "a word is the float family's or this one's, never both ({c:#x})"
                    );
                    match kind {
                        Some(UopKind::XMRT) => kinds[0] += 1,
                        Some(UopKind::XMF2) => kinds[1] += 1,
                        Some(UopKind::XMF3) => kinds[2] += 1,
                        _ => {}
                    }
                }
            }
        }
        assert!(
            kinds.iter().all(|&n| n > 0),
            "every kind was met: {kinds:?}"
        );
        assert_eq!(multi_kind(0), None, "bit 22 clear");
        assert_eq!(multi_kind(0x40_3000), None, "there is no MR3 transfer");
    }
}
