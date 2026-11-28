//! Block assembly: collects the uops of one basic block and derives its budget, cumulative step
//! table and footprint.
//!
//! OWNER: W1-LOWER. Skeleton: `push`, `interp` and `finish` are complete for an all-`Interp`
//! block, which is exact by construction (the footprint of an `Interp` uop is the conservative
//! "everything"); the package adds the guard class, the code hash and the footprint of fast uops.
use super::super::types::{Block, Footprint, GuardClass, Link, Uop, UopKind, NONE, UF_LAST};
use super::budget;
use crate::core::decode_cache::CachedInsn;

/// Builds one block starting at `pc`.
pub(in crate::core) struct BlockBuilder {
    pc: u32,
    simd: bool,
    isa: bool,
    uops: Vec<Uop>,
    footprint: Footprint,
    ended: bool,
    owed: u8,
    end_pc: u32,
}

impl BlockBuilder {
    /// An empty block at `pc`, translated for `simd` and `isa`.
    pub fn new(pc: u32, simd: bool, isa: bool) -> Self {
        BlockBuilder {
            pc,
            simd,
            isa,
            uops: Vec::new(),
            footprint: Footprint::default(),
            ended: false,
            owed: 0,
            end_pc: pc,
        }
    }

    /// Records that the instruction just lowered ends at `next_pc` (the pc after it). `interp`
    /// does it itself and `lower_insn` does it for every instruction, so a family never has to.
    pub fn set_end(&mut self, next_pc: u32) {
        self.end_pc = next_pc;
    }

    /// Protocol hook for the families (design 11.1): the instruction just lowered ends the block
    /// (jump, call, return, system-register access, IDLE). The translator cuts after it whatever
    /// the budget says, and no further instruction may be added.
    pub fn end_block(&mut self) {
        self.ended = true;
    }

    /// Whether [`BlockBuilder::end_block`] was called.
    pub fn ends(&self) -> bool {
        self.ended
    }

    /// Protocol hook for the flow family: a delayed branch was lowered and `n` delay slots are
    /// still owed. The translator must not cut before they are lowered (the branch resolves in
    /// `finish_step` of the last slot, which is where the block then ends).
    pub fn owe_slots(&mut self, n: u8) {
        self.owed = n;
    }

    /// Delay slots still owed after the last lowered instruction.
    pub fn owed_slots(&self) -> u8 {
        self.owed
    }

    /// Called by the translator after it lowered one more instruction: a slot is no longer owed.
    pub fn slot_done(&mut self) {
        self.owed = self.owed.saturating_sub(1);
    }

    /// Appends a uop.
    pub fn push(&mut self, u: Uop) {
        self.uops.push(u);
    }

    /// Appends the `Interp` uop of one instruction: one step at `c.pc`, `a` = ISA flag, `b` = the
    /// pc after the instruction. Its footprint is everything, because the interpreter may touch
    /// any state.
    pub fn interp(&mut self, c: &CachedInsn) {
        self.push(Uop::new(
            UopKind::INTERP,
            1,
            c.pc,
            u32::from(c.isa),
            c.next_pc,
        ));
        self.end_pc = c.next_pc;
        self.footprint = Footprint {
            reads: u64::MAX,
            writes: u64::MAX,
            touches_mem: true,
            touches_flags: true,
        };
    }

    /// Uops collected so far.
    pub fn len(&self) -> usize {
        self.uops.len()
    }

    /// Whether no uop was added.
    pub fn is_empty(&self) -> bool {
        self.uops.is_empty()
    }

    /// Ends the block: marks the last uop `UF_LAST` and computes the budget and the cumulative
    /// steps. `Block::first` is 0 here; the installer sets it when the uops enter the arena.
    pub fn finish(mut self, guard: GuardClass) -> (Block, Vec<Uop>) {
        if let Some(last) = self.uops.last_mut() {
            last.flags |= UF_LAST;
        }
        let (steps, cum) = budget::budget(&self.uops);
        let last_pc = self.uops.last().map_or(self.pc, |u| u.pc);
        let block = Block {
            pc: self.pc,
            last_pc,
            end_pc: self.end_pc,
            steps,
            first: 0,
            len: self.uops.len() as u32,
            cum,
            simd: self.simd,
            isa: self.isa,
            guard,
            links: [Link::NONE; 2],
            code_hash: 0,
            native: NONE,
            poisoned: false,
            footprint: self.footprint,
        };
        (block, self.uops)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::uop::lower::decode::decode_at;

    #[test]
    fn an_all_interp_block_has_one_step_per_instruction_and_a_last_flag() {
        let mut c = core_with(&[0x0001, 0x013e, 0x0000, 0x1312, 0x0001]);
        let (mut pc, mut b) = (c.pc, BlockBuilder::new(c.pc, true, false));
        assert!(b.is_empty());
        for _ in 0..3 {
            let insn = decode_at(&mut c, pc);
            b.interp(&insn);
            pc = insn.next_pc;
        }
        assert_eq!(b.len(), 3);
        let (block, uops) = b.finish(GuardClass::Plain);
        assert_eq!(
            (block.steps, block.len, block.simd, block.isa),
            (3, 3, true, false)
        );
        assert_eq!(&*block.cum, &[0, 1, 2]);
        assert_eq!((block.pc, block.last_pc), (c.pc, c.pc + 4));
        assert_eq!(block.end_pc, c.pc + 5, "the fall-through successor");
        assert_eq!(
            uops.iter().map(|u| u.flags & UF_LAST).collect::<Vec<_>>(),
            [0, 0, UF_LAST]
        );
        assert_eq!(uops[1].b, c.pc + 4, "b is the pc after the instruction");
        assert!(block.footprint.touches_mem && block.footprint.touches_flags);
        assert_eq!((block.native, block.poisoned), (NONE, false));
    }

    #[test]
    fn the_protocol_hooks_record_the_end_of_the_block_and_the_owed_delay_slots() {
        let mut b = BlockBuilder::new(0x12_0000, false, false);
        assert_eq!((b.ends(), b.owed_slots()), (false, 0));
        b.owe_slots(2);
        b.slot_done();
        assert_eq!(b.owed_slots(), 1);
        b.slot_done();
        b.slot_done();
        assert_eq!(b.owed_slots(), 0, "never below zero");
        b.end_block();
        assert!(b.ends());
    }

    #[test]
    fn an_empty_builder_finishes_to_an_empty_block() {
        let (block, uops) = BlockBuilder::new(0x12_0000, false, false).finish(GuardClass::Plain);
        assert!(uops.is_empty());
        assert_eq!(
            (block.steps, block.len, block.last_pc, block.cum.len()),
            (0, 0, 0x12_0000, 0)
        );
    }
}
