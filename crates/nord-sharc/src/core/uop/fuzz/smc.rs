//! Self-modifying code and invalidation: a store into translated code (its own block and a later
//! one), random stores into the program through the DAG, host pokes and overlay loads between
//! slices, and the maintenance that retranslates what was dropped.
//!
//! OWNER: W3-FUZZ. The contract (design 7.4, `docs/specs/sharc-uop.md` section 5): a store into
//! a code page marks it dirty and ends the block with `Exit::Smc` after the store completed; at
//! the next entry the blocks of the dirty pages are revalidated by `code_hash`, the changed ones
//! dropped and run by the interpreter until `uop_maintain` retranslates them. Whatever happens in
//! between, the state must equal the interpreter's at every slice boundary.
use super::driver::sweep;
use super::scenario::{Reg, Scenario, Step};
use super::shapes::{control, insn, loops, straight, vary_cfg, ALL, REGS};
use super::sieve::{cached, pools, Fam};
use super::state::Prep;
use crate::core::decode_cache::{DecodedOp, MoveOp};
use crate::core::uop::exec::h_flow::enc::{self, TRUE};
use crate::core::uop::verify::gen::{Rng, CODE_PC};

/// Cases per test at the CI bound. ASSUMED: 30 keeps the file near 2 s in the debug profile.
const CASES: u64 = 30;

/// A store the lowering takes: `DM(Ii, Mm) = Rn` unconditional, no compute, `Rn` a data register.
struct Store {
    parcels: Vec<u16>,
    ii: usize,
    mi: usize,
    reg: usize,
}

/// Draws a store from the moves pool by decoding every candidate.
fn store(rng: &mut Rng) -> Store {
    let pool = &pools().moves;
    loop {
        let parcels = &pool[rng.below(pool.len() as u32) as usize];
        let Some(DecodedOp::Move(MoveOp::DagMem {
            ii,
            mi,
            ureg,
            write: true,
            cond: 31,
            compute: None,
            ..
        })) = cached(parcels).map(|c| c.op)
        else {
            continue;
        };
        if ureg < 16 {
            return Store {
                parcels: parcels.clone(),
                ii: ii.into(),
                mi: mi.into(),
                reg: ureg.into(),
            };
        }
    }
}

/// A word to store over two parcels of code: two NOPs, or the first parcels of a fast instruction
/// (a one-parcel instruction is followed by a NOP), so the code stays decodable most of the time.
fn replacement(rng: &mut Rng) -> u32 {
    match rng.below(3) {
        0 => 0x0001_0001,
        1 => rng.next_u32(),
        _ => {
            let i = insn(rng, &REGS);
            u32::from(i[0]) | u32::from(*i.get(1).unwrap_or(&0x0001)) << 16
        }
    }
}

/// Short-word pc of every instruction start of `program` (as laid out from `CODE_PC`).
fn starts_of(program: &[u16]) -> Vec<u32> {
    use crate::core::support::core_with;
    use crate::core::uop::lower::decode::decode_at;
    let mut c = core_with(program);
    let (mut pc, end) = (c.pc, c.pc + program.len() as u32);
    let mut starts = Vec::new();
    while pc < end {
        starts.push(pc);
        pc = decode_at(&mut c, pc).next_pc;
    }
    starts
}

/// A program that stores over one of its own later instructions and then runs it, twice (the
/// closing jump goes back to the start): head, store, tail. The index and the value register of
/// the store are pinned, so the store lands on the word that holds the chosen instruction start.
fn store_into_own_code(rng: &mut Rng, seed: u64) -> Scenario {
    let st = store(rng);
    let (head, tail) = (1 + rng.below(3) as usize, 4 + rng.below(8) as usize);
    let mut p = straight(rng, &REGS, head);
    p.extend(&st.parcels);
    let tail_from = p.len();
    p.extend(straight(rng, &REGS, tail));
    let tail_starts: Vec<u32> = starts_of(&p)
        .into_iter()
        .filter(|s| *s >= CODE_PC + tail_from as u32)
        .collect();
    let target = tail_starts[rng.below(tail_starts.len() as u32) as usize];
    p.extend(enc::jump(TRUE, CODE_PC, false, false));
    let mut s = Scenario::new("smc-own", seed, p, 60 + u64::from(rng.below(60)));
    s.prep = Prep {
        stale: true,
        simd: rng.below(2) == 1,
        dag: true,
        ..Prep::default()
    };
    s.cfg = vary_cfg(rng);
    // a store that hits its own block on every pass can leave a case with no fast uop
    s.heavy = true;
    s.set = vec![
        (Reg::I(st.ii), target / 2),
        (Reg::L(st.ii), 0),
        (Reg::M(st.mi), 0),
        (Reg::R(st.reg), replacement(rng)),
    ];
    s
}

