//! Lowering: from the decoded instructions of a code range to blocks of uops.
//!
//! OWNER: W1-LOWER (this file and `scan`, `cut`, `cbwrap`, `group`, `translate`, `budget`,
//! `guard`, `pcmap`, `decode`, `builder`, and the test-only `support` and `diff`); `float`, `fixed`,
//! `moves`, `flow` belong to W2-FLOAT, W2-FIXED, W2-MOVE and W2-FLOW. Registration of the
//! families is STATIC: [`lower_insn`] asks the four [`Family`] implementations in a fixed order
//! and falls back to an `Interp` uop, so a family that lowers nothing leaves its instructions
//! exact (design section 8) and the packages never edit this file for a new family member.
//!
//! [`translate_range`] walks the range once (`scan`), cuts it into blocks (`cut`, `translate`)
//! for both PEy modes and lowers every instruction with [`lower_insn`]; an instruction no family
//! takes is an `Interp` uop, which is exact by construction.
pub mod budget;
pub mod builder;
pub mod cbwrap;
pub mod cut;
pub mod decode;
pub mod fixed;
pub mod float;
pub mod flow;
pub mod group;
pub mod guard;
pub mod moves;
pub mod pcmap;
pub mod scan;
pub mod translate;

#[cfg(test)]
mod diff;
#[cfg(test)]
mod support;

use super::config::{UopConfig, UopError};
use super::types::{Block, Uop};
use crate::core::decode_cache::CachedInsn;
use crate::core::Core;
use builder::BlockBuilder;

/// What a [`Family`] needs to know about the block being built.
pub(crate) struct LowerCtx<'a> {
    pub cfg: &'a UopConfig,
    /// pc the pass loop must see (the pass idle pc): a block ends at it.
    pub stop_pc: Option<u32>,
    /// PEy participates (`core.simd()` at translation time).
    pub simd: bool,
    /// ISA-mode code (`isa_at` of the block pc).
    pub isa: bool,
}

/// A family of instructions with fast uops (float, fixed, moves, flow).
pub(in crate::core) trait Family {
    /// Emits the uops of `c` into `out` and returns true, or returns false (emitting nothing)
    /// so the caller emits an `Interp` uop. Must be exact: whatever it emits equals the
    /// interpreter step bit for bit, including hidden state and the step count.
    fn lower(cx: &LowerCtx<'_>, c: &CachedInsn, out: &mut BlockBuilder) -> bool;
}

/// Lowers one instruction: the first family that takes it, else an `Interp` uop.
pub(in crate::core) fn lower_insn(cx: &LowerCtx<'_>, c: &CachedInsn, out: &mut BlockBuilder) {
    let taken = float::FloatFamily::lower(cx, c, out)
        || fixed::FixedFamily::lower(cx, c, out)
        || moves::MoveFamily::lower(cx, c, out)
        || flow::FlowFamily::lower(cx, c, out);
    if !taken {
        out.interp(c);
    }
    out.set_end(c.next_pc);
}

/// Translates the short words `lo..hi` into non-overlapping basic blocks and their uops (block
/// `first` indexes are relative to the returned vector). Reads the code through the decode path
/// of `Core::step`. Every range is translated twice, once per PEy mode (rule P4), the PEy-off
/// blocks first. `core.uop_run.stop_pc` is the pass idle pc a block must end at (none when
/// `NONE`); the caller sets it before translating (`uop_maintain` keeps it, and `enable_uop`
/// saves it across `disable_uop` and restores it before it translates).
pub(crate) fn translate_range(
    core: &mut Core,
    cfg: &UopConfig,
    lo: u32,
    hi: u32,
) -> Result<(Vec<Block>, Vec<Uop>), UopError> {
    if lo >= hi {
        return Err(UopError::OutOfRange { lo, hi });
    }
    Ok(translate::translate_with(core, cfg, lo, hi, lower_insn))
}

/// Test helper shared by the family tests: lowers the first instruction of `program` with family
/// `F` and returns whether `F` took it and how many uops it emitted.
#[cfg(test)]
pub(in crate::core) fn lower_first<F: Family>(program: &[u16]) -> (bool, usize) {
    let mut core = crate::core::support::core_with(program);
    let pc = core.pc;
    let insn = decode::decode_at(&mut core, pc);
    let cfg = UopConfig::default();
    let cx = LowerCtx {
        cfg: &cfg,
        stop_pc: None,
        simd: false,
        isa: false,
    };
    let mut b = BlockBuilder::new(pc, false, false);
    let taken = F::lower(&cx, &insn, &mut b);
    (taken, b.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::uop::types::UopKind;

    #[test]
    fn an_empty_or_reversed_range_is_rejected() {
        let mut c = core_with(&[0x0001, 0x0001]);
        let cfg = UopConfig::default();
        let pc = c.pc;
        for (lo, hi) in [(5, 5), (pc + 2, pc)] {
            assert_eq!(
                translate_range(&mut c, &cfg, lo, hi).unwrap_err(),
                UopError::OutOfRange { lo, hi }
            );
        }
        let (blocks, uops) = translate_range(&mut c, &cfg, pc, pc + 2).unwrap();
        assert_eq!(
            (blocks.len(), uops.len()),
            (2, 4),
            "two NOPs, both PEy modes"
        );
    }

    #[test]
    fn an_instruction_no_family_takes_becomes_an_interp_uop() {
        let mut c = core_with(&[0x0001]);
        let pc = c.pc;
        let insn = decode::decode_at(&mut c, pc);
        let cfg = UopConfig::default();
        let cx = LowerCtx {
            cfg: &cfg,
            stop_pc: None,
            simd: false,
            isa: false,
        };
        let mut b = BlockBuilder::new(pc, false, false);
        lower_insn(&cx, &insn, &mut b);
        let (block, uops) = b.finish(Default::default());
        assert_eq!((block.steps, block.len, uops.len()), (1, 1, 1));
        assert_eq!((uops[0].h, uops[0].pc), (UopKind::INTERP.0, pc));
    }
}
