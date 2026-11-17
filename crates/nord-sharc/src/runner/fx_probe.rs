//! Pass probes for the live FX runs (split out of `fx_run.rs`): the entry registers of a stage
//! routine and the number of stage calls per pass.
use super::pass::PassConfig;
use super::system::start;
use crate::core::Core;

/// Runs one pass and returns the `(M, I)` register banks the first time `pc` is about to execute
/// (the entry registers of a stage routine: what the dispatcher provides). `None` when the pass
/// never reaches `pc`.
pub fn regs_at(core: &mut Core, cfg: &PassConfig, pc: u32) -> Option<([u32; 16], [u32; 16])> {
    start(core, cfg);
    let begin = core.steps;
    while core.steps - begin < cfg.max_steps {
        if core.pc == pc {
            return Some((core.m, core.i));
        }
        if core.step().is_err() || (core.pc == cfg.idle_pc && core.dm_read(cfg.trigger) == Ok(0)) {
            return None;
        }
    }
    None
}

/// Number of times a pass reaches `pc` (how often the dispatcher calls a stage routine): the
/// stage count of the pass, 1 when exactly one stage is installed. `None` is never returned; a
/// pass that faults ends the count.
pub fn calls_at(core: &mut Core, cfg: &PassConfig, pc: u32) -> u32 {
    start(core, cfg);
    let (begin, mut n) = (core.steps, 0);
    while core.steps - begin < cfg.max_steps {
        if core.pc == pc {
            n += 1;
        }
        if core.step().is_err() || (core.pc == cfg.idle_pc && core.dm_read(cfg.trigger) == Ok(0)) {
            break;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::super::pass::tests::clearing_core;
    use super::*;

    #[test]
    fn regs_at_reports_the_registers_where_a_pass_reaches_the_pc() {
        let cfg = PassConfig {
            trigger: 0xB_00E2,
            idle_pc: 0x12_0002,
            max_steps: 10,
            irq: None,
        };
        let mut c = clearing_core();
        c.m[2] = 2;
        let first = c.pc;
        let (m, i) = regs_at(&mut c, &cfg, first).expect("the start pc is reached at once");
        assert_eq!((m[2], i[0]), (2, 0xB_00E2));
        let mut c = clearing_core();
        assert_eq!(
            regs_at(&mut c, &cfg, 0x1F_FFFF),
            None,
            "a pc the pass never reaches"
        );
    }

    #[test]
    fn calls_at_counts_how_often_a_pass_reaches_the_pc() {
        let cfg = PassConfig {
            trigger: 0xB_00E2,
            idle_pc: 0x12_0002,
            max_steps: 10,
            irq: None,
        };
        let mut c = clearing_core();
        let first = c.pc;
        assert_eq!(
            calls_at(&mut c, &cfg, first),
            1,
            "the start pc is reached once"
        );
        let mut c = clearing_core();
        assert_eq!(calls_at(&mut c, &cfg, 0x1F_FFFF), 0);
    }
}
