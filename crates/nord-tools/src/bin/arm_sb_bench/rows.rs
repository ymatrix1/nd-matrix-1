//! Counters of one phase and the derived figures of gate G0 (`docs/research/arm0-jit-design.md`
//! section 13): probe share, executed instructions per tick, handler instructions per tick and,
//! with `--time`, the fixed tick cost.

use nord_arm::machine::idle::IdleStats;

/// EMPIRICAL: host cost of one executed guest instruction, 5.3 ns = 185 to 198 M insns/s on the
/// predecode cache (least disturbed runs, design note section 1, from the PR 689 note
/// `arm0-rust-runtime`). Only the `--time` split uses it; the deterministic counters do not.
pub const NS_PER_INSN: f64 = 5.3;

/// Counters of one phase at one framing (one tick = `budget` virtual instructions plus one timer
/// interrupt).
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub name: String,
    pub ticks: u64,
    /// Virtual instructions per tick (the budget of `Machine::run_tick_idle`).
    pub budget: u64,
    /// Idle statistics summed over the phase (`executed` counts the tick run, probes included).
    pub stats: IdleStats,
    /// Instructions run by the injected handlers (timer interrupt, UART ISR of the MIDI send):
    /// the `cpu.insns` advance of a tick minus its budget. Not part of `stats.executed`.
    pub irq_insns: u64,
    /// Largest `stats.executed` advance of a single tick.
    pub max_tick_executed: u64,
    /// Thread CPU time of all ticks in ns (`--time` only; `None` keeps the output deterministic).
    pub time_ns: Option<u64>,
}

impl Row {
    pub fn new(name: &str, budget: u64, timed: bool) -> Self {
        Self {
            name: name.into(),
            ticks: 0,
            budget,
            stats: IdleStats::default(),
            irq_insns: 0,
            max_tick_executed: 0,
            time_ns: timed.then_some(0),
        }
    }

    /// Account one finished tick (`stats` was advanced by the caller).
    pub fn end_tick(&mut self, executed: u64, irq_insns: u64, ns: Option<u64>) {
        self.ticks += 1;
        self.irq_insns += irq_insns;
        self.max_tick_executed = self.max_tick_executed.max(executed);
        if let (Some(total), Some(ns)) = (&mut self.time_ns, ns) {
            *total += ns;
        }
    }

    /// `probe_executed / executed` in percent (0 when nothing executed): gate G0 (a).
    pub fn probe_share(&self) -> f64 {
        if self.stats.executed == 0 {
            0.0
        } else {
            100.0 * self.stats.probe_executed as f64 / self.stats.executed as f64
        }
    }

    /// `total / ticks` (0 for an empty phase).
    fn per_tick(&self, total: u64) -> f64 {
        if self.ticks == 0 {
            0.0
        } else {
            total as f64 / self.ticks as f64
        }
    }

    /// Executed instructions per tick (probe instructions included): gate G0 (c).
    pub fn executed_per_tick(&self) -> f64 {
        self.per_tick(self.stats.executed)
    }

    pub fn probes_per_tick(&self) -> f64 {
        self.per_tick(self.stats.probes)
    }

    /// Handler instructions per tick (they run on the plain path and are not in `executed`).
    pub fn irq_per_tick(&self) -> f64 {
        self.per_tick(self.irq_insns)
    }

    /// Executed instructions per second when a tick takes `1 / hz` s.
    pub fn minsn_per_s(&self, hz: u64) -> f64 {
        self.executed_per_tick() * hz as f64 / 1e6
    }

    /// Mean thread CPU time per tick in us (`--time` only).
    pub fn tick_us(&self) -> Option<f64> {
        self.time_ns.map(|ns| self.per_tick(ns) / 1000.0)
    }

    /// Fixed tick cost in us (`--time` only): gate G0 (b).
    pub fn fixed_us(&self) -> Option<f64> {
        self.tick_us()
            .map(|t| fixed_cost_us(t, self.executed_per_tick()))
    }
}

