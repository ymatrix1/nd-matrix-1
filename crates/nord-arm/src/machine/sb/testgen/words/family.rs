//! OWNER: W1-VGEN. The instruction families of `words.rs`.

/// An instruction family: the unit of the per-family tests of the lowering and executor work
/// packages. [`Family::ALL`] lists every one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Family {
    /// Data processing, rotated immediate (form 0); all 16 opcodes, `S` free (forced for compares).
    DpImm,
    /// Data processing, register shifted by an immediate (form 1).
    DpShiftImm,
    /// Data processing, register shifted by a register (form 2).
    DpShiftReg,
    /// `mul`, `mla`.
    Mul,
    /// `umull`, `umlal`, `smull`, `smlal`.
    MulLong,
    /// `clz`.
    Clz,
    /// `qadd`, `qsub`, `qdadd`, `qdsub`.
    Qadd,
    /// `smla<x><y>`, `smulw<y>`, `smlal<x><y>`, `smul<x><y>`.
    DspMul,
    /// `ldr`, `ldrb` with an immediate or a shifted register offset, any P/U/W.
    Ldr,
    /// `str`, `strb`, same addressing.
    Str,
    /// `ldrh`, `ldrsb`, `ldrsh`, `strh` with an immediate or register offset, any P/U/W.
    Half,
    /// `ldrd`, `strd` (odd and `r14` pairs included).
    Double,
    /// `ldm` in all four modes, with and without write-back, the `S` bit and `pc` in the list.
    Ldm,
    /// `stm`, same.
    Stm,
    /// `swp`, `swpb`.
    Swp,
    /// `b`.
    B,
    /// `bl`.
    Bl,
    /// `bx rm`.
    Bx,
    /// `blx rm`.
    BlxReg,
    /// `blx <label>` (condition field 15).
    BlxImm,
    /// `mrs`.
    Mrs,
    /// `msr`, register and immediate forms, any field mask, `cpsr` and `spsr`.
    Msr,
    /// `mrc`/`mcr` p15, the control register, the ID registers and `wfi`.
    Cp15,
    /// `swi`.
    Swi,
    /// Words the interpreter reports as `Fault::Undefined`.
    Undefined,
}

impl Family {
    pub const ALL: [Family; 25] = [
        Family::DpImm,
        Family::DpShiftImm,
        Family::DpShiftReg,
        Family::Mul,
        Family::MulLong,
        Family::Clz,
        Family::Qadd,
        Family::DspMul,
        Family::Ldr,
        Family::Str,
        Family::Half,
        Family::Double,
        Family::Ldm,
        Family::Stm,
        Family::Swp,
        Family::B,
        Family::Bl,
        Family::Bx,
        Family::BlxReg,
        Family::BlxImm,
        Family::Mrs,
        Family::Msr,
        Family::Cp15,
        Family::Swi,
        Family::Undefined,
    ];

    /// True for the families whose words never fault on the interpreter (`Cp15` only when it is
    /// not `wfi`; `Swi` and `Undefined` always do).
    pub fn is_fault_free(self) -> bool {
        !matches!(self, Family::Cp15 | Family::Swi | Family::Undefined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_family_is_listed_once_and_only_the_fault_families_fault() {
        let all: BTreeSet<Family> = Family::ALL.into_iter().collect();
        assert_eq!(all.len(), Family::ALL.len(), "no family twice");
        let faulty: Vec<Family> = Family::ALL
            .into_iter()
            .filter(|f| !f.is_fault_free())
            .collect();
        assert_eq!(faulty, vec![Family::Cp15, Family::Swi, Family::Undefined]);
    }
}
