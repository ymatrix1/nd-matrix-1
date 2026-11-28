//! Program shapes: straight code per family, counted and arithmetic loops (nested), delayed
//! branches with their slots, calls and returns, system-register loads. Every body instruction
//! is a fast instruction from the pools of `sieve.rs`, so a program is never "all `Interp`".
//!
//! OWNER: W3-FUZZ. All targets lie on the start of an instruction of the program itself (the
//! second return value of [`control`]), so a run does not leave the program at the first jump.
//! The shapes cover the contract of design 4.5 and P1/P2: a loop end as a block-final uop with
//! `UF_FIN`, a delayed branch whose two slot instructions carry `UF_FIN`, a terminator inside a
//! loop body, `DO UNTIL` with a data-dependent exit, indirect jumps, `RTS` on a prepared stack.
use super::sieve::{pools, Fam};
use crate::core::uop::config::UopConfig;
use crate::core::uop::exec::h_flow::enc::{self, TRUE};
use crate::core::uop::verify::gen::{Rng, CODE_PC};

/// Block-length limits drawn for a case: the cuts of the translator move with the limit, so the
/// same program meets block ends at different places. ASSUMED: 2 and 3 force many tiny blocks,
/// 48 is the default; the three between exercise ordinary cuts.
const BLOCK_LIMITS: [u32; 6] = [2, 3, 5, 8, 16, 48];

/// A config with a drawn block limit (the minimum stays 1).
pub(super) fn vary_cfg(rng: &mut Rng) -> UopConfig {
    UopConfig {
        max_block_steps: rng.pick(&BLOCK_LIMITS),
        ..UopConfig::default()
    }
}

/// One instruction of the pools of `fams`, drawn with equal weight per family.
pub(super) fn insn(rng: &mut Rng, fams: &[Fam]) -> Vec<u16> {
    let pool = pools().of(rng.pick(fams));
    pool[rng.below(pool.len() as u32) as usize].clone()
}

/// An instruction of the circular-buffer pool (the I7 and I15 wrap latch), `cb` of the pools.
pub(super) fn cb_insn(rng: &mut Rng) -> Vec<u16> {
    let pool = &pools().cb;
    pool[rng.below(pool.len() as u32) as usize].clone()
}

/// `n` fast instructions of the families `fams`, concatenated.
pub(super) fn straight(rng: &mut Rng, fams: &[Fam], n: usize) -> Vec<u16> {
    (0..n).flat_map(|_| insn(rng, fams)).collect()
}

/// Every family.
pub(super) const ALL: [Fam; 3] = [Fam::Float, Fam::Fixed, Fam::Move];
/// The families that touch no memory: safe inside control shapes whose index registers are
/// aimed at code starts.
pub(super) const REGS: [Fam; 2] = [Fam::Float, Fam::Fixed];

/// A nested counted loop of depth `depth` (0 is a single loop): `DO .. UNTIL LCE` or
/// `DO .. UNTIL cond`, body of 1 to 4 instructions with the inner loop somewhere before the last
/// one (the loop end is the start of the last instruction of the body).
pub(super) fn do_loop(rng: &mut Rng, fams: &[Fam], depth: u32) -> Vec<u16> {
    let units = 2 + rng.below(3) as usize;
    let inner_at = (depth > 0).then(|| rng.below(units as u32 - 1) as usize);
    let mut body = Vec::new();
    let mut last_len = 0;
    for k in 0..units {
        if Some(k) == inner_at {
            body.extend(do_loop(rng, fams, depth - 1));
            continue;
        }
        let i = insn(rng, fams);
        last_len = i.len();
        body.extend(i);
    }
    let end_off = 3 + (body.len() - last_len) as u32;
    let mut out = match rng.below(4) {
        0 => enc::do_until(rng.below(32) as u8, end_off),
        1 => enc::do_reg(rng.below(16) as u8, end_off),
        _ => enc::do_count(1 + rng.below(4) as u16, end_off),
    };
    out.extend(body);
    out
}

/// A program of one loop nest between a prologue and an epilogue of fast instructions.
pub(super) fn loops(rng: &mut Rng, fams: &[Fam]) -> Vec<u16> {
    let (head, depth, tail) = (1 + rng.below(2), rng.below(3), 2 + rng.below(3));
    let mut p = straight(rng, fams, head as usize);
    p.extend(do_loop(rng, fams, depth));
    p.extend(straight(rng, fams, tail as usize));
    p
}

/// The kinds of one slot of [`control`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Body,
    Jump,
    Call,
    JumpI,
    Rts,
    Do,
    DoUntil,
    SysLoad,
}

/// The system registers a load may write: MODE1, MODE2 and IMASK. VERIFIED: the numbers the
/// `h_flow` tests use (`diff_events.rs`: 0x72 MODE1, 0x7B IMASK; 0x73 is the next register).
const SYS_REGS: [u8; 3] = [0x72, 0x73, 0x7B];

