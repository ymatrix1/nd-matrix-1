//! Test fixtures shared by the tests of the lowering: programs, a stand-in family, the translated
//! result of a program, the guard-relevant state of a core and an instruction-level lockstep
//! against `Core::step`.
//!
//! OWNER: W1-LOWER. Only compiled for tests. `lockstep` is NOT the uop engine: it executes every
//! uop with `steps > 0` as `exec_cached` of its instruction, which is the semantics of an `Interp`
//! uop, next to the interpreter, so it proves the structure of the blocks (the instruction chain,
//! the step budget, where the pc may change) and not any fast handler (the handlers are checked
//! by the differential tests of `exec/h_*` and `verify/`).
use super::builder::BlockBuilder;
use super::cut::{flow_of, Flow};
use super::decode::decode_at;
use super::translate::translate_with;
use super::{lower_insn, LowerCtx};
use crate::core::decode_cache::CachedInsn;
use crate::core::support::{core_with, CODE};
use crate::core::uop::config::UopConfig;
use crate::core::uop::types::{Block, Uop, UopKind, NONE, UF_CB};
use crate::core::Core;

/// (name, parcels, flow class) of one instruction per kind of terminator, with and without delay
/// slots. Encodings are the ones of the `branch.rs`, `branch_ext.rs` and `arith_loop.rs` tests.
pub(super) const FLOW_CASES: &[(&str, &[u16], Flow)] = &[
    ("nop", &[0x0001], Flow::Plain),
    ("alu", &[0x013e, 0x0000, 0x1312], Flow::Plain),
    ("load 17b", &[0x0F81, 0x0007], Flow::Plain),
    ("jump", &[0x063E, 0x0000, 0x0010], Flow::End { slots: 0 }),
    ("jump db", &[0x063E, 0x0400, 0x0010], Flow::End { slots: 2 }),
    (
        "jump rel",
        &[0x073E, 0x0000, 0x0004],
        Flow::End { slots: 0 },
    ),
    (
        "jump rel db",
        &[0x073E, 0x0400, 0x0004],
        Flow::End { slots: 2 },
    ),
    (
        "if eq jump",
        &[0x0600, 0x0000, 0x0010],
        Flow::End { slots: 0 },
    ),
    ("call", &[0x06BE, 0x0012, 0x0008], Flow::End { slots: 0 }),
    ("call db", &[0x06BE, 0x0400, 0x0010], Flow::End { slots: 2 }),
    ("jump (m,i)", &[0x083E, 0x003F], Flow::End { slots: 0 }),
    ("jump (m,i) db", &[0x083E, 0x043F], Flow::End { slots: 2 }),
    ("call (m,i)", &[0x08BE, 0x003F], Flow::End { slots: 0 }),
    ("rts", &[0x0ABE], Flow::End { slots: 0 }),
    ("rts db", &[0x0AFE], Flow::End { slots: 2 }),
    (
        "rts wide db",
        &[0x0A3E, 0x0400, 0x0000],
        Flow::End { slots: 2 },
    ),
    (
        "do lce imm",
        &[0x0C00, 0x0500, 0x0006],
        Flow::End { slots: 0 },
    ),
    (
        "do lce reg",
        &[0x0D00, 0x0000, 0x0003],
        Flow::End { slots: 0 },
    ),
    (
        "do until",
        &[0x0E00, 0x0000, 0x0007],
        Flow::End { slots: 0 },
    ),
    (
        "do forever",
        &[0x0E3E, 0x0000, 0x0007],
        Flow::End { slots: 0 },
    ),
    ("idle", &[0x0081], Flow::End { slots: 0 }),
    ("push loop", &[0x1780, 0, 0], Flow::End { slots: 0 }),
    ("push pcstk", &[0x1708], Flow::End { slots: 0 }),
    ("pop loop", &[0x1740], Flow::End { slots: 0 }),
    ("pop pcstk", &[0x1704], Flow::End { slots: 0 }),
    ("cjump", &[0x1800, 0x0000, 0x1234], Flow::End { slots: 0 }),
    (
        "cjump db",
        &[0x1804, 0x0000, 0x1234],
        Flow::End { slots: 2 },
    ),
    (
        "cjump pc-rel db",
        &[0x1844, 0x0000, 0x0010],
        Flow::End { slots: 2 },
    ),
    ("rframe", &[0x1901], Flow::Plain),
];

