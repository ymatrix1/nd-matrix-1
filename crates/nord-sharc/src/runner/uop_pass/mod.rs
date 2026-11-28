//! The pass loop with the uop engine: [`lean_pass_uop`] and its checked twin. The arming of a
//! booted core (which code to translate, the stop pc, engine names) is in [`arm`] and
//! re-exported here.
//!
//! OWNER: W1-RUN. Same contract as `lean_pass` (`system.rs`) and `lean_pass_aot`
//! (`pass_sched.rs`): trigger the pass, run to the poll loop with the mailbox cleared, return
//! `(steps, reached the idle loop)`. On a core without `Core::enable_uop` every `uop_try`
//! returns false and the loop is the interpreter's. After the pass, a program change or overlay
//! load (`uop_dirty() != 0`) is followed by `uop_maintain`, which may allocate; a steady pass
//! never reaches it (gate G2, `maintain_calls == 0`).
mod arm;
mod probe;

pub use arm::{
    arm_for, arm_uop, arm_uop_ranges, code_ranges, engine_named, uop_config, CODE_WINDOW,
    RANGE_GAP, RANGE_TAIL,
};
pub use probe::{pc_ranges, probe_ranges, ranges_for, RangeMode};

use super::pass::PassConfig;
use super::system::start;
use crate::core::{shadow_run_with, Core, ExactDiff, Shadow};

/// The pass loop proper, after the trigger: block hook first, `Core::step` for the gaps.
fn uop_body(core: &mut Core, cfg: &PassConfig) -> (u64, bool) {
    let begin = core.steps;
    let limit = begin + cfg.max_steps;
    while core.steps < limit {
        if core.uop_try(limit) {
            if core.uop_take_fault().is_some() {
                return (core.steps - begin, false);
            }
        } else if core.step().is_err() {
            return (core.steps - begin, false);
        }
        if core.pc == cfg.idle_pc && core.dm_read(cfg.trigger) == Ok(0) {
            if core.uop_needs_maintenance() {
                core.uop_maintain();
            }
            return (core.steps - begin, true);
        }
    }
    (core.steps - begin, false)
}

/// The interpreter's loop after the trigger (what `lean_pass` does once `start` ran).
fn interp_body(core: &mut Core, cfg: &PassConfig) -> (u64, bool) {
    let begin = core.steps;
    while core.steps - begin < cfg.max_steps {
        if core.step().is_err() {
            return (core.steps - begin, false);
        }
        if core.pc == cfg.idle_pc && core.dm_read(cfg.trigger) == Ok(0) {
            return (core.steps - begin, true);
        }
    }
    (core.steps - begin, false)
}

/// [`super::system::lean_pass`] with the uop engine: translated blocks run whole, the interpreter
/// fills the gaps.
pub fn lean_pass_uop(core: &mut Core, cfg: &PassConfig) -> (u64, bool) {
    start(core, cfg);
    core.uop_begin_pass(cfg.idle_pc);
    uop_body(core, cfg)
}

/// The checked pass around any engine closure: `engine` runs the pass after the trigger, the core
/// is rewound to that state, the interpreter runs the same pass and the two end states are
/// compared. The core is left in the interpreter's state. Split out of
/// [`lean_pass_uop_shadowed`] so a test can hand it a deliberately wrong engine.
fn checked_pass(
    core: &mut Core,
    cfg: &PassConfig,
    engine: impl FnOnce(&mut Core) -> (u64, bool),
) -> (u64, bool, Shadow) {
    let mut result = (0, false);
    let verdict = shadow_run_with(
        core,
        |c| result = engine(c),
        |c, _| {
            let _ = interp_body(c, cfg);
        },
    );
    (result.0, result.1, verdict)
}

