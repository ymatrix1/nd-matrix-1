//! Whole-pass AOT lowering (bean `ns4-lle3`, D118): the code a pass executes is cut into basic
//! blocks that end at control flow, and every instruction of a block becomes either a PE-batched
//! native op (`aot_lower.rs`) or an [`ExecItem`]: the interpreter's own execution of that one
//! instruction, built from constants (`CachedInsn::aot`) and inlined, so the decode, the
//! decoded-op `match` and the bit-field extractions fold away at compile time while the semantics
//! stay the interpreter's source code, not a second copy of it.
//!
//! A block ends after a flow instruction (and its two delay slots), a `DO`, an `IDLE` or a stack
//! operation (the dispatcher re-enters by the new pc), or falls into the next block leader.
//! Instructions in the delay slots never join a native batch (the branch lands after the last
//! one). Anything not in the executed set, or in ISA mode, ends a block and runs on the
//! interpreter.
//!
//! The block runner is `aot_hot.rs`'s (`Core::run_aot`): entry bitmap, guard, code-snapshot
//! check, and after an item that can change guarded state the signature check of `aot_glue.rs`.
use super::aot_lower::{AotTier, Item, Lowered, LoweredOp, MIN_BATCH};
use super::aot_mem::MemXfer;
use super::decode_cache::{DecodedOp, FlowOp, MoveOp};
use super::Core;
use crate::visa::{bits, Kind};
use std::collections::{BTreeMap, BTreeSet};

/// Delay slots of a delayed branch (`branch.rs::DELAY_SLOTS`).
const DELAY_SLOTS: u32 = 2;

/// What a block re-checks after an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Chk {
    /// Nothing: the item cannot leave the straight line or change guarded state.
    None,
    /// The pc is the next instruction (inside delay slots, where the pending branch lands).
    Pc,
    /// The pc is the next instruction and the guard signature of `aot_glue.rs` is unchanged
    /// (the item may write the interrupt controller, the mode bits, the code or the stacks).
    Sig,
    /// A store through the data bus: the signature is re-checked only when the access left the
    /// internal SRAM (`Core::aot_slow`), because a store to SRAM cannot change guarded state.
    /// A store into the code of the running block is not an exit: self-modified SRAM code is only
    /// seen at the next block entry (`aot_code_matches`). ASSUMED not to happen inside one block
    /// (the 1:1 diff on ORG Core2 holds); a block that rewrites its own later steps would run the
    /// stale lowered code until it ends.
    Slow,
}

/// How an instruction relates to the state the block guard assumed constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Guard {
    /// Cannot change it.
    Keep,
    /// A store: changes it only when it leaves the internal SRAM.
    Store,
    /// May change it (system register write, bit operation on a system register, ...): the
    /// block ends after the instruction and the dispatcher re-enters (and re-checks the guard).
    Cut,
}

/// Where an instruction sits in a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pos {
    /// Straight line: a [`Guard::Cut`] instruction ends the block.
    Plain,
    /// A delayed flow instruction: the two delay slots follow in the block.
    Flow,
    /// A delay slot.
    Slot,
}

/// One instruction of a whole-pass block that is not a native batch op: a lowered memory transfer
/// (`aot_mem.rs`) or one interpreter step (`Core::step`, out of line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StepItem {
    pub pc: u32,
    pub next: u32,
    pub kind: Kind,
    pub xfer: Option<MemXfer>,
    pub chk: Chk,
    /// The block ends here: the item set the pc (flow, loop setup, last delay slot).
    pub last: bool,
}