/// A stand-in for a family that took the instruction: one fast uop of the move family. It has no
/// handler; the structure tests only look at what the translator does around it.
pub(super) fn fast_lower(_cx: &LowerCtx<'_>, c: &CachedInsn, out: &mut BlockBuilder) {
    out.push(Uop::new(
        UopKind(UopKind::MOVE_BASE + 1),
        1,
        c.pc,
        0,
        c.next_pc,
    ));
}

/// [`fast_lower`] with `UF_CB` on the uop of every instruction at one of `cb_pcs`: a stand-in for
/// a DAG access through I7 or I15 that can wrap its circular buffer.
pub(super) fn fast_lower_cb(
    cb_pcs: Vec<u32>,
) -> impl Fn(&LowerCtx<'_>, &CachedInsn, &mut BlockBuilder) {
    move |_cx, c, out| {
        let flags = if cb_pcs.contains(&c.pc) { UF_CB } else { 0 };
        let mut u = Uop::new(UopKind(UopKind::MOVE_BASE + 1), 1, c.pc, 0, c.next_pc);
        u.flags |= flags;
        out.push(u);
    }
}

/// Blocks and uops as `translate_range` returns them.
pub(super) struct Built {
    pub blocks: Vec<Block>,
    pub uops: Vec<Uop>,
}

impl Built {
    /// The blocks translated for PEy off (`false`) or on, in address order, with their uops.
    pub fn mode(&self, simd: bool) -> Vec<(&Block, &[Uop])> {
        self.blocks
            .iter()
            .filter(|b| b.simd == simd)
            .map(|b| (b, &self.uops[b.first as usize..(b.first + b.len) as usize]))
            .collect()
    }

    /// The block of mode `simd` that starts at `pc`.
    pub fn at(&self, pc: u32, simd: bool) -> Option<(&Block, &[Uop])> {
        self.mode(simd).into_iter().find(|(b, _)| b.pc == pc)
    }
}