/// [`lean_pass_uop`] checked against the interpreter, with the verdict of the check: the pass runs
/// under the engine, the core is rewound to the state after the trigger, the interpreter runs the
/// same pass and the two end states are compared with `exact_diff`. The core is left in the
/// interpreter's state, so a wrong engine heals. Allocates (two memory images and a second core);
/// tools and tests only. A core with peripheral models cannot be rewound: there the pass is
/// unchecked and the verdict is [`Shadow::Skipped`], which a caller that reports "no difference"
/// must not read as [`Shadow::Match`].
pub fn lean_pass_uop_shadowed(core: &mut Core, cfg: &PassConfig) -> (u64, bool, Shadow) {
    start(core, cfg);
    core.uop_begin_pass(cfg.idle_pc);
    checked_pass(core, cfg, |c| uop_body(c, cfg))
}

/// [`lean_pass_uop_shadowed`] with the difference, if any: `None` for a match and for a pass that
/// could not be checked (use the shadowed form to tell the two apart).
pub fn lean_pass_uop_checked(core: &mut Core, cfg: &PassConfig) -> (u64, bool, Option<ExactDiff>) {
    let (steps, idle, verdict) = lean_pass_uop_shadowed(core, cfg);
    match verdict {
        Shadow::Mismatch(d) => (steps, idle, Some(d)),
        Shadow::Match | Shadow::Skipped => (steps, idle, None),
    }
}

#[cfg(test)]
mod tests {
    use super::super::system::lean_pass;
    use super::*;
    use crate::core::support::core_with;
    use crate::core::UopConfig;

    const CODE: u32 = 0x12_0000;

    fn cfg() -> PassConfig {
        PassConfig {
            trigger: 0xB_00E2,
            idle_pc: 0x12_0000,
            max_steps: 500,
            irq: None,
        }
    }

    /// 600 NOPs: a pass of 500 steps ends on the step limit without leaving the code.
    fn core() -> Core {
        core_with(&[0x0001; 600])
    }

    /// Mailbox word the poll program reads: a scripted device word that always reads 0, so the
    /// pass ends whenever the pc is back on the poll instruction.
    const POLL_TRIGGER: u32 = 0x3000_0000;
    /// The poll instruction `IF TRUE JUMP CODE`, after ten NOPs from `CODE`. A pass is the jump
    /// and the ten NOPs, 11 steps, and ends back on the poll instruction.
    const POLL_PC: u32 = CODE + 10;

    fn poll_cfg() -> PassConfig {
        PassConfig {
            trigger: POLL_TRIGGER,
            idle_pc: POLL_PC,
            max_steps: 100,
            irq: None,
        }
    }

    /// A core idling at [`POLL_PC`]: ten NOPs from [`CODE`], then `IF TRUE JUMP CODE`.
    fn poll_core() -> Core {
        let mut c = core_with(&[0x0001; 10]);
        for (k, p) in [0x063e_u16, 0x0012, 0x0000].iter().enumerate() {
            c.dm_write(POLL_PC + k as u32, u32::from(*p)).unwrap();
        }
        c.preset_device(POLL_TRIGGER, 0);
        c.pc = POLL_PC;
        c
    }

    #[test]
    fn stub_pass_equals_interpreter_pass() {
        let (mut want, mut got) = (core(), core());
        let w = lean_pass(&mut want, &cfg());
        assert_eq!(lean_pass_uop(&mut got, &cfg()), w, "engine never enabled");
        assert_eq!(want.state_mismatch(&got), None);
        // armed on the code, the skeleton translates nothing and the pass is still the interpreter's
        let mut armed = core();
        armed
            .enable_uop(UopConfig::default(), &[(0x12_0000, 0x12_0258)])
            .unwrap();
        assert_eq!(lean_pass_uop(&mut armed, &cfg()), w);
        assert_eq!(want.state_mismatch(&armed), None);
        assert_eq!(armed.uop_stats().passes, 1);
        assert_eq!(
            armed.uop_stats().steps_uop + armed.uop_stats().steps_core_step,
            w.0,
            "every step of the pass is accounted once, by an engine block or by Core::step"
        );
    }