/// A program of `slots` slots chosen from fast bodies and control flow: absolute and relative
/// jumps, calls, indirect jumps through DAG2, returns, delayed variants of each with two slot
/// instructions, `DO` loops and system loads. Returns the parcels and the pc of every slot
/// start; the caller points the indirect registers and the call stack at those.
pub(super) fn control(rng: &mut Rng, fams: &[Fam], slots: usize) -> (Vec<u16>, Vec<u32>) {
    let kinds: Vec<Slot> = (0..slots)
        .map(|k| {
            if k == 0 {
                return Slot::Body; // the first instruction always runs: the case is exercised
            }
            rng.pick(&[
                Slot::Body,
                Slot::Body,
                Slot::Body,
                Slot::Body,
                Slot::Jump,
                Slot::Jump,
                Slot::Call,
                Slot::JumpI,
                Slot::Rts,
                Slot::Do,
                Slot::Do,
                Slot::DoUntil,
                Slot::SysLoad,
            ])
        })
        .collect();
    let delayed: Vec<bool> = kinds
        .iter()
        .map(|k| {
            matches!(k, Slot::Jump | Slot::Call | Slot::JumpI | Slot::Rts) && rng.below(3) == 0
        })
        .collect();
    // draw the bodies first: the layout depends on their lengths
    let bodies: Vec<Vec<u16>> = (0..slots).map(|_| insn(rng, fams)).collect();
    let slot_pair: Vec<[Vec<u16>; 2]> = (0..slots)
        .map(|_| [insn(rng, fams), insn(rng, fams)])
        .collect();
    // a system load is 2 parcels when the value fits 16 bits (sign extended) and 3 otherwise
    let sys: Vec<Vec<u16>> = (0..slots)
        .map(|_| {
            let reg = rng.pick(&SYS_REGS);
            let value = match reg {
                0x72 => (rng.next_u32() & 0x0003_E000) | rng.below(2) << 21,
                0x7B => rng.next_u32() & 0xFFFF_F000,
                _ => rng.next_u32() & 0xFF,
            };
            enc::load_imm(reg, value)
        })
        .collect();
    let len = |k: usize| -> u32 {
        let base = match kinds[k] {
            Slot::Body => bodies[k].len(),
            Slot::Jump | Slot::Call | Slot::Do | Slot::DoUntil => 3,
            Slot::JumpI => 2,
            Slot::SysLoad => sys[k].len(),
            Slot::Rts => 1,
        } as u32;
        let extra = if delayed[k] {
            (slot_pair[k][0].len() + slot_pair[k][1].len()) as u32
        } else {
            0
        };
        base + extra
    };
    let mut starts = Vec::with_capacity(slots);
    let mut pc = CODE_PC;
    for k in 0..slots {
        starts.push(pc);
        pc += len(k);
    }
    let mut p = Vec::new();
    for k in 0..slots {
        let cond = if rng.below(2) == 0 {
            TRUE
        } else {
            rng.below(32) as u8
        };
        let target = starts[rng.below(slots as u32) as usize];
        // a loop must end after its own header and inside the program
        let ahead = starts[(k + 1 + rng.below(3) as usize).min(slots - 1)].max(starts[k] + 3);
        let end_off = ahead - starts[k];
        let d = delayed[k];
        match kinds[k] {
            Slot::Body => p.extend(&bodies[k]),
            Slot::Jump => p.extend(enc::jump(cond, target, false, d)),
            Slot::Call => p.extend(enc::jump(cond, target, true, d)),
            Slot::JumpI => p.extend(enc::jump_i(
                cond,
                8 + rng.below(8) as u8,
                8 + rng.below(8) as u8,
                false,
                d,
            )),
            Slot::Rts => p.extend(enc::rts(cond, d, false)),
            Slot::Do => p.extend(enc::do_count(1 + rng.below(3) as u16, end_off)),
            Slot::DoUntil => p.extend(enc::do_until(cond, end_off)),
            Slot::SysLoad => p.extend(&sys[k]),
        }
        if d {
            p.extend(&slot_pair[k][0]);
            p.extend(&slot_pair[k][1]);
        }
    }
    // somewhere to run off the end: fast instructions, not zeros
    p.extend(straight(rng, fams, 3));
    (p, starts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::uop::lower::decode::decode_at;

    /// Walks `program` instruction by instruction and returns the start of each.
    fn starts_of(program: &[u16]) -> Vec<u32> {
        let mut c = core_with(program);
        let (mut pc, end) = (c.pc, c.pc + program.len() as u32);
        let mut starts = Vec::new();
        while pc < end {
            starts.push(pc);
            pc = decode_at(&mut c, pc).next_pc;
        }
        assert_eq!(pc, end, "the program ends on an instruction boundary");
        starts
    }

    #[test]
    fn straight_and_loop_programs_decode_to_whole_instructions() {
        let mut rng = Rng::new(3);
        for _ in 0..30 {
            starts_of(&straight(&mut rng, &ALL, 8));
            starts_of(&loops(&mut rng, &REGS));
            starts_of(&loops(&mut rng, &ALL));
        }
    }

    #[test]
    fn control_programs_decode_whole_and_every_slot_start_is_an_instruction_start() {
        let mut rng = Rng::new(5);
        for _ in 0..40 {
            let (p, slot_starts) = control(&mut rng, &REGS, 12);
            let all = starts_of(&p);
            assert!(slot_starts.iter().all(|s| all.contains(s)));
            assert_eq!(slot_starts[0], CODE_PC);
        }
    }

    #[test]
    fn a_loop_nest_decodes_whole_and_its_first_instruction_is_a_loop_header() {
        let mut rng = Rng::new(9);
        for _ in 0..60 {
            let p = do_loop(&mut rng, &REGS, 1);
            let starts = starts_of(&p);
            assert!(
                starts.len() >= 5,
                "an outer loop holds an inner one: {p:04x?}"
            );
            let mut c = core_with(&p);
            let first = decode_at(&mut c, CODE_PC);
            assert_eq!(first.insn.len, 3, "{p:04x?}");
        }
    }

    #[test]
    fn every_case_of_a_shape_draws_fast_instructions_and_the_block_limit_varies() {
        let mut rng = Rng::new(1);
        let limits: std::collections::BTreeSet<u32> = (0..60)
            .map(|_| vary_cfg(&mut rng).max_block_steps)
            .collect();
        assert_eq!(limits.len(), BLOCK_LIMITS.len());
        let cb = cb_insn(&mut rng);
        assert!(!cb.is_empty() && !insn(&mut rng, &ALL).is_empty());
    }
}
