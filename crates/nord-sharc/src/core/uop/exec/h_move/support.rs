//! Test harness of the move handlers (P8 recipe, P13 fillers): builds identical cores, draws
//! instructions of a form from the VISA classifier, lowers them with `MoveFamily`, runs the uops
//! through the real handler table with a faithful copy of the dispatch protocol (P2), and compares
//! with `Core::step` through `exact_diff`.
//!
//! OWNER: W2-MOVE, test build only. The state filler is the move family's own (P13): DAG
//! registers with live circular buffers, MODE1 bits of this domain, SRAM data around the indices
//! and every kind of address (SRAM, window edges, IOP, short-word alias, SDRAM, device).
use crate::core::decode_cache::CachedInsn;
use crate::core::types::LoopFrame;
use crate::core::uop::exec::mem::LEN_SHIFT;
use crate::core::uop::hot::UopHot;
use crate::core::uop::lower::builder::BlockBuilder;
use crate::core::uop::lower::decode::decode_at;
use crate::core::uop::lower::moves::MoveFamily;
use crate::core::uop::lower::{Family, LowerCtx};
use crate::core::uop::types::{bail_handler, Exit, Handler, Uop, UopCtx, UF_CB, UF_FIN};
use crate::core::uop::verify::compare::{exact_diff, ExactDiff};
use crate::core::uop::verify::gen::{core_with_code, fill_stale_ext, fill_state, Rng, CODE_PC};
use crate::core::Core;
use crate::visa::{bits, classify, word48, Kind};

pub(super) const NOP: u16 = 0x0001;
/// Data region the indices point into by default (the code sits at normal word `0x9_0000`).
pub(super) const DATA: u32 = 0x9_8000;
/// MODE1 bits of the move domain: bit-reversed `I8` and `I0`, broadcast `I9` and `I1`, circular
/// buffers (`dag.rs`, `dag_modes.rs`).
const MODE1_DAG: u32 = 1 | 1 << 1 | 1 << 22 | 1 << 23 | 1 << 24;

/// An address of the kinds a transfer meets, SRAM first.
pub(super) fn pick_addr(rng: &mut Rng) -> u32 {
    match rng.below(20) {
        0..=11 => DATA + rng.below(0x100),
        12 => 0x9_0000 + rng.below(4),     // the code words themselves
        13 => 0xF_FFFC + rng.below(4),     // top of the window, `+ 1` leaves it
        14 => 0x9_0000 - 2 + rng.below(4), // bottom edge
        15 => 0x3_E000 + rng.below(8),     // IOP
        16 => 0x12_4000 + rng.below(8),    // short-word alias of SRAM
        17 => 0x0802_0000 + rng.below(8),  // SDRAM
        18 => 0x70_0000 + rng.below(8),    // device store
        _ => [0x3108_D004, u32::MAX, 0, 0x1081][rng.below(4) as usize], // periph, wrap, low
    }
}

/// Fills the DAG registers (live circular buffers), MODE1, the SRAM data and the other windows.
pub(super) fn fill_dag(c: &mut Core, rng: &mut Rng) {
    for k in 0..16 {
        c.i[k] = pick_addr(rng);
        c.m[k] = [0, 1, 2, 3, u32::MAX, u32::MAX - 1, 7, 0x100][rng.below(8) as usize];
        if rng.below(2) == 0 {
            c.l[k] = 2 + rng.below(7);
            c.b[k] = DATA + 0x80 + rng.below(0x20);
            c.i[k] = c.b[k] + rng.below(c.l[k]);
        } else if rng.below(4) == 0 {
            (c.l[k], c.b[k]) = (0, pick_addr(rng));
        }
    }
    c.mode1 = (c.mode1 & !MODE1_DAG) | (rng.next_u32() & MODE1_DAG);
    // data in the SRAM window around the indices and at its top edge, in the IOP and the device
    // store, through the interpreter's own write path
    let words = (DATA - 8..DATA + 0x300)
        .chain(0xF_FFF8..0x10_0000)
        .chain(0x3_E000..0x3_E008)
        .chain(0x70_0000..0x70_0008);
    for w in words {
        let _ = c.dm_write(w, rng.next_u32());
    }
    c.irq.due = u64::MAX;
}

