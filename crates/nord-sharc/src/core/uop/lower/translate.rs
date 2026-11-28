//! The translator: instructions of a code range to blocks, one PEy mode at a time.
//!
//! OWNER: W1-LOWER. The range is walked once, linearly, and cut into non-overlapping blocks (the
//! rules are in `cut.rs`, the leaders in `scan.rs`); every instruction is lowered by the lowering
//! function (`lower_insn`: the families, else an `Interp` uop) and the translator then applies
//! what only it knows (design 10, rules P1..P6, P11):
//!
//! * a delayed branch and its two delay slots form one group inside one block; a group that does
//!   not fit (a slot is a branch, a system-register load, outside the range, at `stop_pc`, ...)
//!   is not translated at all, the interpreter runs the branch;
//! * the uops of a delay slot and of a static loop end get `UF_FIN`, except `Interp` uops, which
//!   run `finish_step` themselves and must not run it twice;
//! * a block that holds a fast uop for an instruction that can raise an arithmetic exception is
//!   `Arith`, one with a delay group `Delayed`, with a loop end `Loop`;
//! * an instruction whose uops do not account exactly one step is lowered as `Interp` instead
//!   (the static budget must equal the interpreter's steps);
//! * a block ends after an instruction whose uop is followed by the circular-buffer latch
//!   (`UF_CB`), or that instruction is lowered `Interp` when delay slots are still owed
//!   (`cbwrap.rs`);
//! * `Block::code_hash` is stored here, over the parcels `pc..end_pc` (rule P6).
use super::builder::BlockBuilder;
use super::cbwrap::latches_cb;
use super::cut::{can_raise_arith, cuts_after, flow_of, Flow};
use super::decode::{code_hash, decode_at};
use super::group::{fits, group_fits, readable};
use super::scan::{scan, Scan};
use super::LowerCtx;
use crate::core::decode_cache::CachedInsn;
use crate::core::uop::config::UopConfig;
use crate::core::uop::types::{Block, Uop, UopKind, NONE};
use crate::core::Core;

