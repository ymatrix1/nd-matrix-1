//! Block discovery: the leaders and the static loop ends of a code range.
//!
//! OWNER: W1-LOWER. Design 4.2: a block starts at a pass entry, `stop_pc`, a static jump or call
//! target, the instruction after a terminator (after its delay slots) and the first instruction of
//! a loop body; dynamic targets (computed jumps, returns) land on a block start or on a uop
//! boundary found through the pc map. A block also starts after a loop end: the last instruction
//! of a statically known loop body is block-final, because the loop back-edge is `finish_step`.
//!
//! The scan is one linear walk of the range by instruction length; it reads the code through
//! `decode_at`, so it sees what `Core::step` executes. It does not follow control flow: data
//! between functions is walked like code and only costs a few useless leaders. Pcs the decode
//! path cannot be trusted for (ISA words, an open vector window) are skipped one short word at a
//! time, like instructions that are not translatable or overhang `hi`.
use super::cut::{flow_of, translatable, Flow};
use super::decode::{decode_at, window_hides};
use crate::core::decode_cache::{CachedInsn, DecodedOp, FlowOp};
use crate::core::{sign_extend, Core};
use crate::visa::bits;

/// What the walk of a range found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::core) struct Scan {
    leaders: Vec<u32>,
    loop_ends: Vec<u32>,
}

impl Scan {
    /// Whether a block must start at `pc` (the previous one ends before it).
    pub fn is_leader(&self, pc: u32) -> bool {
        self.leaders.binary_search(&pc).is_ok()
    }

    /// Whether `pc` is the last instruction of a statically known loop body: a `DO` of the range
    /// names it, or a loop frame that is active at translation time ends there.
    pub fn is_loop_end(&self, pc: u32) -> bool {
        self.loop_ends.binary_search(&pc).is_ok()
    }

    /// The leaders in ascending order.
    pub fn leaders(&self) -> &[u32] {
        &self.leaders
    }

    /// The loop ends in ascending order.
    pub fn loop_ends(&self) -> &[u32] {
        &self.loop_ends
    }
}

/// The static target of a branch instruction, when it has one.
fn static_target(c: &CachedInsn) -> Option<u32> {
    match c.op {
        DecodedOp::Flow(FlowOp::JumpDirect { target, .. }) => Some(target),
        DecodedOp::Flow(FlowOp::JumpIndirect { rel_target, .. }) => rel_target,
        // CJUMP (`branch_ext.rs::exec_type25`); RFRAME has no target
        DecodedOp::Type25 if matches!(flow_of(c), Flow::End { .. }) => {
            let raw = bits(c.w, 23, 0) as u32;
            Some(if bits(c.w, 38, 38) == 1 {
                c.pc.wrapping_add(sign_extend(raw, 24))
            } else {
                raw
            })
        }
        _ => None,
    }
}

/// The last instruction of the loop body a `DO` sets up (`FlowOp::DoUntilCounter::end`,
/// `arith_loop.rs::exec_do_until` for the `UNTIL cond` form).
fn loop_end_of(c: &CachedInsn) -> Option<u32> {
    match c.op {
        DecodedOp::Flow(FlowOp::DoUntilCounter { end, .. }) => Some(end),
        DecodedOp::DoUntil => Some(c.pc.wrapping_add(sign_extend(bits(c.w, 22, 0) as u32, 23))),
        _ => None,
    }
}

/// Whether the decode path may be used for the instruction `c` read at `pc` inside `[lo, hi)`.
fn usable(core: &Core, c: &CachedInsn, hi: u32) -> bool {
    c.next_pc > c.pc && c.next_pc <= hi && translatable(c) && !core.isa_at(c.pc)
}

/// The pc after the delay slots of the branch `c` (stops at `hi`).
fn after_slots(core: &mut Core, c: &CachedInsn, slots: u8, hi: u32) -> u32 {
    let mut pc = c.next_pc;
    for _ in 0..slots {
        if pc >= hi || window_hides(core, pc) || core.isa_at(pc) {
            break;
        }
        pc = decode_at(core, pc).next_pc;
    }
    pc
}