/// The core of one case: the program at `CODE_PC` followed by NOPs, every register drawn from
/// `seed`, PEy on when `simd`. Two calls with the same arguments give identical cores.
pub(super) fn build(parcels: &[u16], seed: u64, simd: bool) -> Core {
    let mut c = core_with_code(&[parcels, &[NOP; 4]].concat());
    let mut rng = Rng::new(seed);
    fill_state(&mut c, &mut rng, simd);
    fill_stale_ext(&mut c, &mut rng);
    fill_dag(&mut c, &mut rng);
    c
}

/// A `LoopFrame` whose last instruction is the one at `CODE_PC`: after it the step jumps back.
pub(super) fn loop_end_here(c: &mut Core) {
    c.loops.push(LoopFrame {
        start: CODE_PC + 0x40,
        end: CODE_PC,
        remaining: 3,
        term: None,
        check: u32::MAX,
        ..Default::default()
    });
}

fn patch(w: u64, hi: u32, lo: u32, v: u64) -> u64 {
    let mask = (u64::MAX >> (63 - (hi - lo))) << lo;
    (w & !mask) | ((v << lo) & mask)
}

/// A random instruction of one of `kinds`, drawn from the classifier, with the fields the lowering
/// looks at steered towards the forms it takes: unconditional, no compute, no long word, and a
/// register below the system registers most of the time.
pub(super) fn rand_insn(rng: &mut Rng, kinds: &[Kind]) -> Vec<u16> {
    loop {
        let p = [
            rng.next_u32() as u16,
            rng.next_u32() as u16,
            rng.next_u32() as u16,
        ];
        let (kind, len) = classify(p[0], Some(p[1]));
        if !kinds.contains(&kind) || len > 3 {
            continue;
        }
        // one draw in eight keeps the raw fields: conditional, long-word, compute and system
        // register forms, which the lowering must decline
        let keep = rng.below(8) != 0;
        let w = steer(kind, word48(&p), keep, rng);
        let out = [(w >> 32) as u16, (w >> 16) as u16, w as u16];
        return out[..len].to_vec();
    }
}

/// Steers the fields of one instruction word towards the forms the lowering takes (`keep`).
fn steer(kind: Kind, mut w: u64, keep: bool, rng: &mut Rng) -> u64 {
    if !keep {
        return w;
    }
    // a 7-bit register field below the system registers
    let tame = |w: u64, hi, lo| {
        let v = bits(w, hi, lo);
        if v >= 0x60 {
            patch(w, hi, lo, v & 0x5F)
        } else {
            w
        }
    };
    if matches!(
        kind,
        Kind::T3a
            | Kind::T3b
            | Kind::T4a
            | Kind::T4b
            | Kind::T5a
            | Kind::T5b
            | Kind::T7a
            | Kind::T7b
    ) {
        w = patch(w, 37, 33, 31); // IF TRUE
    }
    if matches!(
        kind,
        Kind::T1a | Kind::T3a | Kind::T4a | Kind::T5a | Kind::T7a
    ) {
        w = patch(w, 22, 0, 0); // no compute
    }
    match kind {
        Kind::T14a => {
            w = tame(w, 38, 32);
            w = patch(w, 39, 39, 0); // no long word
            patch(w, 31, 0, u64::from(pick_addr(rng)))
        }
        Kind::T15a => {
            w = tame(w, 38, 32);
            w = patch(w, 39, 39, 0);
            let offset = if rng.below(2) == 0 {
                rng.below(0x40)
            } else {
                rng.below(0x40).wrapping_neg()
            };
            patch(w, 31, 0, u64::from(offset))
        }
        Kind::T15b => patch(tame(w, 29, 23), 39, 39, 0),
        Kind::T3a | Kind::T3b => patch(tame(w, 29, 23), 30, 30, 0),
        Kind::T5a | Kind::T5b => {
            w = tame(w, 29, 23);
            // the source of a register move (not a swap) is two fields: 42..38 and 32..31
            let src_hi = bits(w, 42, 38);
            if bits(w, 43, 43) == 0 && src_hi >= 24 {
                w = patch(w, 42, 38, src_hi & 0x17);
            }
            w
        }
        Kind::T17a | Kind::T17b => tame(w, 38, 32),
        _ => w,
    }
}