/// Whether an executed instruction may change state the block's guard assumed constant.
fn may_change_guard(op: &DecodedOp, kind: Kind, w: u64) -> bool {
    match op {
        DecodedOp::Nop | DecodedOp::ComputeCond { .. } | DecodedOp::ComputeBoth { .. } => false,
        DecodedOp::ModifyImm | DecodedOp::ShiftImm => false,
        DecodedOp::Move(MoveOp::LoadImm { ureg, .. }) => *ureg >= 0x60,
        DecodedOp::Move(MoveOp::RegMove { dst, .. }) => *dst >= 0x60,
        DecodedOp::Move(MoveOp::RegSwap { .. }) => false,
        DecodedOp::Move(MoveOp::DirectMem { write, ureg, .. }) => *write || *ureg >= 0x60,
        DecodedOp::Move(MoveOp::DagMem { write, ureg, .. }) => *write || *ureg >= 0x60,
        DecodedOp::Move(MoveOp::Other) => match kind {
            // loads into a register file below the system registers cannot change the guard
            Kind::T4a | Kind::T4b => bits(w, 39, 39) == 1 || bits(w, 26, 23) >= 0x60,
            Kind::T15a => bits(w, 40, 40) == 1 || bits(w, 38, 32) >= 0x60,
            Kind::T15b => bits(w, 40, 40) == 1 || bits(w, 29, 23) >= 0x60,
            Kind::T7a | Kind::T7b => false,
            _ => true,
        },
        _ => true,
    }
}

/// The [`Guard`] class of an instruction.
fn guard_class(xfer: &Option<MemXfer>, op: &DecodedOp, kind: Kind, w: u64) -> Guard {
    match xfer {
        Some(x) if x.is_store() => Guard::Store,
        Some(x) if x.may_change_guard() => Guard::Cut,
        Some(_) => Guard::Keep,
        None if may_change_guard(op, kind, w) => Guard::Cut,
        None => Guard::Keep,
    }
}

/// Delay slots a block must keep out of native batches after this op, and whether it ends the
/// block (its pc is not the fall-through pc).
fn terminator(op: &DecodedOp) -> Option<u32> {
    match op {
        DecodedOp::Flow(FlowOp::JumpDirect { delayed, .. })
        | DecodedOp::Flow(FlowOp::JumpIndirect { delayed, .. })
        | DecodedOp::Flow(FlowOp::Return { delayed, .. }) => {
            Some(if *delayed { DELAY_SLOTS } else { 0 })
        }
        DecodedOp::Flow(FlowOp::DoUntilCounter { .. })
        | DecodedOp::DoUntil
        | DecodedOp::Type25
        | DecodedOp::StackOp
        | DecodedOp::Idle => Some(0),
        _ => None,
    }
}

impl Core {
    /// Parcels of the instruction at `pc`, its class for block building.
    pub(super) fn whole_decode(&self, pc: u32) -> (u32, DecodedOp, Kind, [u16; 3], u64) {
        let d = self.fetch(pc);
        let op = DecodedOp::from_insn(pc, &d.insn, d.w);
        (d.insn.len as u32, op, d.insn.kind, d.insn.parcels, d.w)
    }

    /// The last-instruction pcs of the counter loops (`DO ... UNTIL LCE`) entered by `executed`.
    /// A block ends there, so the loop-back is the block's own exit (`finish_step` in the tail).
    pub(super) fn loop_ends(&self, executed: &BTreeMap<u32, bool>) -> BTreeSet<u32> {
        executed
            .keys()
            .filter(|&&pc| !self.isa_at(pc))
            .filter_map(|&pc| match self.whole_decode(pc).1 {
                DecodedOp::Flow(FlowOp::DoUntilCounter { end, .. })
                    if executed.contains_key(&end) =>
                {
                    Some(end)
                }
                _ => None,
            })
            .collect()
    }

    /// Block start pcs for the executed pcs `executed` (pc to "reached from outside the previous
    /// instruction") plus `forced`: the leaders, and the pc after every flow instruction with its
    /// delay slots.
    pub(super) fn whole_starts(
        &self,
        executed: &BTreeMap<u32, bool>,
        forced: &[u32],
    ) -> BTreeSet<u32> {
        let mut starts: BTreeSet<u32> = forced
            .iter()
            .copied()
            .filter(|p| executed.contains_key(p))
            .collect();
        starts.extend(executed.iter().filter(|(_, &l)| l).map(|(&p, _)| p));
        for end in self.loop_ends(executed) {
            let after = end + self.whole_decode(end).0;
            if executed.contains_key(&after) {
                starts.insert(after);
            }
        }
        for &pc in executed.keys() {
            if self.isa_at(pc) {
                continue;
            }
            let (len, op, kind, _, w) = self.whole_decode(pc);
            let Some(slots) = terminator(&op).or_else(|| self.cuts_block(pc, &op, kind, w)) else {
                continue;
            };
            let mut after = pc + len;
            for _ in 0..slots {
                after += self.whole_decode(after).0;
            }
            if executed.contains_key(&after) {
                starts.insert(after);
            }
        }
        starts
    }

