//! Lowering: from the decoded instructions of a code range to blocks of uops.
//!
//! OWNER: W1-LOWER (this file and `scan`, `cut`, `budget`, `guard`, `pcmap`, `decode`,
//! `builder`); `float`, `fixed`, `moves`, `flow` belong to W2-FLOAT, W2-FIXED, W2-MOVE and
//! W2-FLOW. Registration of the families is STATIC: [`lower_insn`] asks the four [`Family`]
//! implementations in a fixed order and falls back to an `Interp` uop, so a family that lowers
//! nothing leaves its instructions exact (design section 8) and the packages never edit this file
//! for a new family member.
//!
//! Skeleton: [`translate_range`] validates the range and returns no blocks, so the interpreter
//! runs everything.
pub mod budget;
pub mod builder;
pub mod cut;
pub mod decode;
pub mod fixed;
pub mod float;
pub mod flow;
pub mod guard;
pub mod moves;
pub mod pcmap;
pub mod scan;

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
/// of `Core::step`.
pub(crate) fn translate_range(
    core: &mut Core,
    cfg: &UopConfig,
    lo: u32,
    hi: u32,
) -> Result<(Vec<Block>, Vec<Uop>), UopError> {
    if lo >= hi {
        return Err(UopError::OutOfRange { lo, hi });
    }
    let _ = (core, cfg);
    Ok((Vec::new(), Vec::new()))
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
    fn the_skeleton_translates_nothing_and_rejects_an_empty_range() {
        let mut c = core_with(&[0x0001, 0x0001]);
        let cfg = UopConfig::default();
        let pc = c.pc;
        let (blocks, uops) = translate_range(&mut c, &cfg, pc, pc + 2).unwrap();
        assert!(blocks.is_empty() && uops.is_empty());
        assert_eq!(
            translate_range(&mut c, &cfg, 5, 5).unwrap_err(),
            UopError::OutOfRange { lo: 5, hi: 5 }
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
