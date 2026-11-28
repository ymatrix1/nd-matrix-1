//! Block assembly: collects the uops of one basic block and derives its budget, cumulative step
//! table and footprint.
//!
//! OWNER: W1-LOWER. An all-`Interp` block is exact by construction (the footprint of an `Interp`
//! uop is the conservative "everything"). The translator (`translate.rs`) drives the protocol
//! hooks below after every instruction and asks [`BlockBuilder::guard_class`] for the guard the
//! block needs.
//!
//! Footprint layout. ASSUMED conservative: a block with any uop other than `NOP` reads and writes
//! "everything" (all bits, memory and flags), because no consumer (exit sweep, register
//! promotion) exists yet to say which registers a fast uop touches; narrowing it later cannot make
//! a block less exact. Bit [`FP_LAST_SEQ`] is the one exception, it is not a register: it is set
//! in `writes` exactly when the LAST uop of the block runs `finish_step` itself (an `Interp` uop,
//! a uop with `UF_FIN`, or a flow terminator), which is what the entry guard needs to know to
//! let a block start while a loop frame ends at its last instruction (`guard::check`).
use super::super::types::{
    Block, Footprint, GuardClass, Link, Uop, UopKind, NONE, UF_FIN, UF_LAST,
};
use super::{budget, guard};
use crate::core::decode_cache::CachedInsn;

/// Bit of `Footprint::writes`: the last uop of the block calls `finish_step` itself.
pub(in crate::core) const FP_LAST_SEQ: u64 = 1 << 63;

/// A point of a [`BlockBuilder`] to go back to ([`BlockBuilder::checkpoint`]).
#[derive(Clone, Copy)]
pub(in crate::core) struct Checkpoint {
    len: usize,
    steps: u32,
    owed: u8,
    ended: bool,
    delayed: bool,
    end_pc: u32,
    last_seq: bool,
    footprint: Footprint,
}

impl Checkpoint {
    /// Steps accounted before the checkpoint.
    pub fn steps(&self) -> u32 {
        self.steps
    }
}

