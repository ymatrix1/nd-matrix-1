//! The pass loop with the uop engine: [`lean_pass_uop`] and its checked twin.
//!
//! OWNER: W1-RUN. Same contract as `lean_pass` (`system.rs`) and `lean_pass_aot`
//! (`pass_sched.rs`): trigger the pass, run to the poll loop with the mailbox cleared, return
//! `(steps, reached the idle loop)`. On a core without `Core::enable_uop` every `uop_try`
//! returns false and the loop is the interpreter's. After the pass, a program change or overlay
//! load (`uop_dirty() != 0`) is followed by `uop_maintain`, which may allocate; a steady pass
//! never reaches it (gate G2, `maintain_calls == 0`).
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

/// [`lean_pass_uop`] checked against the interpreter: the pass runs under the engine, the core is
/// rewound to the state after the trigger, the interpreter runs the same pass and the two end
/// states are compared with `exact_diff`. The core is left in the interpreter's state, so a wrong
/// engine heals. Allocates (two memory images and a second core); tools and tests only. A core
/// with peripheral models cannot be rewound, so there the pass is unchecked and the diff is
/// `None`.
pub fn lean_pass_uop_checked(core: &mut Core, cfg: &PassConfig) -> (u64, bool, Option<ExactDiff>) {
    start(core, cfg);
    core.uop_begin_pass(cfg.idle_pc);
    let mut result = (0, false);
    let verdict = shadow_run_with(
        core,
        |c| result = uop_body(c, cfg),
        |c, _| {
            let _ = interp_body(c, cfg);
        },
    );
    match verdict {
        Shadow::Mismatch(d) => (result.0, result.1, Some(d)),
        Shadow::Match | Shadow::Skipped => (result.0, result.1, None),
    }
}

#[cfg(test)]
mod tests {
    use super::super::system::lean_pass;
    use super::*;
    use crate::core::support::core_with;
    use crate::core::UopConfig;

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
            armed.uop_stats().steps_core_step,
            w.0,
            "every step went through Core::step"
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
}
