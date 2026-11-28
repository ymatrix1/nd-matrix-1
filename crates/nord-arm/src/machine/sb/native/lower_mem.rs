//! OWNER: W2-NMEM. Native lowering of loads and stores (kinds `112..=128`, family `k::Family::Mem`).
//!
//! Spec (design note 9, WP W2-NMEM). Every access goes through the page table of `NativeCtl`
//! (`access.rs`): `e = table[a >> 12]`, a load needs a mapped page without `HOOK_R`
//! (`abi::MASK_LOAD`), a store a mapped page with neither `HOOK_W` nor `HOOK_C`
//! (`abi::MASK_STORE`), and the host address is `arena - 4096 + (index << 12) + (a & 0xFFF)`.
//! Anything else is `NCtx::side_exit_generic_if` at the pc of the instruction, with the state
//! exact before it (the address, the new base and the loaded value are SSA values; no register is
//! written before the check), so the driver runs one `Cpu::step` through `SysBus`: device models,
//! the poll breaker, on-demand mapping and the stale queue all behave as in the interpreter.
//! A store that can leave `stale_code` non-empty needs no code of its own: the page of decoded
//! code carries `HOOK_C`, so that store is a slow case, the interpreter performs it and the
//! driver drains the queue (I6; tier 2 reaches the same state through its `Stale` exit).
//!
//! | Uops | Module | Fast path |
//! |---|---|---|
//! | `LDR/LDRB/STR/STRB` immediate and register offset, `LDRH LDRSH LDRSB STRH LDRD STRD` | `single` | one entry check per access; a word is read at `a & !3` and rotated right by `8 * (a & 3)`, a store goes to `a & !3`, a halfword to `a & !1`; `LDRD/STRD` need both words in one page |
//! | `LDM`, `STM` | `block` | one entry check and one range check (`(low & !3) & 0xFFF + 4 * n <= 0x1000`) for the whole transfer, then `n` plain accesses |
//! | `LDR_ABS` | `single` | the page entry of a constant address |
//!
//! The semantics are `exec_mem.rs`'s (the T2 arms, which are tested against the interpreter's
//! handlers): write-back before the loaded register (`rn == rd` keeps the loaded value), a store
//! reads its register before the write-back, `STM` writes the base back after the store of its
//! lowest register, `LDM` skips the write-back when the base is in the list. Shapes the arms
//! refuse or the contract keeps out of this family (`r15` anywhere, `ROR #0` = RRX register
//! offsets, an empty or `r15` list, a pair that reaches `r15`) return `Err(Unsupported)` before
//! anything is emitted.
//!
//! Mutants `850..=879` are the sites of this module and its children; `mutant(id)` is false
//! outside a test build. `tests.rs` kills every one of them with the differential suite.

use super::emit::{NCtx, Unsupported};
use crate::machine::sb::uop::{k, Uop};

mod access;
mod block;
mod single;

#[cfg(test)]
pub(crate) mod harness;
#[cfg(test)]
mod kill;
#[cfg(test)]
mod suites;
#[cfg(test)]
pub(crate) mod tests;

/// Mutation site (layer L5): true only inside `mutants::with(id, ..)` of a test build.
#[cfg(test)]
#[inline(always)]
pub(crate) fn mutant(id: u32) -> bool {
    crate::machine::sb::mutants::is_active(id)
}

/// Mutation site: false outside a test build, so the wrong branch is dead code there.
#[cfg(not(test))]
#[inline(always)]
pub(crate) const fn mutant(_id: u32) -> bool {
    false
}

/// `(id, name)` of the mutation sites of the memory lowerer (block `850..=879` of `mutants.rs`).
#[cfg(test)]
pub(crate) const MUTANTS: &[(u32, &str)] = &[
    (850, "native mem: a load ignores HOOK_R"),
    (851, "native mem: a store ignores HOOK_C and HOOK_W"),
    (852, "native mem: an unaligned word load is not rotated"),
    (853, "native mem: a word load uses the unaligned address"),
    (854, "native mem: a halfword access uses the odd address"),
    (855, "native mem: ldrsb zero-extends"),
    (856, "native mem: ldrsh zero-extends"),
    (857, "native mem: the write-back is skipped"),
    (858, "native mem: the U flag is inverted"),
    (
        859,
        "native mem: the P flag is ignored (always pre-indexed)",
    ),
    (
        860,
        "native mem: a load with rn == rd lets the write-back win",
    ),
    (861, "native mem: a store reads rd after the write-back"),
    (862, "native mem: ldrd swaps the halves"),
    (863, "native mem: ldrd/strd skip the page-end check"),
    (864, "native mem: ldm/stm skip the page-end check"),
    (
        865,
        "native mem: stm writes the base back before the first store",
    ),
    (
        866,
        "native mem: ldm writes the base back although it is in the list",
    ),
    (
        867,
        "native mem: ldm/stm decrement-before starts one word too low",
    ),
    (868, "native mem: a register offset ignores the shift type"),
    (
        869,
        "native mem: a register offset LSR #0 shifts by 0, not 32",
    ),
    (
        870,
        "native mem: a register offset ASR #0 shifts by 0, not 32",
    ),
    (871, "native mem: ldr_abs does not rotate"),
    (
        872,
        "native mem: the side exit reports the next instruction",
    ),
    (873, "native mem: the page-end check is off by one word"),
    (874, "native mem: strb stores a halfword"),
    (875, "native mem: strh stores a byte"),
    (877, "native mem: ldm loads descending"),
    (
        878,
        "native mem: a register offset is added whatever U says",
    ),
    (879, "native mem: a halfword register offset is shifted"),
];

/// Emit the native code of `u` (loads and stores).
pub fn lower_mem(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
    match u.kind {
        k::LDR_IMM..=k::STRB_REG | k::LDRH..=k::STRD => single::lower_single(cx, u),
        k::LDM | k::STM => block::lower_block(cx, u),
        k::LDR_ABS => single::lower_abs(cx, u),
        _ => Err(Unsupported),
    }
}

#[cfg(test)]
mod unit {
    use super::*;
    use crate::machine::sb::native::emit::testing::build_and_verify;

    #[test]
    fn only_the_memory_family_is_lowered_here() {
        let sb = tests::one(Uop::new(k::LDR_IMM, 0x1000));
        build_and_verify(&sb, |cx| {
            let before = cx.b.func.dfg.num_insts();
            for kind in [k::ADD_IMM, k::EXIT, k::GUARD, k::GENERIC] {
                assert_eq!(
                    lower_mem(cx, &Uop::new(kind, 0x1000)),
                    Err(Unsupported),
                    "{kind}"
                );
            }
            assert_eq!(cx.b.func.dfg.num_insts(), before);
            cx.exit_generic(0x1000);
        });
    }
}
