//! One fuzz case end to end: two identical cores, the interpreter on one and the armed engine on
//! the other, run through the same plan and compared after every slice.
//!
//! OWNER: W3-FUZZ. A [`Scenario`] is everything a failure needs to reproduce: the seed, the
//! program, the state fill, the plan and the interrupts. [`check`] panics with the whole scenario
//! (name, seed, program as hex, first differing field) when the engine differs from the
//! interpreter or when the case did not exercise a fast uop (design P10: a case in which nothing
//! was lowered or the executor declined every block equals the interpreter by construction).
//!
//! Inherited requirements, both honoured here: the access log is enabled on BOTH cores before
//! anything is written (a core with peripheral models is compared through its census), and
//! programs that move float registers get the stale-extension fill (`Prep::stale`), because
//! `exact_diff` compares the raw hidden extension bytes.
use super::ran::{fault_diff, Ran};
use super::rig::Rig;
use super::state::{aim_at, arm_interrupts, prepare, Prep, ISR_PC};
use crate::core::uop::config::{UopConfig, UopError, UopStats};
use crate::core::uop::types::{Handler, UopKind};
use crate::core::uop::verify::compare::{exact_diff, ExactDiff};
use crate::core::uop::verify::fuzz::DiffRun;
use crate::core::uop::verify::gen::{core_with_code, CODE_PC};
use crate::core::uop::verify::mutate::{perturb, Mutant};
use crate::core::Core;

/// One slice of a plan.
#[derive(Clone, Debug)]
pub(super) enum Step {
    /// Run this many steps on both cores (`Core::run` and `Core::run_uop`) and compare.
    Run(u64),
    /// A host write of one normal word (`Core::dm_write`: the hook that invalidates translated
    /// code), on both cores.
    Poke { word: u32, value: u32 },
    /// An overlay load: parcels written straight into the SRAM at a short-word pc, then
    /// `invalidate_code_cache` on both cores (the path of `isa.rs`).
    Overlay { pc: u32, parcels: Vec<u16> },
    /// A pass boundary on the engine core: `uop_maintain` (the interpreter has nothing to do).
    Maintain,
    /// The same through a synchronous translation service pumped on the calling thread.
    MaintainAsync,
}

/// A register a case pins to a value after the state fill (the aim of a store, the value it
/// stores).
#[derive(Clone, Copy, Debug)]
pub(super) enum Reg {
    R(usize),
    I(usize),
    M(usize),
    L(usize),
}

/// Interrupts of a case: `(cycle, vector)` pairs scheduled on both cores, and the service
/// routine they all share (placed at [`ISR_PC`], ending in whatever the caller wrote).
#[derive(Clone, Debug)]
pub(super) struct Irq {
    pub at: Vec<(u64, u8)>,
    pub isr: Vec<u16>,
}

/// One differential case.
#[derive(Clone, Debug)]
pub(super) struct Scenario {
    pub name: &'static str,
    pub seed: u64,
    pub program: Vec<u16>,
    pub prep: Prep,
    pub cfg: UopConfig,
    pub plan: Vec<Step>,
    pub irq: Option<Irq>,
    /// The pass idle pc handed to the translator (a block ends there).
    pub stop_pc: Option<u32>,
    /// Enable the access log on both cores (on by default).
    pub log: bool,
    /// A start state in which the fast handlers bail to `Core::step` by design (multiplier edges
    /// and sticky bits make most ops raise an exception, and a handler that would poke the
    /// controller declines the uop): such a case is not asked to run fast uops on its own, the
    /// sweep asks it of half of them.
    pub heavy: bool,
    /// Set `irq.due` to its far value before the run, as the driver fixtures do. Left alone
    /// (`false`) the first step of a fresh core runs through `Core::step`, which asks the
    /// controller; a case with interrupts keeps it so.
    pub due_far: bool,
    /// A wrong handler planted in the engine core: the handler of the first fast uop kind of the
    /// program is wrapped by `mutate::perturb` (the test of the tests, `mutants.rs`).
    pub mutant: Option<Mutant>,
    /// Which distinct fast kind of the program holds the planted handler (modulo their number).
    pub nth: usize,
    /// Translate on a worker thread instead of the calling thread (`MaintainAsync` rounds).
    pub threaded: bool,
    /// Registers pinned after the state fill (before the interrupts are armed).
    pub set: Vec<(Reg, u32)>,
    /// Instruction starts the indirect jumps (DAG2) and the call stack are aimed at, see
    /// `state::aim_at`; empty leaves them as the state fill made them.
    pub targets: Vec<u32>,
}

impl Scenario {
    /// A case that runs `steps` steps of `program` on the state of `seed`.
    pub fn new(name: &'static str, seed: u64, program: Vec<u16>, steps: u64) -> Scenario {
        Scenario {
            name,
            seed,
            program,
            prep: Prep::default(),
            cfg: UopConfig::default(),
            plan: vec![Step::Run(steps)],
            irq: None,
            stop_pc: None,
            log: true,
            heavy: false,
            due_far: true,
            mutant: None,
            nth: 0,
            threaded: false,
            set: Vec::new(),
            targets: Vec::new(),
        }
    }