/// A core with `program` at `CODE` and its translation with `lower`, the pass idle pc `stop_pc`
/// and `cfg`.
pub(super) fn build<L>(
    program: &[u16],
    cfg: &UopConfig,
    stop_pc: Option<u32>,
    lower: L,
) -> (Core, Built)
where
    L: Fn(&LowerCtx<'_>, &CachedInsn, &mut BlockBuilder),
{
    let mut core = core_with(program);
    core.uop_run.stop_pc = stop_pc.unwrap_or(NONE);
    let hi = CODE + program.len() as u32;
    let (blocks, uops) = translate_with(&mut core, cfg, CODE, hi, lower);
    (core, Built { blocks, uops })
}

/// [`build`] with the production lowering and the default configuration.
pub(super) fn build_default(program: &[u16]) -> (Core, Built) {
    build(program, &UopConfig::default(), None, lower_insn)
}

/// [`build`] with the all-`Interp` lowering (`b.interp(c)` for every instruction): the exact-by-
/// construction baseline the structure tests of the translator compare the production lowering
/// with, now that the families take most instructions.
pub(super) fn build_interp(program: &[u16]) -> (Core, Built) {
    build(
        program,
        &UopConfig::default(),
        None,
        crate::core::uop::drive::fixture::interp_lower,
    )
}

/// [`build`] with the production lowering, keeping only the result.
pub(super) fn build_cfg(program: &[u16], cfg: &UopConfig, stop_pc: Option<u32>) -> Built {
    build(program, cfg, stop_pc, lower_insn).1
}

/// `(pc, steps)` of every block of mode `simd`, in address order.
pub(super) fn spans(built: &Built, simd: bool) -> Vec<(u32, u32)> {
    let blocks = built.mode(simd);
    blocks.iter().map(|(b, _)| (b.pc, b.steps)).collect()
}

pub(super) const NOP: u16 = 0x0001;
/// `R3 = R1 + R2`: three parcels, a fixed-point ALU compute.
pub(super) const ALU: [u16; 3] = [0x013e, 0x0000, 0x1312];
/// `JUMP 0x10 (DB)`.
pub(super) const JUMP_DB: [u16; 3] = [0x063E, 0x0400, 0x0010];
/// A load of 1 into universal register 0x6A, a system register.
pub(super) const LOAD_SYS: [u16; 2] = [0x0F80 | 0x6A, 0x0001];

/// `dst = src`, an unconditional register move between two universal register codes (Type 5a, or
/// the 32-bit Type 5b when `short`; fields as `MoveOp::decode` reads them: source bits 42..38 and
/// 32..31, destination bits 29..23, condition bits 37..33 set to TRUE, the 5b marker `0x3F` in
/// the low bits of the second parcel).
pub(super) fn reg_move(src: u8, dst: u8, short: bool) -> Vec<u16> {
    let (src, dst) = (u16::from(src), u16::from(dst));
    let p0 = 0x7000 | (src >> 2) << 6 | 0x3E | (src >> 1) & 1;
    let p1 = (src & 1) << 15 | dst << 7;
    if short {
        vec![p0, p1 | 0x3F]
    } else {
        vec![p0, p1, 0]
    }
}

/// The parcels of an instruction the decoder cannot translate (it faults in the interpreter):
/// the first first-parcel value that decodes to one, with the zero parcels it needs.
pub(super) fn unimplemented_insn() -> Option<Vec<u16>> {
    let mut core = core_with(&[0, 0, 0]);
    (0u16..=u16::MAX).find_map(|p| {
        crate::core::support::load(&mut core, CODE, &[p, 0, 0]);
        core.invalidate_code_cache();
        let c = decode_at(&mut core, CODE);
        let len = (c.next_pc - CODE) as usize;
        (!super::cut::translatable(&c)).then(|| [p, 0, 0][..len].to_vec())
    })
}

/// The state of `c` the entry guard of a block depends on, or that can move `irq.due`: what a
/// plain instruction must leave alone. Not included: the circular-buffer latch (a `UF_CB` uop
/// handles it), the step count and the registers.
pub(super) fn guard_state(c: &Core) -> String {
    let t = &c.timer;
    format!(
        "{:?}",
        (
            c.simd(),
            (c.mode1, c.mode2, c.imask, c.mmask),
            (c.irq.irptl, c.irq.lirptl, c.irq.idle, c.irq.ivt_base),
            (c.irq.window_lo, c.irq.window_len, c.irq.stall),
            (c.irq.due, c.irq.taken, c.irq.wake_at, &c.irq.scheduled),
            (t.tcount, t.tperiod, t.running, t.base_cycle, t.base_count),
            (c.trace.is_some(), c.site_hits.is_empty()),
            c.arith_irq_enabled(),
        )
    )
}

/// What [`lockstep`] saw.
#[derive(Debug, Default)]
pub(super) struct Seen {
    pub blocks: u32,
    pub steps: u64,
    /// Blocks that ended on a flow instruction and left the pc somewhere else than `end_pc`.
    pub taken: u32,
    /// Blocks that ended on a flow instruction (or a delay group) and fell through to `end_pc`.
    pub fell_through: u32,
    /// Blocks that hold a delay group (a delayed branch and its owed slots).
    pub groups: u32,
    /// Blocks whose last instruction is a loop end that jumped back.
    pub loop_backs: u32,
}

/// Runs `a` with `Core::step` and `b` through the blocks of `built`, instruction by instruction,
/// for at most `max_steps` steps, and fails at the first difference: the pc of the interpreter is
/// the pc of the uop, an interior instruction never moves the pc away from the next instruction,
/// the instructions of a block that ran to its end are exactly `block.steps`, and after every
/// block `exact_diff` finds the two cores identical. Stops at a pc without a block, at a fault,
/// and when `irq.due` is reached (the exit sweep of the engine would stop there).
pub(super) fn lockstep(
    a: &mut Core,
    b: &mut Core,
    built: &Built,
    stop_pc: Option<u32>,
    max_steps: u64,
) -> Result<Seen, String> {
    use crate::core::uop::verify::compare::exact_diff;
    let mut seen = Seen::default();
    while seen.steps < max_steps {
        let Some((block, uops)) = built.at(b.pc, b.simd()) else {
            break;
        };
        let start = b.steps;
        // a delay group is a branch with owed slots; the guard class cannot tell (a block with a
        // fast compute is `Arith` first, `guard::classify`), the instructions can
        let mut delayed = false;
        let insns: Vec<&Uop> = uops.iter().filter(|u| u.steps > 0).collect();
        let mut complete = true;
        for (k, u) in insns.iter().enumerate() {
            if a.pc != u.pc || b.pc != u.pc {
                return Err(format!("pc {:#x} / {:#x}, uop at {:#x}", a.pc, b.pc, u.pc));
            }
            let c = decode_at(b, u.pc);
            delayed |= matches!(flow_of(&c), Flow::End { slots } if slots > 0);
            let (rb, ra) = (b.exec_cached(&c), a.step());
            if format!("{ra:?}") != format!("{rb:?}") {
                return Err(format!("result {ra:?} vs {rb:?} at {:#x}", u.pc));
            }
            seen.steps += 1;
            let last = k + 1 == insns.len();
            if !last && (b.pc != c.next_pc || stop_pc == Some(b.pc)) {
                return Err(format!(
                    "interior instruction at {:#x} left the chain",
                    u.pc
                ));
            }
            if rb.is_err() || (!last && b.irq.due <= b.steps) {
                complete = false;
                break;
            }
        }
        if let Some(d) = exact_diff(a, b) {
            return Err(format!("block {:#x}: {d:?}", block.pc));
        }
        if !complete {
            break;
        }
        if b.steps - start != u64::from(block.steps) {
            return Err(format!(
                "block {:#x}: {} steps, budget {}",
                block.pc,
                b.steps - start,
                block.steps
            ));
        }
        seen.blocks += 1;
        seen.groups += u32::from(delayed);
        match (b.pc == block.end_pc, b.loops.is_empty()) {
            (true, _) => seen.fell_through += 1,
            (false, true) => seen.taken += 1,
            (false, false) => seen.loop_backs += 1,
        }
        if b.irq.due <= b.steps {
            break;
        }
    }
    Ok(seen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_translates_both_modes_and_lockstep_follows_the_interpreter() {
        let (mut a, built) = build_default(&[0x0001, 0x013e, 0x0000, 0x1312, 0x0001]);
        let mut b = core_with(&[0x0001, 0x013e, 0x0000, 0x1312, 0x0001]);
        a.irq.due = u64::MAX;
        b.irq.due = u64::MAX;
        assert_eq!((built.mode(false).len(), built.mode(true).len()), (1, 1));
        let seen = lockstep(&mut a, &mut b, &built, None, 100).unwrap();
        assert_eq!((seen.blocks, seen.steps, seen.fell_through), (1, 3, 1));
        assert!(built.at(CODE, false).is_some() && built.at(CODE + 1, false).is_none());
    }

    #[test]
    fn reg_move_encodes_every_source_and_destination_as_the_decoder_reads_them() {
        use crate::core::decode_cache::{DecodedOp, MoveOp};
        for short in [false, true] {
            for (src, dst) in [
                (0u8, 0u8),
                (0x0F, 0x7F),
                (0x10, 0x60),
                (0x7F, 0x01),
                (0x5B, 0x6A),
            ] {
                let mut c = core_with(&reg_move(src, dst, short));
                let pc = c.pc;
                let insn = decode_at(&mut c, pc);
                let DecodedOp::Move(MoveOp::RegMove {
                    src: s,
                    dst: d,
                    cond,
                    compute,
                }) = insn.op
                else {
                    panic!("{src:#x} -> {dst:#x} short={short}: {:?}", insn.op)
                };
                assert_eq!((s, d, cond), (src, dst, 31), "short={short}");
                assert_eq!(compute.is_some(), !short, "only Type 5a carries a compute");
                assert_eq!(insn.next_pc - pc, if short { 2 } else { 3 });
            }
        }
    }

    #[test]
    fn lockstep_reports_a_block_that_does_not_match_the_interpreter() {
        let (mut a, mut built) = build_default(&[0x0001, 0x0001]);
        let mut b = core_with(&[0x0001, 0x0001]);
        a.irq.due = u64::MAX;
        b.irq.due = u64::MAX;
        built.blocks[0].steps = 5;
        assert!(lockstep(&mut a, &mut b, &built, None, 100)
            .unwrap_err()
            .contains("budget"));
    }
}
