//! Handler-level differential of the float uops against `Core::step` (recipe P8 of the design: no
//! driver and no translator, only the lowering of one instruction and its handler).
//!
//! OWNER: W2-FLOAT, test only. Per case: two cores with the same random state, the interpreter
//! steps one instruction on one, the lowering turns the same instruction into a uop and its
//! handler runs on the other, the dispatch accounting (steps of an `Ok` or `Jump` uop, the pc)
//! is applied as the driver does, and `exact_diff` must find nothing: registers, hidden float
//! extensions (tag and low byte, stale ones included), flags, ASTAT, STKY, the latched interrupts
//! and `irq.due`, the bank copies of the register files and 4 MiB of memory.
//!
//! The state is as hostile as the generators make it (design 6.2): grid operands (zero, negative
//! zero, denormals, infinities, NaN, exponent edges, rounding ties) with a live or a stale
//! extension, `fill_stale_ext` extensions of an older register value, SRRFL and SRRFH toggles
//! that bring them back to life, random sticky bits, ASTAT extras, FIFO state and MODE1 float
//! bits, SIMD on and off, and every condition code with its own flags per element.
//!
//! Non-vacuous (design P10): [`Tally`] counts, per kind and mode, the cases and the handler
//! exits, the four (PEx, PEy) enable combinations, stale operands that were read, hidden
//! extension writes, sticky exceptions and `Jump` exits, and the tests assert minimums.
use super::support::{alu, computes, mul, prepare, t2a, ALU_OPS, SRRFH, SRRFL};
use super::*;
use crate::core::decode_cache::ComputeOp;
use crate::core::support::core_with;
use crate::core::uop::config::UopConfig;
use crate::core::uop::exec::exit::materialize;
use crate::core::uop::kernels::grid;
use crate::core::uop::lower::builder::BlockBuilder;
use crate::core::uop::lower::decode::decode_at;
use crate::core::uop::lower::{Family, LowerCtx};
use crate::core::uop::types::{Exit, Uop, UopCtx, UF_LAST};
use crate::core::uop::verify::compare::exact_diff;
use crate::core::uop::verify::gen::{
    fill_astat_extra, fill_fifo, fill_stale_ext, fill_state, fill_sticky, Rng,
};
use crate::core::Core;
use std::collections::BTreeMap;

/// The lowering of the instruction at the pc of `c` (the family must take it) followed by a NOP
/// uop, so that "the next uop" of a `Jump` exists.
pub(super) fn lowered(c: &mut Core) -> Vec<Uop> {
    let pc = c.pc;
    let insn = decode_at(c, pc);
    let cfg = UopConfig::default();
    let cx = LowerCtx {
        cfg: &cfg,
        stop_pc: None,
        simd: c.simd(),
        isa: false,
    };
    let mut b = BlockBuilder::new(pc, c.simd(), false);
    assert!(
        crate::core::uop::lower::float::FloatFamily::lower(&cx, &insn, &mut b),
        "instruction at {pc:#x} not taken"
    );
    let mut uops = b.uops().to_vec();
    uops.push(Uop::new(
        crate::core::uop::types::UopKind::NOP,
        1,
        insn.next_pc,
        0,
        0,
    ));
    uops
}

/// What running the first uop did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ran {
    pub exit: Exit,
    pub next_pc: u32,
}

/// Runs `uops[0]` on `c` through its handler and applies the accounting of the dispatch loop: an
/// `Ok` or `Jump` uop adds its steps, the pc is the next uop's (`Ok`) or `ctx.next_pc` (`Jump`).
pub(super) fn run_uop(c: &mut Core, table: &[Handler], uops: &[Uop]) -> Ran {
    let (u, next) = (&uops[0], uops[1].pc);
    let mut ctx = UopCtx::new(c as *mut Core, c.steps, 8);
    // SAFETY: `ctx.core` is `c`, which is not used until the handler has returned; `uops` holds
    // the uop and its successor
    let r = unsafe { table[usize::from(u.h)](&mut ctx, u) };
    let exit = Exit::from_u32(r).expect("a handler returns an exit code");
    let pc = match exit {
        Exit::Ok => next,
        Exit::Jump => ctx.next_pc,
        other => {
            return Ran {
                exit: other,
                next_pc: c.pc,
            }
        }
    };
    ctx.done += u32::from(u.steps);
    materialize(c, &ctx, pc);
    Ran { exit, next_pc: pc }
}