    /// The code image: the program, then (with interrupts) padding up to [`ISR_PC`] and the
    /// service routine.
    pub fn image(&self) -> Vec<u16> {
        let mut image = self.program.clone();
        if let Some(irq) = &self.irq {
            let at = (ISR_PC - CODE_PC) as usize;
            assert!(
                image.len() <= at,
                "{}: the program overlaps the service routine",
                self.name
            );
            image.resize(at, 0x0001);
            image.extend(&irq.isr);
        }
        image
    }

    /// The short-word ranges the engine translates.
    pub fn ranges(&self) -> Vec<(u32, u32)> {
        let mut r = vec![(CODE_PC, CODE_PC + self.program.len() as u32)];
        if let Some(irq) = &self.irq {
            r.push((ISR_PC, ISR_PC + irq.isr.len() as u32));
        }
        r
    }

    /// A fresh core in the start state of this case (the interpreter and the engine core are both
    /// built by this function, so they start identical).
    pub fn core(&self) -> Core {
        let mut c = core_with_code(&self.image());
        if self.log {
            c.enable_access_log();
        }
        prepare(&mut c, self.seed, &self.prep);
        if !self.targets.is_empty() {
            aim_at(&mut c, self.seed, &self.targets);
        }
        for &(reg, v) in &self.set {
            match reg {
                Reg::R(k) => c.r[k] = v,
                Reg::I(k) => c.i[k] = v,
                Reg::M(k) => c.m[k] = v,
                Reg::L(k) => c.l[k] = v,
            }
        }
        if self.due_far && self.irq.is_none() {
            c.irq.due = u64::MAX;
        }
        if let Some(irq) = &self.irq {
            // the arithmetic exception vectors are enabled only in the case that asks for them
            arm_interrupts(&mut c, irq.at.is_empty());
            for &(cycle, vector) in &irq.at {
                c.schedule_interrupt(cycle, vector);
            }
        }
        c
    }