/// What one case did.
pub(super) struct Run {
    /// `MoveFamily` took the instruction.
    pub taken: bool,
    pub uops: Vec<Uop>,
    /// Exit of the last handler call.
    pub exit: Option<Exit>,
    /// First difference to the interpreter after the instruction (the handler's, then `Core::step`
    /// for a bail): `None` means identical.
    pub diff: Option<ExactDiff>,
    /// The interpreter's core after the step.
    pub want: Core,
    /// The core the handlers ran on, after materialization (and the bail's `Core::step`).
    pub got: Core,
}

/// Lowers the instruction at `CODE_PC` of `c` with `MoveFamily`.
pub(super) fn lower(c: &mut Core, simd: bool) -> (Option<Vec<Uop>>, CachedInsn) {
    let insn = decode_at(c, CODE_PC);
    let cfg = Default::default();
    let cx = LowerCtx {
        cfg: &cfg,
        stop_pc: None,
        simd,
        isa: false,
    };
    let mut b = BlockBuilder::new(CODE_PC, simd, false);
    let taken = MoveFamily::lower(&cx, &insn, &mut b);
    (taken.then(|| b.uops().to_vec()), insn)
}

/// The dispatch protocol of `exec/mod.rs` (P2) for the uops of one instruction, followed by the
/// materialization of design 4.5 rule 2. `fin` marks the last uop `UF_FIN`, as the translator does
/// for a delay slot or a loop end.
pub(super) fn dispatch(core: &mut Core, uops: &[Uop], insn: &CachedInsn) -> Exit {
    let table = UopHot::handler_table();
    let base = core.steps;
    let mut ctx = UopCtx::new(core as *mut Core, base, 1000);
    let (mut exit, mut pc) = (Exit::Ok, insn.next_pc);
    for (k, u) in uops.iter().enumerate() {
        let bail = bail_handler as Handler as usize;
        assert_ne!(
            table[usize::from(u.h)] as usize,
            bail,
            "kind {:#x} has no handler",
            u.h
        );
        let r = unsafe { table[usize::from(u.h)](&mut ctx, u) };
        exit = Exit::from_u32(r).expect("a handler returns an Exit");
        let core = unsafe { &mut *ctx.core };
        match exit {
            Exit::Ok => {
                ctx.done += u32::from(u.steps);
                if u.flags & UF_CB != 0 && core.cb_wrap.get() != 0 {
                    core.latch_cb_overflow();
                }
                if u.flags & UF_FIN != 0 {
                    pc = core.finish_step(u.pc, insn.insn.kind, insn.next_pc);
                }
            }
            Exit::Jump => {
                ctx.done += u32::from(u.steps);
                pc = ctx.next_pc;
                break;
            }
            Exit::Smc => {
                ctx.done += u32::from(u.steps);
                pc = uops.get(k + 1).map_or(insn.next_pc, |n| n.pc);
                break;
            }
            _ => {
                pc = u.pc;
                break;
            }
        }
    }
    let core = unsafe { &mut *ctx.core };
    core.pc = pc;
    core.steps = base + u64::from(ctx.done);
    exit
}