/// What a run covered.
#[derive(Debug, Default)]
pub(super) struct Tally {
    pub cases: usize,
    pub per_kind: BTreeMap<(u16, bool), usize>,
    pub oks: usize,
    pub jumps: usize,
    /// (PEx enabled, PEy enabled) -> cases, SIMD cases only for the PEy entries.
    pub enables: BTreeMap<(bool, bool), usize>,
    pub stale_read: usize,
    pub hidden_changed: usize,
    pub sticky_raised: usize,
    /// Cases in which the interpreter's step poked the controller (`irq.due` is 0 afterwards).
    pub pokes: usize,
    pub changed: usize,
}

/// The kind of the first uop and what the interpreter's condition says about each element.
fn enables(c: &Core, cond: u32) -> (bool, bool) {
    (
        cond == 31 || c.cond_for(false, u64::from(cond)),
        c.simd() && (cond == 31 || c.cond_for(true, u64::from(cond))),
    )
}

/// One case of [`case`]: an instruction and the state it runs on.
pub(super) struct Setup<'a> {
    pub parcels: &'a [u16],
    /// The condition code the instruction carries (31 = unconditional).
    pub cond: u32,
    pub seed: u64,
    pub simd: bool,
    /// MODE1 bits flipped after the state is filled (SRRFL, SRRFH).
    pub toggle: u32,
    /// Marks the uop as the last of its block (`UF_LAST`).
    pub last: bool,
}

/// One instruction on one random state: interpreter against handler.
pub(super) fn case(t: &mut Tally, table: &[Handler], setup: &Setup<'_>) {
    let Setup {
        parcels,
        cond,
        seed,
        simd,
        toggle,
        last,
    } = *setup;
    let (mut a, mut b) = (
        prepare(parcels, seed, simd, toggle),
        prepare(parcels, seed, simd, toggle),
    );
    let before = prepare(parcels, seed, simd, toggle);
    let mut uops = lowered(&mut b);
    if last {
        uops[0].flags |= UF_LAST;
    }
    let ctx = format!("{parcels:x?} cond {cond} seed {seed:#x} simd {simd} toggle {toggle:#x}");
    let en = enables(&a, cond);
    a.step().expect("the interpreter runs the instruction");
    let ran = run_uop(&mut b, table, &uops);
    assert!(
        matches!(ran.exit, Exit::Ok | Exit::Jump),
        "{ctx}: exit {:?}",
        ran.exit
    );
    if let Some(d) = exact_diff(&a, &b) {
        panic!("handler vs interpreter, {ctx}: {d}");
    }
    // the exit is Jump exactly when an exception poked the controller of a uop that may exit
    assert_eq!(
        ran.exit == Exit::Jump,
        a.irq.due == 0 && !last,
        "{ctx}: exit {:?}, due {}",
        ran.exit,
        a.irq.due
    );
    t.cases += 1;
    *t.per_kind.entry((uops[0].h, simd)).or_default() += 1;
    t.oks += usize::from(ran.exit == Exit::Ok);
    t.jumps += usize::from(ran.exit == Exit::Jump);
    *t.enables.entry(en).or_default() += 1;
    t.changed += usize::from(exact_diff(&before, &b).is_some());
    t.hidden_changed += usize::from(
        (before.fx.tag, before.fx.lo, before.fx_y.tag, before.fx_y.lo)
            != (b.fx.tag, b.fx.lo, b.fx_y.tag, b.fx_y.lo),
    );
    t.sticky_raised += usize::from(before.sys.stky != b.sys.stky);
    t.pokes += usize::from(a.irq.due == 0);
    let stale = |c: &Core| (0..16).any(|n| c.fx.tag[n] != c.r[n] && c.fx.lo[n] != 0);
    t.stale_read += usize::from(stale(&before));
}

