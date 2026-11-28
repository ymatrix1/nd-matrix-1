//! Test fixture of the driver and the invalidation code: hand-built all-`Interp` engines and the
//! interpreter-against-engine comparison.
//!
//! OWNER: W2-DRIVE. The engine under test is armed with an all-`Interp` lowering
//! ([`interp_lower`]), so its blocks equal the interpreter by construction and what a test can
//! find is a defect of the driver, the dispatch loop, the event rule or the invalidation, never of
//! a family. The fast families may land (or not) without changing a test that uses it. A result
//! that never ran a block is identical to the interpreter's by construction too (the vacuity the
//! W1-VERIFY header warns about), so [`Ran::exercised`] states the evidence a test must assert.
use crate::core::decode_cache::{CachedInsn, DecodedOp};
use crate::core::support::{core_with, CODE};
use crate::core::uop::config::{UopBuildReport, UopConfig, UopStats};
use crate::core::uop::hot::UopHot;
use crate::core::uop::lower::builder::BlockBuilder;
use crate::core::uop::lower::decode::{code_hash, decode_at};
use crate::core::uop::lower::LowerCtx;
use crate::core::uop::run::CodePages;
use crate::core::uop::types::{GuardClass, Handler, Uop, UopKind};
use crate::core::uop::verify::compare::{exact_diff, ExactDiff};
use crate::core::{Core, Fault};
use std::rc::Rc;

/// The lowering of every instruction as one `Interp` uop.
pub(crate) fn interp_lower(_cx: &LowerCtx<'_>, c: &CachedInsn, b: &mut BlockBuilder) {
    b.interp(c);
}

/// Arms `core` with all-`Interp` blocks over `ranges` (both PEy modes, like `enable_uop`).
pub(crate) fn arm_interp(core: &mut Core, ranges: &[(u32, u32)]) -> UopBuildReport {
    core.uop_arm_with(UopConfig::default(), ranges, interp_lower)
        .expect("ranges inside the code window")
}

/// The lowering that makes a `NOP` a fast `NOP` uop and everything else an `Interp` uop. Unlike the
/// all-`Interp` lowering it can be wrong: a block built from code that has since changed still
/// holds the old NOP uops, so a stale block shows in the state (the self-modifying-code tests need
/// that; an all-`Interp` block decodes the live words again and heals itself).
pub(crate) fn nop_lower(_cx: &LowerCtx<'_>, c: &CachedInsn, b: &mut BlockBuilder) {
    nop_or_interp(c, b);
}

/// [`nop_lower`] without the translator context.
pub(crate) fn nop_or_interp(c: &CachedInsn, b: &mut BlockBuilder) {
    if matches!(c.op, DecodedOp::Nop) {
        b.push(Uop::new(UopKind::NOP, 1, c.pc, 0, c.next_pc));
        b.set_end(c.next_pc);
    } else {
        b.interp(c);
    }
}

/// Arms `core` with [`nop_lower`] blocks over `ranges`.
pub(crate) fn arm_nop_fast(core: &mut Core, ranges: &[(u32, u32)]) -> UopBuildReport {
    core.uop_arm_with(UopConfig::default(), ranges, nop_lower)
        .expect("ranges inside the code window")
}

/// Arms `core` with exactly one block over `[lo, hi)` (PEy off) that the cuts of the translator do
/// not get to shape: every instruction goes through `lower`, and `handlers` are entered in the
/// table (a test kind that fakes a handler). It is the block a family could build when it lets a
/// store or a system-register write sit in the middle of a block.
pub(crate) fn arm_one_block(
    core: &mut Core,
    (lo, hi): (u32, u32),
    lower: impl Fn(&CachedInsn, &mut BlockBuilder),
    handlers: &[(UopKind, Handler)],
) {
    let mut b = BlockBuilder::new(lo, false, false);
    let mut pc = lo;
    while pc < hi {
        let c = decode_at(core, pc);
        lower(&c, &mut b);
        pc = c.next_pc;
    }
    let (mut block, uops) = b.finish(GuardClass::Plain);
    block.code_hash = code_hash(core, block.pc, block.end_pc);
    let mut hot = UopHot::new();
    for (kind, h) in handlers {
        hot.handlers[usize::from(kind.0)] = *h;
    }
    hot.install(vec![block], uops);
    core.uop_run.code_pages.set_range(lo, hi);
    core.uop_run.active = true;
    core.uop = Some(Box::new(hot));
}