/// Gate G0 (b): the part of a tick that does not scale with the executed instructions,
/// `tick time - executed instructions x 5.3 ns`, in us (negative when the model overshoots).
pub fn fixed_cost_us(tick_us: f64, executed_per_tick: f64) -> f64 {
    tick_us - executed_per_tick * NS_PER_INSN / 1000.0
}

/// What the boot reported (`Machine::boot_system`) and the state hash right after it.
#[derive(Clone, Debug, PartialEq)]
pub struct BootInfo {
    pub stats: IdleStats,
    /// `cpu.insns` advance of the boot (executed and skipped).
    pub virtual_insns: u64,
    /// Wait-loop timer interrupts injected during the boot.
    pub irqs: u64,
    pub pc: u32,
    pub state: u64,
}

impl BootInfo {
    /// `probe_executed / executed` of the boot in percent.
    pub fn probe_share(&self) -> f64 {
        if self.stats.executed == 0 {
            0.0
        } else {
            100.0 * self.stats.probe_executed as f64 / self.stats.executed as f64
        }
    }
}

#[cfg(test)]
pub fn sample_row(name: &str, ticks: u64, executed: u64, probe_executed: u64) -> Row {
    let mut r = Row::new(name, 200_000, false);
    r.ticks = ticks;
    r.stats = IdleStats {
        executed,
        probe_executed,
        probes: ticks * 2,
        ..IdleStats::default()
    };
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_share_and_the_per_tick_figures_follow_the_counters() {
        let mut r = sample_row("idle", 24, 4_800_000, 1_200_000);
        r.irq_insns = 2_400;
        assert_eq!(r.probe_share(), 25.0);
        assert_eq!(r.executed_per_tick(), 200_000.0);
        assert_eq!(r.probes_per_tick(), 2.0);
        assert_eq!(r.irq_per_tick(), 100.0);
        // 200 k executed per tick at 24 kHz is 4.8 G insns/s, at 1 kHz 200 M
        assert_eq!(r.minsn_per_s(24_000), 4_800.0);
        assert_eq!(r.minsn_per_s(1_000), 200.0);
        assert_eq!((r.tick_us(), r.fixed_us()), (None, None));
    }

    #[test]
    fn an_empty_phase_has_zero_figures_not_nan() {
        let r = Row::new("none", 1, false);
        assert_eq!(
            (r.probe_share(), r.executed_per_tick(), r.irq_per_tick()),
            (0.0, 0.0, 0.0)
        );
        let b = BootInfo {
            stats: IdleStats::default(),
            virtual_insns: 0,
            irqs: 0,
            pc: 0,
            state: 0,
        };
        assert_eq!(b.probe_share(), 0.0);
    }

    #[test]
    fn end_tick_accumulates_and_tracks_the_largest_tick() {
        let mut r = Row::new("load", 100, true);
        r.end_tick(10, 2, Some(1_000));
        r.end_tick(30, 2, Some(3_000));
        r.end_tick(20, 0, None);
        assert_eq!(
            (r.ticks, r.irq_insns, r.max_tick_executed, r.time_ns),
            (3, 4, 30, Some(4_000))
        );
        // an untimed row never grows a time
        let mut u = Row::new("load", 100, false);
        u.end_tick(1, 0, Some(5));
        assert_eq!(u.time_ns, None);
    }

    #[test]
    fn the_fixed_cost_is_the_tick_time_minus_the_executed_instructions() {
        // the design note's idle tick: 63 us for 3,384 insns x 5.3 ns = 17.9 us -> 45.1 us fixed
        let fixed = fixed_cost_us(63.0, 3_384.0);
        assert!((fixed - 45.06).abs() < 0.01, "{fixed}");
        assert!(fixed_cost_us(10.0, 3_384.0) < 0.0);
        // through a timed row: 2 ticks of 126 us in total -> 63 us per tick
        let mut r = sample_row("idle", 2, 6_768, 0);
        r.time_ns = Some(126_000);
        assert_eq!(r.tick_us(), Some(63.0));
        assert!((r.fixed_us().unwrap() - fixed).abs() < 1e-9);
    }
}