/// A seeded run: `rounds` states, every compute of [`computes`], both modes, conditions and bank
/// toggles drawn per case.
pub(super) fn run(seed: u64, rounds: usize, only: impl Fn(ComputeOp) -> bool) -> Tally {
    let table = crate::core::uop::hot::UopHot::handler_table();
    let mut rng = Rng::new(seed);
    let mut t = Tally::default();
    for _ in 0..rounds {
        for simd in [false, true] {
            for compute in computes(&mut rng) {
                if !only(ComputeOp::decode(u64::from(compute))) {
                    continue;
                }
                let cond = if rng.below(3) == 0 { rng.below(32) } else { 31 };
                let toggle = [0, SRRFL, SRRFH, SRRFL | SRRFH][rng.below(4) as usize];
                let parcels = t2a(cond as u16, compute);
                let setup = Setup {
                    parcels: &parcels,
                    cond,
                    seed: rng.next_u64(),
                    simd,
                    toggle,
                    last: false,
                };
                case(&mut t, &table, &setup);
            }
        }
    }
    t
}

#[test]
fn float_kinds_equal_interpreter_on_random_states() {
    let rounds = 90;
    let t = run(0xF10A_0001, rounds, |_| true);
    let per_mode = ALU_OPS.len() + 2;
    assert_eq!(t.cases, rounds * 2 * per_mode);
    // every kind ran in both modes, many times
    let kinds: Vec<u16> = t.per_kind.keys().map(|k| k.0).collect();
    for k in KINDS {
        for simd in [false, true] {
            let n = t.per_kind.get(&(k.0, simd)).copied().unwrap_or(0);
            assert!(
                n >= rounds,
                "kind {:#x} simd {simd}: {n} cases ({kinds:x?})",
                k.0
            );
        }
    }
    // the handlers wrote registers and hidden extensions, raised exceptions and handed over
    assert!(t.changed * 100 > t.cases * 80, "{t:?}");
    assert!(t.hidden_changed * 4 > t.cases, "{t:?}");
    assert!(t.sticky_raised * 20 > t.cases, "{t:?}");
    assert!(
        t.jumps * 50 > t.cases && t.oks > t.jumps && t.jumps == t.pokes,
        "{t:?}"
    );
    assert!(
        t.stale_read * 4 > t.cases,
        "stale extensions were in the state: {t:?}"
    );
    // all four (PEx, PEy) enable combinations occurred
    for combo in [(true, true), (true, false), (false, true), (false, false)] {
        assert!(
            t.enables.get(&combo).copied().unwrap_or(0) >= 20,
            "{combo:?}: {t:?}"
        );
    }
}

#[test]
fn fgen_ops_equal_interpreter() {
    // RECIPS, RSQRTS and the float multifunctions only, more rounds: `Pe::exec` is the v1 element
    // and is checked here against the interpreter, hidden extension included
    let t = run(0xF10A_0002, 260, fgen_supports);
    assert_eq!(
        t.per_kind.keys().map(|k| k.0).collect::<Vec<_>>(),
        [FGEN.0, FGEN.0]
    );
    assert!(
        t.cases >= 260 * 2 * 3 && t.changed * 100 > t.cases * 80,
        "{t:?}"
    );
    assert!(t.hidden_changed * 4 > t.cases, "{t:?}");
}

#[test]
fn a_last_uop_returns_ok_even_after_an_exception() {
    let table = crate::core::uop::hot::UopHot::handler_table();
    let mut t = Tally::default();
    for seed in 0..120 {
        let parcels = t2a(31, alu(0x81, 3, 1, 2));
        let setup = Setup {
            parcels: &parcels,
            cond: 31,
            seed,
            simd: seed % 2 == 0,
            toggle: 0,
            last: true,
        };
        case(&mut t, &table, &setup);
    }
    assert!(t.pokes > 10 && t.jumps == 0 && t.oks == 120, "{t:?}");
}