/// One differential case: the interpreter steps once on `want`, the handlers run on `got`; a
/// bailing handler is followed by `Core::step`, after a check that it changed nothing.
/// `tweak` runs on both cores after the fill (watches, peripherals, loop frames, ...).
pub(super) fn run(
    parcels: &[u16],
    seed: u64,
    simd: bool,
    fin: bool,
    tweak: &dyn Fn(&mut Core),
) -> Run {
    let make = || {
        let mut c = build(parcels, seed, simd);
        tweak(&mut c);
        c
    };
    let (mut want, mut got, mut pristine) = (make(), make(), make());
    let stepped = want.step();
    let (lowered, insn) = lower(&mut got, simd);
    let _ = decode_at(&mut pristine, CODE_PC); // the decode path leaves the same marks everywhere
    let Some(mut uops) = lowered else {
        return Run {
            taken: false,
            uops: vec![],
            exit: None,
            diff: None,
            want,
            got,
        };
    };
    assert!(stepped.is_ok(), "the interpreter faults on {parcels:x?}");
    if fin {
        if let Some(last) = uops.last_mut() {
            last.flags |= UF_FIN;
        }
    }
    let exit = dispatch(&mut got, &uops, &insn);
    let mut diff = None;
    if matches!(exit, Exit::Slow | Exit::Fault | Exit::Guard) {
        // a uop that bails must not have changed anything: the interpreter runs the instruction
        diff = exact_diff(&got, &pristine);
        if diff.is_none() {
            let _ = got.step();
        }
    }
    let diff = diff
        .or_else(|| exact_diff(&want, &got))
        .or_else(|| hits_diff(&want, &got));
    Run {
        taken: true,
        uops,
        exit: Some(exit),
        diff,
        want,
        got,
    }
}

/// The watch logs, which `exact_diff` leaves out: the same accesses with the same `steps`, `pc`
/// and value.
fn hits_diff(a: &Core, b: &Core) -> Option<ExactDiff> {
    let pairs = [
        (
            "watch_hits",
            format!("{:?}", a.watch_hits),
            format!("{:?}", b.watch_hits),
        ),
        (
            "rwatch_hits",
            format!("{:?}", a.rwatch_hits),
            format!("{:?}", b.rwatch_hits),
        ),
    ];
    pairs
        .into_iter()
        .find(|(_, x, y)| x != y)
        .map(|(field, a, b)| ExactDiff { field, a, b })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::UopKind;

    #[test]
    fn the_harness_draws_forms_lowers_them_and_runs_the_handlers() {
        let mut rng = Rng::new(7);
        let mut seen = std::collections::BTreeSet::new();
        for k in 0..400 {
            let p = rand_insn(
                &mut rng,
                &[
                    Kind::T3a,
                    Kind::T3b,
                    Kind::T3c,
                    Kind::T4b,
                    Kind::T14a,
                    Kind::T15b,
                ],
            );
            let r = run(&p, k, k % 2 == 0, false, &|_| {});
            assert_eq!(r.diff, None, "{p:x?} seed {k}");
            if r.taken {
                assert_eq!(r.exit.map(|e| e as u32 <= Exit::Smc as u32), Some(true));
                seen.insert(UopKind(r.uops[0].h).0);
                assert_eq!(r.uops[0].a >> LEN_SHIFT, p.len() as u32);
            }
        }
        assert!(seen.len() > 8, "{} transfer kinds ran", seen.len());
    }

    #[test]
    fn building_twice_gives_identical_cores_and_patching_replaces_a_field() {
        let a = build(&[NOP], 5, true);
        assert_eq!(exact_diff(&a, &build(&[NOP], 5, true)), None);
        assert!(exact_diff(&a, &build(&[NOP], 6, true)).is_some());
        assert_eq!(patch(0xFFFF_FFFF_FFFF, 37, 33, 0), 0xFFC1_FFFF_FFFF);
        assert_eq!(patch(0, 6, 0, 0x1FF), 0x7F);
    }
}