/// Walks `lo..hi` once. `stop_pc` is the pass idle pc, a leader when inside the range.
pub(in crate::core) fn scan(core: &mut Core, stop_pc: Option<u32>, lo: u32, hi: u32) -> Scan {
    let in_range = |p: u32| (lo..hi).contains(&p);
    let mut leaders = vec![lo];
    let mut loop_ends = Vec::new();
    leaders.extend(stop_pc.filter(|&p| in_range(p)));
    for f in &core.loops {
        loop_ends.push(f.end);
        leaders.extend(Some(f.start).filter(|&p| in_range(p)));
    }
    let mut pc = lo;
    while pc < hi {
        if window_hides(core, pc) || core.isa_at(pc) {
            pc += 1;
            continue;
        }
        let c = decode_at(core, pc);
        if !usable(core, &c, hi) {
            pc = c.next_pc.max(pc + 1);
            continue;
        }
        if let Some(t) = static_target(&c).filter(|&t| in_range(t)) {
            leaders.push(t);
        }
        loop_ends.extend(loop_end_of(&c));
        if let Flow::End { slots } = flow_of(&c) {
            leaders.push(after_slots(core, &c, slots, hi));
        }
        pc = c.next_pc;
    }
    // the instruction after the last instruction of a loop body starts a block
    for &end in &loop_ends {
        if in_range(end) && !window_hides(core, end) && !core.isa_at(end) {
            leaders.push(decode_at(core, end).next_pc);
        }
    }
    leaders.retain(|&p| in_range(p));
    leaders.sort_unstable();
    leaders.dedup();
    loop_ends.sort_unstable();
    loop_ends.dedup();
    Scan { leaders, loop_ends }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::{core_with, CODE};

    fn scan_all(program: &[u16]) -> (Scan, u32) {
        let mut c = core_with(program);
        let hi = CODE + program.len() as u32;
        (scan(&mut c, None, CODE, hi), hi)
    }

    #[test]
    fn straight_code_has_one_leader_the_start() {
        // NOP, R3 = R1 + R2, NOP
        let (s, _) = scan_all(&[0x0001, 0x013e, 0x0000, 0x1312, 0x0001]);
        assert_eq!(s.leaders(), [CODE]);
        assert!(s.loop_ends().is_empty());
    }

    #[test]
    fn the_instruction_after_a_terminator_and_its_slots_is_a_leader() {
        // 0: NOP  1: jump db (3 parcels, to 0x12_0009)  4: NOP (slot)  5: NOP (slot)  6: NOP
        // 7: rts (1 parcel)  8: NOP  9: NOP
        let mut p = vec![0x0001, 0x063E, 0x0412, 0x0009, 0x0001, 0x0001, 0x0001];
        p.extend([0x0ABE, 0x0001, 0x0001]);
        let (s, _) = scan_all(&p);
        // jump target CODE + 9 is the static target; the group ends at CODE + 6; rts ends at 8
        assert_eq!(s.leaders(), [CODE, CODE + 6, CODE + 8, CODE + 9]);
    }

    #[test]
    fn a_static_target_outside_the_range_is_not_a_leader_of_it() {
        let (s, _) = scan_all(&[0x063E, 0x0000, 0x0010, 0x0001]);
        assert_eq!(s.leaders(), [CODE, CODE + 3], "0x10 is outside the range");
    }

    #[test]
    fn a_do_loop_names_its_end_and_the_pc_after_it() {
        // the DO loop of `branch.rs`: R0 = 0; I0 = 0xc0430; DO (+3) UNTIL LCE, LCNTR = 4;
        // DM(I0, M1) = R0; NOP
        let p = [
            0x0F80, 0x0000, 0x0F10, 0x000C, 0x0430, 0x0C00, 0x0480, 0x0003, 0x507E, 0x8000, 0x0000,
            0x0001,
        ];
        let (s, _) = scan_all(&p);
        // DO at CODE + 5 (3 parcels), body at CODE + 8 (3 parcels), end = DO pc + 3 = CODE + 8
        assert_eq!(s.loop_ends(), [CODE + 8]);
        assert_eq!(
            s.leaders(),
            [CODE, CODE + 8, CODE + 11],
            "start, body, after the loop"
        );
        assert!(s.is_loop_end(CODE + 8) && !s.is_loop_end(CODE + 9));
        assert!(s.is_leader(CODE + 11) && !s.is_leader(CODE + 5));
    }

    #[test]
    fn stop_pc_and_active_loop_frames_are_taken_from_the_core() {
        let mut c = core_with(&[0x0001; 8]);
        c.loops.push(crate::core::types::LoopFrame {
            start: CODE + 2,
            end: CODE + 5,
            ..Default::default()
        });
        let s = scan(&mut c, Some(CODE + 7), CODE, CODE + 8);
        assert_eq!(s.loop_ends(), [CODE + 5]);
        assert_eq!(s.leaders(), [CODE, CODE + 2, CODE + 6, CODE + 7]);
        let none = scan(&mut core_with(&[0x0001; 2]), Some(CODE + 9), CODE, CODE + 2);
        assert_eq!(
            none.leaders(),
            [CODE],
            "a stop_pc outside the range is ignored"
        );
    }

    #[test]
    fn an_open_vector_window_and_an_overhanging_instruction_are_skipped() {
        // NOP, then a 3-parcel ALU op cut off by hi
        let mut c = core_with(&[0x0001, 0x013e, 0x0000, 0x1312]);
        assert_eq!(scan(&mut c, None, CODE, CODE + 3).leaders(), [CODE]);
        let mut w = core_with(&[0x0001, 0x063E, 0x0000, 0x0000, 0x0001]);
        (w.irq.window_lo, w.irq.window_len) = (CODE + 1, 3);
        let s = scan(&mut w, None, CODE, CODE + 5);
        assert_eq!(
            s.leaders(),
            [CODE],
            "the jump inside the window was not decoded"
        );
    }
}