    /// `Some(0)` when the instruction at `pc` ends its block because it may change guarded state
    /// ([`Guard::Cut`]); the block after it starts at the next pc.
    fn cuts_block(&self, pc: u32, op: &DecodedOp, kind: Kind, w: u64) -> Option<u32> {
        let d = self.fetch(pc);
        let xfer = MemXfer::from_insn(&d.insn, w).or_else(|| MemXfer::from_decoded(op, w));
        (guard_class(&xfer, op, kind, w) == Guard::Cut).then_some(0)
    }

    /// Lowers the block that starts at `start`.
    pub(super) fn lower_whole_block(
        &self,
        start: u32,
        executed: &BTreeMap<u32, bool>,
        starts: &BTreeSet<u32>,
    ) -> Result<Lowered, String> {
        let (mut pc, mut code, mut items, mut steps) = (start, Vec::new(), Vec::new(), 0u32);
        let mut pending: Vec<(u32, u32, LoweredOp)> = Vec::new();
        let (mut slots_left, mut ended) = (0u32, false);
        let mut last_start = start;
        let loop_ends = self.loop_ends(executed);
        let flush = |pending: &mut Vec<(u32, u32, LoweredOp)>, items: &mut Vec<Item>| {
            if pending.len() >= MIN_BATCH {
                items.push(Item::Batch(pending.drain(..).map(|p| p.2).collect()));
            } else {
                for (pc, next, _) in pending.drain(..) {
                    items.push(self.step_item(pc, next, false, Pos::Plain));
                }
            }
        };
        loop {
            if !executed.contains_key(&pc)
                || self.isa_at(pc)
                || (pc != start && starts.contains(&pc))
            {
                break;
            }
            let (len, op, ..) = self.whole_decode(pc);
            let next = pc + len;
            for k in 0..len {
                code.push(self.parcel(pc + k));
            }
            steps += 1;
            last_start = pc;
            if slots_left > 0 {
                flush(&mut pending, &mut items);
                slots_left -= 1;
                let last = slots_left == 0;
                items.push(self.step_item(pc, next, last, Pos::Slot));
                pc = next;
                if last {
                    ended = true;
                    break;
                }
                continue;
            }
            if loop_ends.contains(&pc) && terminator(&op).is_none() {
                flush(&mut pending, &mut items);
                items.push(self.step_item(pc, next, true, Pos::Plain));
                pc = next;
                ended = true;
                break;
            }
            if let Some(slots) = terminator(&op) {
                flush(&mut pending, &mut items);
                if slots == 0 {
                    items.push(self.step_item(pc, next, true, Pos::Plain));
                    pc = next;
                    ended = true;
                    break;
                }
                items.push(self.step_item(pc, next, false, Pos::Flow));
                (slots_left, pc) = (slots, next);
                continue;
            }
            let (_, native, _) = self.lower_insn(pc);
            match native {
                Some(n) => pending.push((pc, next, n)),
                None if matches!(op, DecodedOp::Nop) => pending.push((
                    pc,
                    next,
                    LoweredOp::Compute(super::decode_cache::ComputeOp::Nop),
                )),
                None => {
                    flush(&mut pending, &mut items);
                    let item = self.step_item(pc, next, false, Pos::Plain);
                    let cut = matches!(&item, Item::Step(e) if e.last);
                    items.push(item);
                    if cut {
                        pc = next;
                        ended = true;
                        break;
                    }
                }
            }
            pc = next;
        }
        flush(&mut pending, &mut items);
        if steps == 0 {
            return Err(format!("empty block at 0x{start:x}"));
        }
        let mut at = start;
        let spans = items
            .iter()
            .map(|it| {
                let first = at;
                for _ in 0..match it {
                    Item::Batch(ops) => ops.len(),
                    _ => 1,
                } {
                    at += self.whole_decode(at).0;
                }
                (first, at)
            })
            .collect();
        Ok(Lowered {
            start,
            last: last_start,
            exit: pc,
            steps,
            code,
            items,
            spans,
            dynamic_exit: ended,
        })
    }