/// Translates `lo..hi` for both PEy modes (rule P4: blocks are never shared between them; the
/// PEy-off blocks come first). `lower` lowers one instruction; production passes `lower_insn`.
pub(in crate::core) fn translate_with<L>(
    core: &mut Core,
    cfg: &UopConfig,
    lo: u32,
    hi: u32,
    lower: L,
) -> (Vec<Block>, Vec<Uop>)
where
    L: Fn(&LowerCtx<'_>, &CachedInsn, &mut BlockBuilder),
{
    let stop_pc = Some(core.uop_run.stop_pc).filter(|&p| p != NONE);
    let sc = scan(core, stop_pc, lo, hi);
    let (mut blocks, mut uops) = (Vec::new(), Vec::new());
    for simd in [false, true] {
        let cx = LowerCtx {
            cfg,
            stop_pc,
            simd,
            isa: false,
        };
        let mut pc = lo;
        while pc < hi {
            let (built, next) = build_block(core, &cx, &sc, pc, hi, &lower);
            if let Some((mut block, bu)) = built.filter(|(b, _)| b.steps >= cfg.min_block_steps) {
                block.first = uops.len() as u32;
                uops.extend(bu);
                blocks.push(block);
            }
            pc = next.max(pc + 1);
        }
    }
    (blocks, uops)
}

/// Builds the block starting at `start` and returns it (none when the instruction there is left
/// to the interpreter) with the pc where the walk goes on.
fn build_block<L>(
    core: &mut Core,
    cx: &LowerCtx<'_>,
    sc: &Scan,
    start: u32,
    hi: u32,
    lower: &L,
) -> (Option<(Block, Vec<Uop>)>, u32)
where
    L: Fn(&LowerCtx<'_>, &CachedInsn, &mut BlockBuilder),
{
    let mut b = BlockBuilder::new(start, cx.simd, false);
    let mut pc = start;
    while readable(core, pc) {
        let c = decode_at(core, pc);
        let slots = match flow_of(&c) {
            Flow::End { slots } if b.owed_slots() == 0 => slots,
            _ => 0,
        };
        if !fits(core, &c, hi) || (slots > 0 && !group_fits(core, cx, &c, slots, hi)) {
            // left to the interpreter: the block ends before it; at the start nothing is built
            return if b.is_empty() {
                (None, c.next_pc.max(pc + 1))
            } else {
                (Some(finish(core, b)), pc)
            };
        }
        lower_one(cx, sc, &c, &mut b, lower);
        pc = c.next_pc;
        let free = b.owed_slots() == 0;
        let ends = b.ends() || cuts_after(cx, &c, b.steps()) || sc.is_loop_end(c.pc);
        if free && (ends || sc.is_leader(pc) || pc >= hi) {
            break;
        }
    }
    if b.is_empty() {
        return (None, pc.max(start + 1));
    }
    (Some(finish(core, b)), pc)
}

/// Lowers `c` into `b` and applies the translator's part of the protocol.
fn lower_one<L>(cx: &LowerCtx<'_>, sc: &Scan, c: &CachedInsn, b: &mut BlockBuilder, lower: &L)
where
    L: Fn(&LowerCtx<'_>, &CachedInsn, &mut BlockBuilder),
{
    let in_slot = b.owed_slots() > 0;
    let (first, cp) = (b.len(), b.checkpoint());
    lower(cx, c, b);
    b.set_end(c.next_pc);
    if b.steps() - cp.steps() != 1 {
        // a family that does not account exactly one step is wrong; the interpreter is not
        b.rollback(cp);
        b.interp(c);
    }
    if latches_cb(&b.uops()[first..]) {
        // the wrap latch pokes `irq.due`: nothing may run after it in the block (`cbwrap.rs`)
        let slots_follow = match flow_of(c) {
            Flow::End { slots } if !in_slot => slots > 0,
            _ => in_slot && b.owed_slots() > 1,
        };
        if slots_follow {
            b.rollback(cp);
            b.interp(c);
        } else {
            b.end_block();
        }
    }
    if can_raise_arith(c) && b.uops()[first..].iter().any(|u| u.h != UopKind::INTERP.0) {
        b.note_arith();
    }
    let loop_end = sc.is_loop_end(c.pc);
    match flow_of(c) {
        // a branch: its handler runs finish_step itself (flow family contract, like `Interp`),
        // so it never gets `UF_FIN`, also when it is the last instruction of a static loop body
        // (`loop_end` below): this arm wins over the `loop_end` arm, a second `finish_step` after
        // the handler would take the loop back-edge twice (`lower/flow/chain.rs`,
        // `exec/h_flow/diff_chain.rs`)
        Flow::End { slots } if !in_slot => {
            if slots > 0 && b.owed_slots() == 0 {
                b.owe_slots(slots);
            }
            b.end_block();
            b.note_terminator();
        }
        _ if in_slot => {
            b.mark_fin(first);
            b.slot_done();
        }
        _ if loop_end => {
            b.mark_fin(first);
        }
        _ => {}
    }
    if loop_end {
        b.note_loop_end();
    }
}

/// Ends `b`: guard class, code hash.
fn finish(core: &Core, b: BlockBuilder) -> (Block, Vec<Uop>) {
    let guard = b.guard_class();
    let (mut block, uops) = b.finish(guard);
    block.code_hash = code_hash(core, block.pc, block.end_pc);
    (block, uops)
}

#[cfg(test)]
mod tests {
    use super::super::builder::FP_LAST_SEQ;
    use super::super::lower_insn;
    use super::super::support::{
        build, build_cfg, build_default, fast_lower, spans, unimplemented_insn, Built, ALU,
        JUMP_DB, LOAD_SYS, NOP,
    };
    use super::*;
    use crate::core::support::{core_with, CODE};
    use crate::core::uop::types::{GuardClass, UF_FIN, UF_LAST};

    #[test]
    fn straight_code_is_one_block_per_mode_with_its_hash_and_budget() {
        let program = [&[NOP][..], &ALU, &[NOP, NOP]].concat();
        let (core, built) = build_default(&program);
        assert_eq!(built.blocks.len(), 2);
        for (k, simd) in [false, true].into_iter().enumerate() {
            let [(b, uops)] = built.mode(simd)[..] else {
                panic!("one block per mode")
            };
            assert_eq!((b.pc, b.last_pc, b.end_pc), (CODE, CODE + 5, CODE + 6));
            let shape = (b.steps, b.len, b.first, b.simd, b.isa);
            assert_eq!(shape, (4, 4, 4 * k as u32, simd, false));
            assert_eq!(&*b.cum, &[0, 1, 2, 3]);
            assert_eq!(b.guard, GuardClass::Plain);
            assert_eq!(b.code_hash, code_hash(&core, CODE, CODE + 6));
            assert!(uops.iter().all(|u| u.h == UopKind::INTERP.0));
            let at: Vec<u32> = uops.iter().map(|u| u.pc).collect();
            assert_eq!(at, [CODE, CODE + 1, CODE + 4, CODE + 5]);
            assert_eq!(
                b.footprint.writes & FP_LAST_SEQ,
                FP_LAST_SEQ,
                "Interp finishes"
            );
        }
    }

    #[test]
    fn a_delayed_branch_and_its_two_slots_are_one_block_with_fin_on_the_slots() {
        let program = [&[NOP][..], &JUMP_DB, &[NOP; 4]].concat();
        let (_, all_interp) = build_default(&program);
        assert_eq!(spans(&all_interp, false), [(CODE, 4), (CODE + 6, 2)]);
        let (b1, interp_uops) = all_interp.at(CODE, false).unwrap();
        assert_eq!(b1.guard, GuardClass::Delayed);
        assert_eq!((b1.last_pc, b1.end_pc), (CODE + 5, CODE + 6));
        assert!(
            interp_uops.iter().all(|u| u.flags & UF_FIN == 0),
            "Interp has no UF_FIN"
        );
        let (_, fast) = build(&program, &UopConfig::default(), None, fast_lower);
        let (b, uops) = fast.at(CODE, false).unwrap();
        let fin: Vec<u8> = uops.iter().map(|u| u.flags & (UF_FIN | UF_LAST)).collect();
        assert_eq!(
            fin,
            [0, 0, UF_FIN, UF_FIN | UF_LAST],
            "plain, branch, slot, slot"
        );
        assert_eq!(b.footprint.writes & FP_LAST_SEQ, FP_LAST_SEQ);
        assert_eq!(b.guard, GuardClass::Delayed);
    }

    #[test]
    fn stop_pc_starts_a_block_and_is_never_interior() {
        let cfg = UopConfig::default();
        assert_eq!(spans(&build_cfg(&[NOP; 6], &cfg, None), false), [(CODE, 6)]);
        let built = build_cfg(&[NOP; 6], &cfg, Some(CODE + 3));
        assert_eq!(spans(&built, false), [(CODE, 3), (CODE + 3, 3)]);
        assert_eq!(spans(&built, true), spans(&built, false));
        let at_start = build_cfg(&[NOP; 6], &cfg, Some(CODE));
        assert_eq!(
            spans(&at_start, false),
            [(CODE, 6)],
            "a block may start at it"
        );
        let outside = build_cfg(&[NOP; 6], &cfg, Some(CODE + 9));
        assert_eq!(spans(&outside, false), [(CODE, 6)]);
    }

    #[test]
    fn the_last_instruction_of_a_static_loop_is_block_final_with_a_fin_uop() {
        // R0 = 0; I0 = 0xc0430; DO (+3) UNTIL LCE, LCNTR = 4; DM(I0, M1) = R0; NOP
        let p = [
            0x0F80, 0x0000, 0x0F10, 0x000C, 0x0430, 0x0C00, 0x0480, 0x0003, 0x507E, 0x8000, 0x0000,
            NOP,
        ];
        let (_, fast) = build(&p, &UopConfig::default(), None, fast_lower);
        assert_eq!(
            spans(&fast, false),
            [(CODE, 3), (CODE + 8, 1), (CODE + 11, 1)]
        );
        let (b, uops) = fast.at(CODE + 8, false).unwrap();
        assert_eq!(
            (b.guard, uops[0].flags & UF_FIN),
            (GuardClass::Loop, UF_FIN)
        );
        let (_, do_block) = fast.at(CODE, false).unwrap();
        assert!(
            do_block.iter().all(|u| u.flags & UF_FIN == 0),
            "the DO is a terminator"
        );
        let (_, interp) = build_default(&p);
        let (b, uops) = interp.at(CODE + 8, false).unwrap();
        assert_eq!((b.guard, uops[0].flags & UF_FIN), (GuardClass::Loop, 0));
        assert_eq!(b.footprint.writes & FP_LAST_SEQ, FP_LAST_SEQ);
    }

    #[test]
    fn the_step_limits_split_and_drop_blocks_and_a_system_load_ends_one() {
        let long = UopConfig {
            max_block_steps: 4,
            ..UopConfig::default()
        };
        let split = build_cfg(&[NOP; 10], &long, None);
        assert_eq!(
            spans(&split, false),
            [(CODE, 4), (CODE + 4, 4), (CODE + 8, 2)]
        );
        let drop_short = UopConfig {
            min_block_steps: 3,
            ..long
        };
        let dropped = build_cfg(&[NOP; 10], &drop_short, None);
        assert_eq!(spans(&dropped, false), [(CODE, 4), (CODE + 4, 4)]);
        let sys = [&[NOP][..], &LOAD_SYS, &[NOP; 2]].concat();
        let cut = build_cfg(&sys, &UopConfig::default(), None);
        assert_eq!(spans(&cut, false), [(CODE, 2), (CODE + 3, 2)]);
    }

    #[test]
    fn unimplemented_instructions_and_an_open_vector_window_are_not_translated() {
        let bad = unimplemented_insn().expect("some parcel is not an instruction");
        let program = [&[NOP][..], &bad, &[NOP]].concat();
        let built = build_cfg(&program, &UopConfig::default(), None);
        let starts: Vec<u32> = built.mode(false).iter().map(|(b, _)| b.pc).collect();
        let after = CODE + 1 + bad.len() as u32;
        assert_eq!(
            starts,
            [CODE, after],
            "the barrier instruction has no block"
        );
        let mut core = core_with(&[NOP; 5]);
        (core.irq.window_lo, core.irq.window_len) = (CODE + 1, 2);
        let cfg = UopConfig::default();
        let (blocks, _) = translate_with(&mut core, &cfg, CODE, CODE + 5, lower_insn);
        let off: Vec<(u32, u32)> = blocks
            .iter()
            .filter(|b| !b.simd)
            .map(|b| (b.pc, b.steps))
            .collect();
        assert_eq!(off, [(CODE, 1), (CODE + 3, 2)]);
    }

    #[test]
    fn a_lowering_that_does_not_account_one_step_is_replaced_by_an_interp_uop() {
        fn none(_: &LowerCtx<'_>, _: &CachedInsn, _: &mut BlockBuilder) {}
        fn two(cx: &LowerCtx<'_>, c: &CachedInsn, out: &mut BlockBuilder) {
            fast_lower(cx, c, out);
            fast_lower(cx, c, out);
            out.end_block();
            out.owe_slots(2);
        }
        for lower in [none, two] {
            let (_, built) = build(&[NOP; 3], &UopConfig::default(), None, lower);
            let [(b, uops)] = built.mode(false)[..] else {
                panic!("one block")
            };
            assert_eq!((b.steps, b.guard), (3, GuardClass::Plain), "{uops:?}");
            assert!(uops.iter().all(|u| u.h == UopKind::INTERP.0));
        }
    }

    #[test]
    fn a_fast_uop_for_a_compute_that_can_raise_an_exception_makes_the_block_arith() {
        let alu = [&[NOP][..], &ALU].concat();
        let class = |built: &Built| built.at(CODE, false).unwrap().0.guard;
        let cfg = UopConfig::default();
        assert_eq!(
            class(&build(&alu, &cfg, None, fast_lower).1),
            GuardClass::Arith
        );
        assert_eq!(
            class(&build(&[NOP; 2], &cfg, None, fast_lower).1),
            GuardClass::Plain
        );
        assert_eq!(
            class(&build_default(&alu).1),
            GuardClass::Plain,
            "Interp raises its own"
        );
    }
}