    /// A description that reproduces the case: name, seed, state fill, plan and the program.
    pub fn describe(&self) -> String {
        let hex = |p: &[u16]| {
            p.iter()
                .map(|x| format!("{x:04x}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        format!(
            "{} seed={:#x} prep={:?} cfg.max_block_steps={} plan={:?} targets={:x?} set={:x?} \
             due_far={} log={} irq={:?}\n  program[{}]: {}",
            self.name,
            self.seed,
            self.prep,
            self.cfg.max_block_steps,
            self.plan,
            self.targets,
            self.set,
            self.due_far,
            self.log,
            self.irq.as_ref().map(|i| (&i.at, hex(&i.isr))),
            self.program.len(),
            hex(&self.program),
        )
    }
}

/// Wraps the handler of the `nth` (modulo their number) distinct fast uop kinds that `got` holds
/// in `m`. Returns the kind, or `None` when every uop is a `NOP` or `Interp` uop. Kind 0 is the
/// first in program order.
pub(super) fn plant(got: &mut Core, m: Mutant, nth: usize) -> Option<UopKind> {
    plant_with(got, nth, |h| perturb(h, m))
}

/// [`plant`] with any wrapper: replaces the handler of the chosen kind by `wrap(handler)`.
pub(super) fn plant_with(
    got: &mut Core,
    nth: usize,
    wrap: impl FnOnce(Handler) -> Handler,
) -> Option<UopKind> {
    let hot = got.uop.as_mut()?;
    let mut kinds: Vec<UopKind> = Vec::new();
    for u in hot.arena.iter().map(|u| UopKind(u.h)) {
        if u != UopKind::NOP && u != UopKind::INTERP && u != UopKind::FIN && !kinds.contains(&u) {
            kinds.push(u);
        }
    }
    let kind = *kinds.get(nth.checked_rem(kinds.len())?)?;
    let at = usize::from(kind.0);
    hot.handlers[at] = wrap(hot.handlers[at]);
    Some(kind)
}

/// Runs the plan on both cores; stops at the first difference.
pub(super) fn run(s: &Scenario) -> Result<Ran, UopError> {
    let (mut want, mut got) = (s.core(), s.core());
    got.uop_set_stop_pc(s.stop_pc);
    let build = got.enable_uop(s.cfg.clone(), &s.ranges())?;
    if let Some(m) = s.mutant {
        plant(&mut got, m, s.nth).expect("a fast uop to plant the mutant in");
    }
    let mut rig = Rig::new(s);
    let mut diff = None;
    for step in &s.plan {
        match step {
            Step::Run(n) => {
                diff = fault_diff(want.run(*n), got.run_uop(*n));
            }
            Step::Poke { word, value } => {
                let (a, b) = (want.dm_write(*word, *value), got.dm_write(*word, *value));
                diff = fault_diff(a.err(), b.err());
            }
            Step::Overlay { pc, parcels } => {
                for c in [&mut want, &mut got] {
                    for (k, p) in parcels.iter().enumerate() {
                        let o = (*pc as usize + k) * 2;
                        c.sram[o..o + 2].copy_from_slice(&p.to_le_bytes());
                    }
                    c.invalidate_code_cache();
                }
            }
            Step::Maintain => {
                got.uop_maintain();
            }
            Step::MaintainAsync => {
                rig.round(&mut got, &s.cfg);
            }
        }
        diff = diff.or_else(|| exact_diff(&want, &got));
        if diff.is_some() {
            break;
        }
    }
    let stats: UopStats = got.uop_stats();
    Ok(Ran {
        run: DiffRun { diff, build, stats },
        taken: got.interrupts_taken(),
        installed: rig.installed(),
        fast_after_install: rig.fast_after_install(&got),
    })
}

/// Runs `s` and panics with the reproduction when the engine differs from the interpreter or the
/// case exercised nothing. Returns the evidence for tests that assert more.
#[track_caller]
pub(super) fn check(s: &Scenario) -> Ran {
    let ran = run(s).unwrap_or_else(|e| panic!("cannot arm the engine: {e}\n{}", s.describe()));
    if let Some(d) = &ran.run.diff {
        panic!(
            "DIVERGENCE first field `{}`: {d}\n{}",
            d.field,
            s.describe()
        );
    }
    assert!(
        ran.exercised(),
        "vacuous case, no fast uop ran: {:?} {:?}\n{}",
        ran.run.build,
        ran.run.stats,
        s.describe()
    );
    ran
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::verify::gen::{random_program, Rng};

    /// `R3 = R1 + R2` (float add), the one fast instruction of the small programs below.
    const FADD: [u16; 3] = [0x013e, 0x0000, 0x1312];

    fn adds(n: usize) -> Vec<u16> {
        FADD.repeat(n)
    }

    #[test]
    fn a_straight_program_of_fast_uops_runs_exercised_and_equal() {
        let ran = check(&Scenario::new("t", 3, adds(6), 6));
        assert!(ran.run.stats.steps_uop >= 6 && ran.run.build.insns > ran.run.build.interp_insns);
    }

    #[test]
    fn the_interpreter_and_the_engine_start_from_one_state_with_the_access_log_on() {
        let mut s = Scenario::new("t", 11, adds(2), 2);
        s.prep = Prep::full(true);
        let (a, b) = (s.core(), s.core());
        assert_eq!(exact_diff(&a, &b), None);
        assert!(a.periph.is_some() && b.periph.is_some());
        s.log = false;
        assert!(s.core().periph.is_none());
    }

    #[test]
    #[should_panic(expected = "vacuous case")]
    fn a_case_that_runs_no_fast_uop_is_refused() {
        check(&Scenario::new("nops", 1, vec![0x0001; 4], 4));
    }

    #[test]
    fn the_description_names_the_seed_the_plan_and_the_program_in_hex() {
        let s = Scenario::new("named", 0xAB, vec![0x013e, 0x0000, 0x1312], 3);
        let d = s.describe();
        assert!(d.contains("named seed=0xab") && d.contains("Run(3)"), "{d}");
        assert!(d.contains("013e 0000 1312"), "{d}");
    }

    #[test]
    fn interrupts_add_a_second_range_and_the_service_routine_sits_at_its_address() {
        let mut s = Scenario::new("t", 1, adds(2), 6);
        s.irq = Some(Irq {
            at: vec![(3, 11)],
            isr: vec![0x0001, 0x0a80 | 0x100 | 31 << 1],
        });
        let image = s.image();
        assert_eq!(image.len(), (ISR_PC - CODE_PC) as usize + 2);
        assert_eq!(s.ranges(), [(CODE_PC, CODE_PC + 6), (ISR_PC, ISR_PC + 2)]);
        let c = s.core();
        assert_eq!(c.parcel_at(ISR_PC + 1), 0x0a80 | 0x100 | 31 << 1);
        assert_eq!(c.irq.scheduled, [(3, 11)]);
    }

    #[test]
    fn the_plan_steps_poke_overlay_and_maintain_reach_both_cores() {
        let mut s = Scenario::new("t", 5, adds(8), 3);
        s.plan = vec![
            Step::Run(3),
            Step::Poke {
                word: CODE_PC / 2 + 6,
                value: 0x0001_0001,
            },
            Step::Maintain,
            Step::Run(2),
            Step::Overlay {
                pc: CODE_PC + 12,
                parcels: vec![0x0001; 3],
            },
            Step::MaintainAsync,
            Step::Run(3),
        ];
        let ran = check(&s);
        assert!(ran.run.stats.maintain_calls >= 1, "{:?}", ran.run.stats);
        // a random program of the shared pool also runs through `run`
        let mut r = Rng::new(2);
        let p = random_program(&mut r, 5);
        assert!(run(&Scenario::new("pool", 2, p, 5))
            .unwrap()
            .run
            .diff
            .is_none());
    }
}