    #[test]
    fn the_checked_pass_finds_no_difference_and_leaves_the_interpreter_state() {
        let (mut want, mut got) = (core(), core());
        let w = lean_pass(&mut want, &cfg());
        got.enable_uop(UopConfig::default(), &[(0x12_0000, 0x12_0258)])
            .unwrap();
        assert_eq!(lean_pass_uop_checked(&mut got, &cfg()), (w.0, w.1, None));
        assert_eq!(want.state_mismatch(&got), None);
    }

    #[test]
    fn a_steady_pass_never_calls_maintain() {
        let mut c = core();
        c.enable_uop(UopConfig::default(), &[(0x12_0000, 0x12_0258)])
            .unwrap();
        lean_pass_uop(&mut c, &cfg());
        assert_eq!(
            (c.uop_stats().maintain_calls, c.uop_needs_maintenance()),
            (0, false)
        );
    }

    #[test]
    fn the_poll_program_ends_every_pass_on_the_poll_instruction() {
        let mut want = poll_core();
        assert_eq!(lean_pass(&mut want, &poll_cfg()), (11, true));
        assert_eq!((want.pc, want.steps), (POLL_PC, 11));
    }

    #[test]
    fn passes_that_reach_the_poll_loop_end_where_the_interpreter_pass_ends() {
        let (mut want, mut plain, mut armed, mut checked) =
            (poll_core(), poll_core(), poll_core(), poll_core());
        let report = arm_uop(&mut armed, UopConfig::default(), POLL_PC).unwrap();
        let report_checked = arm_uop(&mut checked, UopConfig::default(), POLL_PC).unwrap();
        assert_eq!(report, report_checked);
        for pass in 0..4 {
            let w = lean_pass(&mut want, &poll_cfg());
            assert_eq!(w, (11, true), "pass {pass}");
            assert_eq!(
                lean_pass_uop(&mut plain, &poll_cfg()),
                w,
                "off, pass {pass}"
            );
            assert_eq!(lean_pass_uop(&mut armed, &poll_cfg()), w, "on, pass {pass}");
            assert_eq!(
                lean_pass_uop_checked(&mut checked, &poll_cfg()),
                (w.0, w.1, None),
                "checked, pass {pass}"
            );
            for (name, c) in [("off", &plain), ("on", &armed), ("checked", &checked)] {
                assert_eq!(want.state_mismatch(c), None, "{name}, pass {pass}");
                assert_eq!(c.pc, POLL_PC, "{name}, pass {pass}");
            }
        }
        let s = armed.uop_stats();
        assert_eq!((s.passes, s.maintain_calls, s.smc), (4, 0, 0));
        assert_eq!(
            s.steps_uop + s.steps_core_step,
            44,
            "4 passes of 11 steps, each step accounted once"
        );
        // a translated poll program must have run through blocks: the poll instruction is a
        // block start (the stop pc) and the NOP run ends in front of it
        assert!(
            report.blocks > 0,
            "the poll program is translated: {report:?}"
        );
        assert!(s.blocks_run > 0 && s.steps_uop > 0, "{s:?}");
        assert_eq!(
            plain.uop_stats().passes,
            0,
            "an engine that was never armed counts nothing"
        );
    }

    #[test]
    fn a_pass_stops_on_the_step_limit_like_the_interpreter() {
        let mut limited = poll_cfg();
        limited.max_steps = 7; // shorter than the 11 steps of a pass
        let (mut want, mut armed) = (poll_core(), poll_core());
        arm_uop(&mut armed, UopConfig::default(), POLL_PC).unwrap();
        let w = lean_pass(&mut want, &limited);
        assert_eq!(w, (7, false));
        assert_eq!(lean_pass_uop(&mut armed, &limited), w);
        assert_eq!(want.state_mismatch(&armed), None);
        assert_eq!(
            armed.uop_stats().steps_uop + armed.uop_stats().steps_core_step,
            7
        );
    }