/// [`arm_one_block`] with every instruction an `Interp` uop.
pub(crate) fn arm_one_interp_block(core: &mut Core, range: (u32, u32)) {
    arm_one_block(core, range, |c, b| b.interp(c), &[]);
}

/// A core with `n` NOPs from [`CODE`] (and two spare parcels of NOP), no event pending.
pub(crate) fn nop_core(n: usize) -> Core {
    let mut c = core_with(&vec![0x0001; n + 2]);
    c.irq.due = u64::MAX;
    c
}

/// An engine state holding one all-`Interp` block of `n` NOPs at [`CODE`] (block 0, PEy off,
/// `code_hash` set like the translator does) and a core with the code for it.
pub(crate) fn interp_block(n: u32) -> (UopHot, Core) {
    let mut core = nop_core(n as usize);
    let mut b = BlockBuilder::new(CODE, false, false);
    let mut pc = CODE;
    for _ in 0..n {
        let c = decode_at(&mut core, pc);
        b.interp(&c);
        pc = c.next_pc;
    }
    let (mut block, uops) = b.finish(GuardClass::Plain);
    block.code_hash = code_hash(&core, block.pc, block.end_pc);
    let mut hot = UopHot::new();
    hot.install(vec![block], uops);
    (hot, core)
}

/// A change applied to every core a [`Setup`] makes.
type Tweak = Rc<dyn Fn(&mut Core)>;

/// One scenario: a program at [`CODE`] plus whatever the closures set up on a fresh core.
pub(crate) struct Setup {
    program: Vec<u16>,
    tweaks: Vec<Tweak>,
    ranges: Option<Vec<(u32, u32)>>,
    stop_pc: Option<u32>,
}

impl Setup {
    /// `n` NOPs.
    pub fn nops(n: usize) -> Setup {
        Setup::program(&vec![0x0001; n])
    }

    /// The parcels of `program`.
    pub fn program(program: &[u16]) -> Setup {
        Setup {
            program: program.to_vec(),
            tweaks: Vec::new(),
            ranges: None,
            stop_pc: None,
        }
    }

    /// Adds a change applied to every core this setup makes.
    pub fn with(mut self, f: impl Fn(&mut Core) + 'static) -> Setup {
        self.tweaks.push(Rc::new(f));
        self
    }

    /// The short-word ranges to translate (default: the whole program).
    pub fn ranges(mut self, ranges: &[(u32, u32)]) -> Setup {
        self.ranges = Some(ranges.to_vec());
        self
    }

    /// The stop pc set before arming.
    pub fn stop_pc(mut self, pc: u32) -> Setup {
        self.stop_pc = Some(pc);
        self
    }

    /// A core running the scenario through the interpreter.
    pub fn core(&self) -> Core {
        let mut c = core_with(&self.program);
        c.irq.due = u64::MAX;
        for t in &self.tweaks {
            t(&mut c);
        }
        c
    }

    /// The same core armed with the all-`Interp` engine.
    pub fn armed(&self) -> Core {
        let mut c = self.core();
        c.uop_set_stop_pc(self.stop_pc);
        let whole = [(CODE, CODE + self.program.len() as u32)];
        arm_interp(&mut c, self.ranges.as_deref().unwrap_or(&whole));
        c
    }
}

/// What [`run_both`] found.
#[derive(Debug)]
pub(crate) struct Ran {
    /// `None` when the two cores (and their faults) are identical.
    pub diff: Option<ExactDiff>,
    /// Interrupts the engine core took (the interpreter core took the same number if `diff` is
    /// `None`).
    pub taken: u64,
    /// The engine's counters.
    pub stats: UopStats,
}