    /// The [`StepItem`] of the instruction at `pc` at position `pos` in its block. A
    /// [`Guard::Cut`] instruction in a straight line becomes the last item of the block.
    fn step_item(&self, pc: u32, next: u32, last: bool, pos: Pos) -> Item {
        let (_, op, kind, _, w) = self.whole_decode(pc);
        let d = self.fetch(pc);
        let xfer = MemXfer::from_insn(&d.insn, w).or_else(|| MemXfer::from_decoded(&op, w));
        let class = guard_class(&xfer, &op, kind, w);
        let last = last || (pos == Pos::Plain && class == Guard::Cut);
        let chk = match (pos, class) {
            _ if last => Chk::None,
            // the pending delayed branch is expected: the two delay slots run in the block
            (Pos::Flow, _) => Chk::Pc,
            (Pos::Slot, Guard::Keep) => Chk::Pc,
            (Pos::Slot, _) => Chk::Sig,
            (Pos::Plain, Guard::Store) => Chk::Slow,
            (Pos::Plain, _) => Chk::None,
        };
        Item::Step(StepItem {
            pc,
            next,
            kind,
            xfer,
            chk,
            last,
        })
    }

    /// Lowers every block of the executed set (see [`Core::whole_starts`]).
    pub(super) fn lower_whole_pass(
        &self,
        executed: &BTreeMap<u32, bool>,
        forced: &[u32],
    ) -> Result<Vec<Lowered>, String> {
        let starts = self.whole_starts(executed, forced);
        starts
            .iter()
            .map(|&s| self.lower_whole_block(s, executed, &starts))
            .collect()
    }

    /// Rust source of the whole-pass AOT module for the executed pcs `executed`.
    pub fn emit_whole_pass_rust(
        &self,
        executed: &BTreeMap<u32, bool>,
        forced: &[u32],
        tier: AotTier,
    ) -> Result<String, String> {
        self.emit_whole_pass_rust_ev(executed, forced, tier, false)
    }

    /// [`Core::emit_whole_pass_rust`]; `ev` makes the blocks event-aware (`aot2/events.rs`): a
    /// timer or interrupt event due inside a block no longer refuses it.
    pub fn emit_whole_pass_rust_ev(
        &self,
        executed: &BTreeMap<u32, bool>,
        forced: &[u32],
        tier: AotTier,
        ev: bool,
    ) -> Result<String, String> {
        let blocks = self.lower_whole_pass(executed, forced)?;
        Ok(super::aot_emit::emit_module(&blocks, tier, ev))
    }
}

#[cfg(test)]
mod tests {
    use super::super::support::core_with;
    use super::*;

    /// `R0 = R0 + R1` (fixed, 3 parcels), as in `aot_state.rs`'s mixed program: a native op.
    const ADD: [u16; 3] = [0x013e, 0x0000, 0x1312];

    fn executed(c: &Core, count: usize) -> BTreeMap<u32, bool> {
        // everything from the entry pc, the first instruction a leader
        let mut m = BTreeMap::new();
        let mut pc = c.pc;
        for k in 0..count {
            m.insert(pc, k == 0);
            pc += c.fetch(pc).insn.len as u32;
        }
        m
    }

    #[test]
    fn straight_line_code_is_one_block_with_a_static_exit() {
        let mut p = Vec::new();
        for _ in 0..4 {
            p.extend_from_slice(&ADD);
        }
        let c = core_with(&p);
        let ex = executed(&c, 4);
        let blocks = c.lower_whole_pass(&ex, &[]).unwrap();
        assert_eq!(blocks.len(), 1);
        let b = &blocks[0];
        assert_eq!((b.steps, b.exit, b.dynamic_exit), (4, c.pc + 12, false));
        assert!(
            matches!(&b.items[..], [Item::Batch(ops)] if ops.len() == 4),
            "{:?}",
            b.items
        );
    }