/// Reads and writes everything.
const EVERYTHING: Footprint = Footprint {
    reads: u64::MAX,
    writes: u64::MAX,
    touches_mem: true,
    touches_flags: true,
};

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
    /// A delayed branch was lowered (the block holds a delay group).
    delayed: bool,
    /// An instruction with a fast uop can raise an arithmetic exception.
    arith: bool,
    /// The block holds the end of a statically known loop body.
    loop_end: bool,
    /// The last uop pushed runs `finish_step` itself.
    last_seq: bool,
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
            delayed: false,
            arith: false,
            loop_end: false,
            last_seq: false,
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
        self.delayed |= n > 0;
    }

    /// Delay slots still owed after the last lowered instruction.
    pub fn owed_slots(&self) -> u8 {
        self.owed
    }

    /// Called by the translator after it lowered one more instruction: a slot is no longer owed.
    pub fn slot_done(&mut self) {
        self.owed = self.owed.saturating_sub(1);
    }

    /// Appends a uop. Any uop but `NOP` widens the footprint to everything.
    pub fn push(&mut self, u: Uop) {
        if u.h != UopKind::NOP.0 {
            self.footprint = EVERYTHING;
        }
        self.last_seq = u.flags & UF_FIN != 0;
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
        // `exec_cached` finishes the step itself: the uop must not get `UF_FIN` as well
        self.last_seq = true;
    }

    /// Uops collected so far.
    pub fn len(&self) -> usize {
        self.uops.len()
    }

    /// The uops collected so far.
    pub fn uops(&self) -> &[Uop] {
        &self.uops
    }

    /// Steps accounted by the uops collected so far.
    pub fn steps(&self) -> u32 {
        self.uops.iter().map(|u| u32::from(u.steps)).sum()
    }

    /// The state to return to with [`BlockBuilder::rollback`].
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            len: self.uops.len(),
            steps: self.steps(),
            owed: self.owed,
            ended: self.ended,
            delayed: self.delayed,
            end_pc: self.end_pc,
            last_seq: self.last_seq,
            footprint: self.footprint,
        }
    }

    /// Drops everything a rejected lowering of one instruction added (the uops and its hook
    /// calls), so it leaves no trace. `arith` and `loop_end` are noted by the translator after
    /// the lowering and are not touched.
    pub fn rollback(&mut self, cp: Checkpoint) {
        self.uops.truncate(cp.len);
        self.owed = cp.owed;
        self.ended = cp.ended;
        self.delayed = cp.delayed;
        self.end_pc = cp.end_pc;
        self.last_seq = cp.last_seq;
        self.footprint = cp.footprint;
    }

    /// Translator hook: the instruction whose uops start at index `first` needs `finish_step`
    /// after it (a delay slot, or the last instruction of a loop body). Sets `UF_FIN` on its last
    /// uop unless that is an `Interp` uop, which finishes the step itself and must not run it
    /// twice. Returns whether a flag was set.
    pub fn mark_fin(&mut self, first: usize) -> bool {
        let Some(last) = self.uops.get_mut(first..).and_then(<[Uop]>::last_mut) else {
            return false;
        };
        if last.h == UopKind::INTERP.0 {
            return false;
        }
        last.flags |= UF_FIN;
        self.last_seq = true;
        true
    }

    /// Translator hook: the last uop is the terminator of a flow instruction, whose handler runs
    /// `finish_step` itself (the flow family contract, like `Interp`).
    pub fn note_terminator(&mut self) {
        self.last_seq = true;
    }

    /// Translator hook: an instruction with a fast uop can raise an arithmetic exception.
    pub fn note_arith(&mut self) {
        self.arith = true;
    }

    /// Translator hook: the instruction just lowered is the end of a static loop body.
    pub fn note_loop_end(&mut self) {
        self.loop_end = true;
    }

    /// The guard the block needs (`guard::classify`).
    pub fn guard_class(&self) -> GuardClass {
        guard::classify(&self.uops, self.delayed, self.arith, self.loop_end)
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
        self.footprint.writes = if self.last_seq {
            self.footprint.writes | FP_LAST_SEQ
        } else {
            self.footprint.writes & !FP_LAST_SEQ
        };
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

    fn fast(kind: u16, pc: u32) -> Uop {
        Uop::new(UopKind(kind), 1, pc, 0, 0)
    }

    #[test]
    fn mark_fin_flags_the_last_fast_uop_of_the_instruction_but_never_an_interp_uop() {
        let mut c = core_with(&[0x0001, 0x0001]);
        let pc = c.pc;
        let mut b = BlockBuilder::new(pc, false, false);
        b.push(fast(0x301, pc));
        b.push(Uop::new(UopKind(0x302), 0, pc, 0, 0)); // second uop of the same instruction
        let (first, interp) = (b.len(), decode_at(&mut c, pc + 1));
        assert!(b.mark_fin(0));
        let fin: Vec<_> = b.uops().iter().map(|u| u.flags & UF_FIN).collect();
        assert_eq!(fin, [0, UF_FIN], "only the last uop of the instruction");
        b.interp(&interp);
        assert!(!b.mark_fin(first), "an Interp uop runs finish_step itself");
        assert_eq!(b.uops()[first].flags & UF_FIN, 0);
        assert!(!b.mark_fin(9), "no instruction there");
        assert_eq!(
            b.steps(),
            2,
            "steps count uops of one step, the second uop has none"
        );
    }

    #[test]
    fn the_footprint_widens_with_any_uop_but_nop_and_the_last_seq_bit_follows_the_last_uop() {
        let last_seq = |b: BlockBuilder| b.finish(GuardClass::Plain).0.footprint;
        let mut b = BlockBuilder::new(0x12_0000, false, false);
        b.push(Uop::new(UopKind::NOP, 1, 0x12_0000, 0, 0));
        assert_eq!(last_seq(b), Footprint::default(), "a NOP touches nothing");
        let mut b = BlockBuilder::new(0x12_0000, false, false);
        b.push(fast(0x301, 0x12_0000));
        let f = last_seq(b);
        assert!(f.touches_mem && f.reads == u64::MAX && f.writes & FP_LAST_SEQ == 0);
        let mut b = BlockBuilder::new(0x12_0000, false, false);
        b.push(fast(0x301, 0x12_0000));
        b.mark_fin(0);
        assert_ne!(last_seq(b).writes & FP_LAST_SEQ, 0);
        // an Interp uop in the middle widens to everything but the last uop decides the bit
        let mut c = core_with(&[0x0001]);
        let pc = c.pc;
        let mut b = BlockBuilder::new(pc, false, false);
        b.interp(&decode_at(&mut c, pc));
        b.push(fast(0x301, pc + 1));
        assert_eq!(last_seq(b).writes & FP_LAST_SEQ, 0);
        let mut b = BlockBuilder::new(pc, false, false);
        b.push(fast(0x301, pc));
        b.note_terminator();
        assert_ne!(
            last_seq(b).writes & FP_LAST_SEQ,
            0,
            "a flow terminator sequences"
        );
    }

    #[test]
    fn a_rollback_forgets_the_uops_and_the_hook_calls_of_a_rejected_lowering() {
        let mut b = BlockBuilder::new(0x12_0000, false, false);
        b.push(fast(0x301, 0x12_0000));
        b.set_end(0x12_0001);
        let cp = b.checkpoint();
        assert_eq!(cp.steps(), 1);
        b.push(fast(0x302, 0x12_0001));
        b.owe_slots(2);
        b.end_block();
        b.set_end(0x12_0002);
        b.rollback(cp);
        assert_eq!(
            (b.len(), b.steps(), b.owed_slots(), b.ends()),
            (1, 1, 0, false)
        );
        let (block, _) = b.finish(GuardClass::Plain);
        assert_eq!(block.end_pc, 0x12_0001);
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