    #[test]
    fn a_faulting_instruction_ends_the_pass_not_idle_like_the_interpreter() {
        // two NOPs, then a parcel sequence the oracle does not model (`Core::step` returns an error
        // and leaves the state untouched, `core/exec.rs` `faulting_step_leaves_state_untouched`)
        let make = || {
            let mut c = core_with(&[0x0001, 0x0001, 0xd70a, 0x3c23, 0xd70a, 0x0001]);
            c.preset_device(POLL_TRIGGER, 0);
            c
        };
        let cfg = PassConfig {
            trigger: POLL_TRIGGER,
            idle_pc: CODE + 5,
            max_steps: 100,
            irq: None,
        };
        let (mut want, mut got) = (make(), make());
        got.enable_uop(UopConfig::default(), &[(CODE, CODE + 6)])
            .unwrap();
        let w = lean_pass(&mut want, &cfg);
        assert_eq!(w, (2, false), "two NOPs, then the fault");
        assert_eq!(lean_pass_uop(&mut got, &cfg), w);
        assert_eq!(want.state_mismatch(&got), None);
    }

    #[test]
    fn the_checked_wrapper_reports_a_wrong_engine_and_heals_to_the_interpreter() {
        let cfg = poll_cfg();
        // the engine runs the pass correctly, then breaks one register
        let mut c = poll_core();
        start(&mut c, &cfg);
        let (steps, idle, verdict) = checked_pass(&mut c, &cfg, |c| {
            let r = uop_body(c, &cfg);
            c.r[3] ^= 0x10;
            r
        });
        assert_eq!((steps, idle), (11, true));
        let Shadow::Mismatch(d) = verdict else {
            panic!("a corrupted register is a difference: {verdict:?}");
        };
        assert_eq!(d.field, "r");
        let mut want = poll_core();
        lean_pass(&mut want, &cfg);
        assert_eq!(
            want.state_mismatch(&c),
            None,
            "healed to the interpreter state"
        );
        // a stray memory write
        let mut c = poll_core();
        start(&mut c, &cfg);
        let (_, _, verdict) = checked_pass(&mut c, &cfg, |c| {
            let r = uop_body(c, &cfg);
            c.dm_write(0xA_0000, 0x55).unwrap();
            r
        });
        assert!(
            matches!(verdict, Shadow::Mismatch(d) if d.field == "sram"),
            "memory is compared too"
        );
        assert_eq!(c.dm_read(0xA_0000), Ok(0), "and undone");
        // an engine that steps one instruction too far
        let mut c = poll_core();
        start(&mut c, &cfg);
        let (_, _, verdict) = checked_pass(&mut c, &cfg, |c| {
            let r = uop_body(c, &cfg);
            let _ = c.step();
            r
        });
        assert!(
            matches!(verdict, Shadow::Mismatch(_)),
            "an extra step shows in pc, steps or the call state"
        );
    }

    #[test]
    fn the_shadowed_pass_says_match_when_it_compared_and_skipped_when_it_could_not() {
        let cfg = poll_cfg();
        let mut c = poll_core();
        arm_uop(&mut c, UopConfig::default(), POLL_PC).unwrap();
        assert_eq!(
            lean_pass_uop_shadowed(&mut c, &cfg),
            (11, true, Shadow::Match)
        );
        // a core with a peripheral model cannot be rewound: the pass ran, nothing was compared
        let mut p = poll_core();
        p.enable_epdma();
        arm_uop(&mut p, UopConfig::default(), POLL_PC).unwrap();
        assert_eq!(
            lean_pass_uop_shadowed(&mut p, &cfg),
            (11, true, Shadow::Skipped)
        );
        // the checked form reports both as "no difference"
        assert_eq!(lean_pass_uop_checked(&mut p, &cfg), (11, true, None));
    }
}