#[test]
fn every_condition_code_enables_each_element_like_the_interpreter() {
    let table = crate::core::uop::hot::UopHot::handler_table();
    let mut t = Tally::default();
    for cond in 0..32 {
        for (simd, seed) in (0..9u64).map(|s| (s != 0, s)) {
            let parcels = t2a(cond as u16, alu(0x81, 5, 6, 7));
            let setup = Setup {
                parcels: &parcels,
                cond,
                seed: seed * 97 + u64::from(cond),
                simd,
                toggle: 0,
                last: false,
            };
            case(&mut t, &table, &setup);
        }
    }
    assert_eq!(t.cases, 32 * 9);
    // a compute that was skipped on an element left its registers alone: both elements ran in
    // some cases, only one in others, none in others
    for combo in [(true, true), (true, false), (false, true), (false, false)] {
        assert!(
            t.enables.get(&combo).copied().unwrap_or(0) >= 8,
            "{combo:?}: {t:?}"
        );
    }
}

#[test]
fn a_bad_operand_makes_the_handler_bail_with_the_core_untouched() {
    let table = crate::core::uop::hot::UopHot::handler_table();
    let (fixed_add, ok_add) = (alu(0x01, 1, 2, 3), alu(0x81, 1, 2, 3));
    // (kind, `a`, why it is bad)
    let bad = [
        (FGEN, ok_add, "FADD is not an FGEN op"),
        (FGEN, 1 << 22 | 0x1234, "a fixed-point multifunction"),
        (FGEN, fixed_add, "a fixed-point ALU op"),
        (FSEL, pack(1, 2, 3, 4), "no such select"),
        (FABSNEG, pack(1, 2, 0, 3), "no such sign op"),
        (FCONV, pack(1, 2, 3, 0x81), "FADD is not a conversion"),
        (FCONV, pack(1, 2, 3, 0x100), "an opcode above 8 bits"),
    ];
    for (kind, a, why) in bad {
        for simd in [false, true] {
            let (mut x, y) = (
                prepare(&[0x0001], 7, simd, 0),
                prepare(&[0x0001], 7, simd, 0),
            );
            let mut uop = Uop::new(kind, 1, x.pc, a, 31);
            if simd {
                uop.flags |= crate::core::uop::types::UF_SIMD;
            }
            let mut ctx = UopCtx::new(&mut x as *mut Core, x.steps, 8);
            // SAFETY: `ctx.core` is `x`, unused until the handler returns
            let r = unsafe { table[usize::from(kind.0)](&mut ctx, &uop) };
            assert_eq!(Exit::from_u32(r), Some(Exit::Slow), "{why}");
            assert_eq!(ctx.done, 0, "{why}");
            assert_eq!(exact_diff(&x, &y), None, "{why}: the core changed");
        }
    }
}

#[test]
fn handlers_do_not_allocate() {
    use crate::core::uop::verify::count_alloc::allocs_in;
    let table = crate::core::uop::hot::UopHot::handler_table();
    let mut cores: Vec<(Core, Vec<Uop>)> = (0..12u64)
        .map(|seed| {
            let compute = [
                alu(0x81, 3, 1, 2),
                mul(3, 1, 2),
                alu(0xC4, 3, 1, 2),
                alu(0x89, 3, 1, 2),
            ][seed as usize % 4];
            let mut c = prepare(&t2a(31, compute), seed, seed % 2 == 0, 0);
            let uops = lowered(&mut c);
            (c, uops)
        })
        .collect();
    let (ran, allocs) = allocs_in(|| {
        let mut jumps = 0;
        for (c, uops) in &mut cores {
            jumps += usize::from(run_uop(c, &table, uops).exit == Exit::Jump);
        }
        jumps
    });
    assert_eq!(
        allocs, 0,
        "a float uop never allocates, the cold exception path included"
    );
    assert!(ran < 12);
}
