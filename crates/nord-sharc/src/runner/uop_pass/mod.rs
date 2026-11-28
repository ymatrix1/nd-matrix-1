//! The pass loop with the uop engine: [`lean_pass_uop`] and its checked twins. The arming of a
//! booted core (which code to translate, the stop pc) is in [`arm`], the engine names and the
//! arming a runner was asked for in [`engines`], the checked passes (replayed on the interpreter
//! and compared) in [`checked`]; all are re-exported here.
//!
//! OWNER: W1-RUN. Same contract as `lean_pass` (`system.rs`) and `lean_pass_aot`
//! (`pass_sched/mod.rs`): trigger the pass, run to the poll loop with the mailbox cleared, return
//! `(steps, reached the idle loop)`. On a core without `Core::enable_uop` every `uop_try`
//! returns false and the loop is the interpreter's. After the pass, a program change or overlay
//! load (`uop_dirty() != 0`) is followed by `uop_maintain`, which may allocate; a steady pass
//! never reaches it (gate G2, `maintain_calls == 0`).
//!
//! The timed loops ([`lean_pass_uop`], [`lean_pass_uop_checked`]) watch nothing. The `_entries`
//! forms call back with the counters one block entry moved (one `uop_try` that found a block,
//! whether or not it ran a step), for the untimed block-length histogram of `uop-report`
//! (`uop_report::hist`); the loop is the same, the observer is a generic parameter that is empty
//! for the timed forms.
mod arm;
mod checked;
mod engines;
mod probe;
#[cfg(test)]
pub(crate) mod support;

pub use arm::{arm_uop, arm_uop_ranges, code_ranges, CODE_WINDOW, RANGE_GAP, RANGE_TAIL};
pub use checked::{lean_pass_uop_checked, lean_pass_uop_shadowed, lean_pass_uop_shadowed_entries};
pub use engines::{arm_for, engine_named, uop_config};
pub use probe::{pc_ranges, probe_ranges, ranges_for, RangeMode};

use super::pass::PassConfig;
use super::system::start;
use crate::core::{Core, UopStats};

/// What one block entry (one `uop_try` that found a block) added to the engine counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockEntry {
    /// Blocks run by the entry (`blocks_run`): one, or more when the driver chained linked blocks
    /// inside the `uop_try` (`drive/chain.rs`).
    pub blocks: u64,
    /// Uops dispatched.
    pub uops: u64,
    /// Steps the entry accounted (`steps_uop`: the length of the block as it ran, which is shorter
    /// than the block when it exited early and 0 when it bailed on its first uop).
    pub steps: u64,
}

impl BlockEntry {
    /// The counters `after` moved since `before`.
    pub fn between(before: &UopStats, after: &UopStats) -> Self {
        let after = after.since(before);
        BlockEntry {
            blocks: after.blocks_run,
            uops: after.uops,
            steps: after.steps_uop,
        }
    }
}

/// Watches the block entries of [`uop_body_with`].
trait Entries {
    /// Before each `uop_try`.
    fn before(&mut self, core: &Core);
    /// After each `uop_try`, whether or not it ran a step.
    fn after(&mut self, core: &Core);
}

/// The observer of the timed loops: nothing, and nothing to optimize away.
struct NoEntries;

impl Entries for NoEntries {
    #[inline(always)]
    fn before(&mut self, _: &Core) {}
    #[inline(always)]
    fn after(&mut self, _: &Core) {}
}

/// Reports each block entry to a callback: a `uop_try` that found a block, even when the block
/// bailed before its first step (such an entry has `steps == 0`).
struct EntryCallback<'a> {
    on_entry: &'a mut dyn FnMut(BlockEntry),
    before: UopStats,
}

impl<'a> EntryCallback<'a> {
    fn new(on_entry: &'a mut dyn FnMut(BlockEntry)) -> Self {
        EntryCallback {
            on_entry,
            before: UopStats::default(),
        }
    }
}

impl Entries for EntryCallback<'_> {
    fn before(&mut self, core: &Core) {
        self.before = core.uop_stats();
    }
    fn after(&mut self, core: &Core) {
        let entry = BlockEntry::between(&self.before, &core.uop_stats());
        if entry.blocks > 0 {
            (self.on_entry)(entry);
        }
    }
}

/// The trigger and the pass count: what every pass of the engine starts with.
fn begin_pass(core: &mut Core, cfg: &PassConfig) {
    start(core, cfg);
    core.uop_begin_pass(cfg.idle_pc);
}

/// The pass loop proper, after the trigger: block hook first, `Core::step` for the gaps. In a test
/// build a core marked with `support::mark_wrong_engine` ends the pass wrong, which is how the
/// tests of the checked passes get an engine that differs from the interpreter.
fn uop_body_with(core: &mut Core, cfg: &PassConfig, watch: &mut impl Entries) -> (u64, bool) {
    let result = engine_loop(core, cfg, watch);
    #[cfg(test)]
    support::misbehave(core);
    result
}