    #[test]
    fn a_delayed_jump_keeps_its_delay_slots_out_of_batches_and_ends_the_block() {
        // 3 ADDs, JUMP (DB) +0x40, then two ADD delay slots, then one more ADD (next block)
        let mut p = Vec::new();
        for _ in 0..3 {
            p.extend_from_slice(&ADD);
        }
        p.extend_from_slice(&[0x083e, 0x043f]); // JUMP (M8, I8) (DB)
        p.extend_from_slice(&ADD);
        p.extend_from_slice(&ADD);
        p.extend_from_slice(&ADD);
        let c = core_with(&p);
        let (_, op, ..) = c.whole_decode(c.pc + 9);
        assert!(terminator(&op).is_some(), "{op:?}");
        let ex = executed(&c, 7);
        let blocks = c.lower_whole_pass(&ex, &[]).unwrap();
        let first = &blocks[0];
        assert!(first.dynamic_exit);
        assert_eq!(first.steps, 3 + 1 + terminator(&op).unwrap());
        let execs: Vec<_> = first
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Step(e) => Some(e),
                _ => None,
            })
            .collect();
        assert_eq!(execs.len() as u32, 1 + terminator(&op).unwrap());
        assert!(execs.last().unwrap().last && !execs[0].last);
        // the pending delayed branch is expected after the flow: only the pc is checked
        assert_eq!(execs[0].chk, Chk::Pc);
        assert_eq!(
            blocks.len(),
            2,
            "the pc after the delay slots starts a block"
        );
    }

    #[test]
    fn a_counter_loop_end_ends_its_block_and_leaves_the_loop_back_to_its_tail() {
        // DO (+3) UNTIL LCE, LCNTR = 4; body: one ADD at the loop end; then one more ADD
        let mut p = vec![0x0C00, 0x0480, 0x0003];
        p.extend_from_slice(&ADD);
        p.extend_from_slice(&ADD);
        let c = core_with(&p);
        let ex = executed(&c, 3);
        assert_eq!(c.loop_ends(&ex).into_iter().collect::<Vec<_>>(), [c.pc + 3]);
        let blocks = c.lower_whole_pass(&ex, &[]).unwrap();
        assert_eq!(blocks.len(), 3, "DO, the body, the code after the loop");
        let body = &blocks[1];
        assert_eq!((body.start, body.last, body.steps), (c.pc + 3, c.pc + 3, 1));
        assert!(
            body.dynamic_exit,
            "the tail sets the pc (loop-back or fall out)"
        );
    }

    #[test]
    fn a_system_register_write_ends_its_block_and_a_store_only_checks_a_slow_access() {
        let store = (0b1001u16 << 12) | (2 << 9) | (2 << 6) | (1 << 4) | 0x20; // DM(I2, M2) = R0
        let mut p = Vec::new();
        p.extend_from_slice(&ADD);
        p.push(store);
        p.extend_from_slice(&[0x0f74, 0x0000, 0x0000]); // MODE1 = 0
        p.extend_from_slice(&ADD);
        let c = core_with(&p);
        let ex = executed(&c, 4);
        let blocks = c.lower_whole_pass(&ex, &[]).unwrap();
        assert_eq!(
            blocks.len(),
            2,
            "the pc after the MODE1 write starts a block"
        );
        let first = &blocks[0];
        assert_eq!((first.steps, first.dynamic_exit), (3, true));
        let steps: Vec<_> = first
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Step(e) => Some(e),
                _ => None,
            })
            .collect();
        assert_eq!(steps.len(), 3);
        assert_eq!(
            steps[1].chk,
            Chk::Slow,
            "store: checked only after a slow access"
        );
        assert!(
            steps[2].last && steps[2].chk == Chk::None,
            "MODE1 write ends the block"
        );
    }
}