impl Ran {
    /// Evidence that blocks ran: steps went through uops and every one of them through `Interp`.
    pub fn exercised(&self) -> bool {
        self.stats.steps_uop > 0 && self.stats.steps_uop == self.stats.steps_interp_uop
    }
}

/// Runs `steps` steps of the scenario under `Core::run` and under `Core::run_uop` and compares.
pub(crate) fn run_both(setup: &Setup, steps: u64) -> Ran {
    let (mut want, mut got) = (setup.core(), setup.armed());
    let (fw, fg): (Option<Fault>, Option<Fault>) = (want.run(steps), got.run_uop(steps));
    let diff = if fw != fg {
        Some(ExactDiff {
            field: "fault",
            a: format!("{fw:?}"),
            b: format!("{fg:?}"),
        })
    } else {
        exact_diff(&want, &got)
    };
    Ran {
        diff,
        taken: got.interrupts_taken(),
        stats: got.uop_stats(),
    }
}

#[test]
fn the_hand_built_block_is_one_block_without_the_cuts_of_the_translator() {
    // `IRPTL = 0` writes a system register: the translator cuts after it, the hand-built block
    // does not
    let program = [0x0001, 0x0ffa, 0x0000, 0x0001, 0x0001];
    let mut cut = core_with(&program);
    arm_interp(&mut cut, &[(CODE, CODE + 5)]);
    assert!(
        cut.uop.as_ref().unwrap().blocks.len() > 2,
        "cut after the register write"
    );
    let mut one = core_with(&program);
    arm_one_interp_block(&mut one, (CODE, CODE + 5));
    let hot = one.uop.as_ref().unwrap();
    assert_eq!(
        (hot.blocks.len(), hot.blocks[0].steps, hot.blocks[0].end_pc),
        (1, 4, CODE + 5)
    );
    assert!(
        one.uop_enabled()
            && one
                .uop_run
                .code_pages
                .test(CodePages::page_of(CODE).unwrap())
    );
    // the NOP-fast lowering keeps NOPs as NOP uops and interprets the rest
    let mut n = core_with(&[0x0001, 0xc8be, 0x0001]);
    arm_nop_fast(&mut n, &[(CODE, CODE + 3)]);
    let hot = n.uop.as_ref().unwrap();
    let kinds: Vec<u16> = hot.arena[..3].iter().map(|u| u.h).collect();
    assert_eq!(kinds, [UopKind::NOP.0, UopKind::INTERP.0, UopKind::NOP.0]);
}

#[test]
fn the_fixture_arms_what_it_says_and_compares_what_it_runs() {
    let (hot, core) = interp_block(5);
    assert_eq!(
        (hot.blocks.len(), hot.blocks[0].steps, hot.blocks[0].end_pc),
        (1, 5, CODE + 5)
    );
    assert_eq!(hot.blocks[0].code_hash, code_hash(&core, CODE, CODE + 5));
    assert_ne!(hot.blocks[0].code_hash, 0);
    let s = Setup::nops(8).with(|c| c.r[3] = 7).stop_pc(CODE + 4);
    assert_eq!((s.core().r[3], s.armed().r[3]), (7, 7));
    let a = s.armed();
    assert!(a.uop_enabled());
    assert_eq!(
        a.uop
            .as_ref()
            .unwrap()
            .pcmap(false)
            .get(CODE + 4)
            .map(|e| e.1),
        Some(0)
    );
    let r = run_both(&s, 6);
    assert_eq!(r.diff, None, "{r:?}");
    assert!(r.exercised(), "{r:?}");
    // a scenario the engine cannot have run is not "exercised"
    let none = Setup::nops(8).ranges(&[(CODE + 20, CODE + 24)]);
    let r = run_both(&none, 6);
    assert_eq!(r.diff, None);
    assert!(
        !r.exercised(),
        "no block covers the code: nothing ran through uops"
    );
}