/// The loop of [`uop_body_with`].
fn engine_loop(core: &mut Core, cfg: &PassConfig, watch: &mut impl Entries) -> (u64, bool) {
    let begin = core.steps;
    let limit = begin + cfg.max_steps;
    while core.steps < limit {
        watch.before(core);
        let ran = core.uop_try(limit);
        watch.after(core);
        if ran {
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

/// [`uop_body_with`] that watches nothing: the timed loop.
fn uop_body(core: &mut Core, cfg: &PassConfig) -> (u64, bool) {
    uop_body_with(core, cfg, &mut NoEntries)
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
    begin_pass(core, cfg);
    uop_body(core, cfg)
}

/// [`lean_pass_uop`] that reports every block entry to `on_entry` (untimed profiling: it reads the
/// engine counters around each entry). The pass itself is the same.
pub fn lean_pass_uop_entries(
    core: &mut Core,
    cfg: &PassConfig,
    on_entry: &mut dyn FnMut(BlockEntry),
) -> (u64, bool) {
    begin_pass(core, cfg);
    uop_body_with(core, cfg, &mut EntryCallback::new(on_entry))
}

#[cfg(test)]
mod tests {
    use super::support::{core, poll_cfg, poll_core, CODE, POLL_PC};
    use super::*;
    use crate::core::UopConfig;
    use crate::runner::system::lean_pass;

    fn cfg() -> PassConfig {
        PassConfig {
            trigger: 0xB_00E2,
            idle_pc: CODE,
            max_steps: 500,
            irq: None,
        }
    }

    #[test]
    fn stub_pass_equals_interpreter_pass() {
        let (mut want, mut got) = (core(), core());
        let w = lean_pass(&mut want, &cfg());
        assert_eq!(lean_pass_uop(&mut got, &cfg()), w, "engine never enabled");
        assert_eq!(want.state_mismatch(&got), None);
        // armed on the code, the pass is still the interpreter's
        let mut armed = core();
        armed
            .enable_uop(UopConfig::default(), &[(CODE, CODE + 0x258)])
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
    fn a_steady_pass_never_calls_maintain() {
        let mut c = core();
        c.enable_uop(UopConfig::default(), &[(CODE, CODE + 0x258)])
            .unwrap();
        lean_pass_uop(&mut c, &cfg());
        assert_eq!(
            (c.uop_stats().maintain_calls, c.uop_needs_maintenance()),
            (0, false)
        );
    }

    #[test]
    fn passes_that_reach_the_poll_loop_end_where_the_interpreter_pass_ends() {
        let (mut want, mut plain, mut armed, mut checked) =
            (poll_core(), poll_core(), poll_core(), poll_core());
        let report = arm::arm_uop(&mut armed, UopConfig::default(), POLL_PC).unwrap();
        let report_checked = arm::arm_uop(&mut checked, UopConfig::default(), POLL_PC).unwrap();
        assert_eq!(report, report_checked);
        // the translator is real (W1-LOWER): the poll program is translated, and the stop pc
        // starts a block of its own
        assert!(report.blocks > 0, "{report:?}");
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
        arm::arm_uop(&mut armed, UopConfig::default(), POLL_PC).unwrap();
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
            let mut c =
                crate::core::support::core_with(&[0x0001, 0x0001, 0xd70a, 0x3c23, 0xd70a, 0x0001]);
            c.preset_device(support::POLL_TRIGGER, 0);
            c
        };
        let cfg = PassConfig {
            trigger: support::POLL_TRIGGER,
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
    fn the_entries_form_is_the_same_pass_and_reports_every_entry_the_counters_saw() {
        let (mut plain, mut watched) = (poll_core(), poll_core());
        arm::arm_uop(&mut plain, UopConfig::default(), POLL_PC).unwrap();
        arm::arm_uop(&mut watched, UopConfig::default(), POLL_PC).unwrap();
        let mut seen = Vec::new();
        for pass in 0..3 {
            let want = lean_pass_uop(&mut plain, &poll_cfg());
            let got = lean_pass_uop_entries(&mut watched, &poll_cfg(), &mut |e| seen.push(e));
            assert_eq!(got, want, "pass {pass}");
            assert_eq!(plain.state_mismatch(&watched), None, "pass {pass}");
        }
        assert_eq!(
            plain.uop_stats(),
            watched.uop_stats(),
            "watching moves no counter"
        );
        let s = watched.uop_stats();
        let total = seen.iter().fold(BlockEntry::default(), |t, e| BlockEntry {
            blocks: t.blocks + e.blocks,
            uops: t.uops + e.uops,
            steps: t.steps + e.steps,
        });
        assert_eq!(
            total,
            BlockEntry {
                blocks: s.blocks_run,
                uops: s.uops,
                steps: s.steps_uop
            },
            "the entries add up to the engine counters"
        );
        // one entry per `uop_try` that entered a block, whether or not it ran a step; the driver
        // chains linked blocks inside one `uop_try`, so an entry holds one block or a chain of
        // them and there are fewer entries than blocks (the sum above is the exact statement)
        assert!(seen.iter().all(|e| e.blocks >= 1), "{seen:?}");
        assert!(
            seen.iter().any(|e| e.blocks > 1),
            "a chain of blocks is one entry: {seen:?}"
        );
        assert!((seen.len() as u64) < s.blocks_run, "{seen:?} against {s:?}");
    }

    #[test]
    fn a_block_entry_is_the_difference_of_two_counter_snapshots() {
        let a = UopStats {
            blocks_run: 2,
            uops: 7,
            steps_uop: 9,
            steps_core_step: 100,
            ..UopStats::default()
        };
        let b = UopStats {
            blocks_run: 3,
            uops: 12,
            steps_uop: 20,
            steps_core_step: 100,
            ..UopStats::default()
        };
        assert_eq!(
            BlockEntry::between(&a, &b),
            BlockEntry {
                blocks: 1,
                uops: 5,
                steps: 11
            }
        );
        assert_eq!(
            BlockEntry::between(&b, &a),
            BlockEntry::default(),
            "a reset in between reads as nothing"
        );
    }
}