#[test]
fn a_store_over_a_later_instruction_of_the_same_program_equals_the_interpreter() {
    let t = sweep("smc-own", 40, CASES, |rng, seed| {
        store_into_own_code(rng, seed)
    });
    assert!(t.smc > 0, "no store ever hit translated code: {t:?}");
}

#[test]
fn random_stores_through_the_dag_into_the_program_equal_the_interpreter() {
    // a third of the index registers point at the program: whatever the moves write there is
    // self-modifying code, and the garbage it makes is decoded alike by both engines
    let t = sweep("smc-random", 41, CASES, |rng, seed| {
        let n = 6 + rng.below(20) as usize;
        let mut s = Scenario::new(
            "smc-random",
            seed,
            straight(rng, &[Fam::Move, Fam::Float], n),
            n as u64 + 8,
        );
        s.prep = Prep {
            stale: true,
            dag: true,
            into_code: true,
            simd: rng.below(2) == 1,
            ..Prep::default()
        };
        s.cfg = vary_cfg(rng);
        s
    });
    assert!(t.smc > 0, "no store ever hit translated code: {t:?}");
}

/// A plan of slices with host changes of the code between them: a poke of one word, an overlay
/// of a whole instruction (the same one, or another), each followed by a maintenance pass or not.
fn plan_with_changes(rng: &mut Rng, program: &[u16]) -> Vec<Step> {
    let starts = starts_of(program);
    let mut plan = vec![Step::Run(8 + u64::from(rng.below(24)))];
    for _ in 0..1 + rng.below(3) {
        let at = starts[rng.below(starts.len() as u32) as usize];
        plan.push(match rng.below(3) {
            0 => Step::Poke {
                word: at / 2,
                value: replacement(rng),
            },
            1 => {
                // reload the very same parcels: revalidation keeps the block (P6)
                let o = (at - CODE_PC) as usize;
                Step::Overlay {
                    pc: at,
                    parcels: program[o..(o + 2).min(program.len())].to_vec(),
                }
            }
            _ => Step::Overlay {
                pc: at,
                parcels: insn(rng, &REGS),
            },
        });
        match rng.below(3) {
            0 => plan.push(Step::Maintain),
            1 => plan.push(Step::MaintainAsync),
            _ => {}
        }
        plan.push(Step::Run(8 + u64::from(rng.below(40))));
    }
    plan
}

#[test]
fn host_pokes_and_overlays_between_slices_equal_the_interpreter() {
    let t = sweep("smc-poke", 42, CASES, |rng, seed| {
        let (p, starts) = if rng.below(2) == 0 {
            {
                let n = 8 + rng.below(8) as usize;
                control(rng, &REGS, n)
            }
        } else {
            let p = loops(rng, &REGS);
            (p, vec![CODE_PC])
        };
        let mut s = Scenario::new("smc-poke", seed, p.clone(), 0);
        s.plan = plan_with_changes(rng, &p);
        s.prep = Prep {
            stale: true,
            simd: rng.below(2) == 1,
            ..Prep::default()
        };
        s.cfg = vary_cfg(rng);
        s.targets = starts;
        s
    });
    assert!(t.maintain > 0, "no maintenance pass ran: {t:?}");
}

#[test]
fn code_changed_while_the_engine_is_off_the_blocks_is_retranslated_and_equal() {
    // run, change the code, maintain, run again, change again: the blocks come and go and the
    // result never differs
    sweep("smc-cycle", 43, CASES / 2, |rng, seed| {
        let p = {
            let n = 10 + rng.below(10) as usize;
            straight(rng, &ALL, n)
        };
        let mut s = Scenario::new("smc-cycle", seed, p.clone(), 0);
        let mut plan = Vec::new();
        for _ in 0..3 {
            plan.push(Step::Run(4 + u64::from(rng.below(6))));
            let at = starts_of(&p)[rng.below(6) as usize];
            plan.push(Step::Poke {
                word: at / 2,
                value: replacement(rng),
            });
            plan.push(if rng.below(2) == 0 {
                Step::Maintain
            } else {
                Step::MaintainAsync
            });
        }
        plan.push(Step::Run(6));
        s.plan = plan;
        s.prep = Prep::full(rng.below(2) == 1);
        s.heavy = true;
        s.cfg = vary_cfg(rng);
        s
    });
}
