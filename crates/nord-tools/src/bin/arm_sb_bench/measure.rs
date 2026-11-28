//! Boot and phase passes of `arm-sb-bench`. Main APIs only (`Machine`, `IdleStats`), so the
//! measurement runs on `main` and on the superblock branch alike and never enters `machine::sb`.
//!
//! The phases and the work of a tick are those of `arm-lle-bench` (`plan`, `run_phase`): the MIDI
//! action in the first tick of a phase, `run_tick_idle` (idle skipping on), then the message
//! drain with the DSP return sync.

use super::args::FRAME_DIVISOR;
use super::rows::{BootInfo, Row};
use nord_arm::machine::idle::IdleStats;
use nord_arm::machine::metrics::digest_messages;
use nord_arm::machine::system::Message;
use nord_arm::machine::uart::{note_off, note_on};
use nord_arm::machine::{Machine, Stop};
use nord_sharc::cpu_clock::ThreadClock;

/// ASSUMED: the same eight keys as `arm-lle-bench` (a C major scale from middle C).
const KEYS8: [u8; 8] = [60, 62, 64, 65, 67, 69, 71, 72];
/// ASSUMED: virtual instruction cap of the boot, as `arm-lle-bench`; only a bound, the boot stops
/// at the main loop (a boot that reaches the cap is reported as an error).
const BOOT_MAX: u64 = 1_500_000_000;
/// ASSUMED: arena pages reserved after the boot (64 MB, as `arm-lle-bench`) so on-demand mapping
/// never reallocates in a timed run; no effect on the counters.
const RESERVE_PAGES: usize = 16_384;

pub type Action = fn(&mut Machine);

fn act_none(_: &mut Machine) {}
fn act_on1(m: &mut Machine) {
    m.send_midi(&note_on(KEYS8[0], 100, 0));
}
fn act_off1(m: &mut Machine) {
    m.send_midi(&note_off(KEYS8[0], 0));
}
fn act_on8(m: &mut Machine) {
    KEYS8.iter().for_each(|k| m.send_midi(&note_on(*k, 100, 0)));
}
fn act_off8(m: &mut Machine) {
    KEYS8.iter().for_each(|k| m.send_midi(&note_off(*k, 0)));
}

/// The six phases of `arm-lle-bench` (ticks scaled by `scale` percent, at least one tick each)
/// with the MIDI action sent in the first tick. ASSUMED: lengths and order as that bench, which
/// follows `tools/arm_org_stream.py`.
pub fn plan(scale: u64) -> [(&'static str, u64, Action); 6] {
    let n = |x: u64| (x * scale / 100).max(1);
    [
        ("load", n(800), act_none),
        ("idle", n(300), act_none),
        ("key1 on", n(200), act_on1),
        ("key1 off", n(200), act_off1),
        ("key8 on", n(200), act_on8),
        ("key8 off", n(200), act_off8),
    ]
}

/// Boot the user's OS image with the full board and put the machine in the state the phases
/// start from (`arm-lle-bench` order). `BootInfo::state` is taken right after the boot, before
/// the DSP port and timer set-up, like the `boot rate ... state hash` line of that bench.
pub fn boot(
    code: &[u8],
    sram: &[u8],
    eeprom: &[u8],
    predecode: bool,
) -> Result<(Machine, BootInfo), String> {
    let mut m = Machine::new();
    m.predecode = predecode;
    m.prepare_system(code, sram)
        .map_err(|s| format!("prepare_system stopped with {s:?}"))?;
    m.bus
        .i2c
        .as_mut()
        .ok_or("the board has no I2C EEPROM")?
        .load_image(eeprom);
    let mut stats = IdleStats::default();
    let (virtual_insns, irqs, stop) = m.boot_system(BOOT_MAX, &mut stats);
    if stop != Stop::StopPc {
        return Err(format!(
            "the boot ended with {stop:?} at pc {:#x}, not at the main loop",
            m.cpu.pc()
        ));
    }
    let info = BootInfo {
        stats,
        virtual_insns,
        irqs,
        pc: m.cpu.pc(),
        state: m.state_hash(),
    };
    m.install_dsp_ports();
    m.install_dsp_return();
    m.enable_real_timer();
    m.bus.mem.reserve_pages(RESERVE_PAGES);
    Ok((m, info))
}

/// A copy of the booted machine for the second framing, with the same arena reserve as the
/// original (the clone keeps only the used part of the arena).
pub fn fork(m: &Machine) -> Machine {
    let mut copy = m.clone();
    copy.bus.mem.reserve_pages(RESERVE_PAGES);
    copy
}

/// One phase: `ticks` ticks of `budget` virtual instructions. The messages are returned when
/// `collect` is set, else drained and dropped.
fn run_phase(
    m: &mut Machine,
    (name, ticks, action): (&str, u64, Action),
    budget: u64,
    collect: bool,
    timed: bool,
) -> Result<(Row, Vec<Message>), String> {
    let mut row = Row::new(name, budget, timed);
    let (mut msgs, mut scratch) = (Vec::new(), Vec::with_capacity(4096));
    for i in 0..ticks {
        let (insns0, exec0) = (m.cpu.insns, row.stats.executed);
        let clock = timed.then(ThreadClock::start);
        if i == 0 {
            action(m);
        }
        let stop = m.run_tick_idle(budget, &mut row.stats);
        if collect {
            m.drain_messages_into(&mut msgs);
        } else {
            scratch.clear();
            m.drain_messages_into(&mut scratch);
        }
        let ns = clock.map(|c| c.elapsed_ns());
        if !matches!(stop, Stop::Budget) {
            return Err(format!("{name} tick {i}: {stop:?} at pc {:#x}", m.cpu.pc()));
        }
        // a Budget stop advances `cpu.insns` by exactly the budget; the rest is handler code
        let irq = (m.cpu.insns - insns0).saturating_sub(budget);
        row.end_tick(row.stats.executed - exec0, irq, ns);
    }
    Ok((row, msgs))
}

/// Counters, state hash and message digest of one phase of the 1 ms pass.
#[derive(Clone, Debug, PartialEq)]
pub struct Out {
    pub row: Row,
    pub state: u64,
    pub messages: u64,
}

/// The six phases at the 1 ms framing: `budget` virtual instructions per tick.
pub fn ms_pass(m: &mut Machine, scale: u64, budget: u64, timed: bool) -> Result<Vec<Out>, String> {
    plan(scale)
        .into_iter()
        .map(|p| {
            let (row, msgs) = run_phase(m, p, budget, true, timed)?;
            Ok(Out {
                row,
                state: m.state_hash(),
                messages: digest_messages(&msgs),
            })
        })
        .collect()
}

/// The same virtual time at the 24 kHz framing: [`FRAME_DIVISOR`] times as many ticks of
/// `budget / FRAME_DIVISOR` virtual instructions, one timer interrupt each. Counters only.
pub fn khz24_pass(
    m: &mut Machine,
    scale: u64,
    budget: u64,
    timed: bool,
) -> Result<Vec<Row>, String> {
    plan(scale)
        .into_iter()
        .map(|(name, ticks, action)| {
            let p = (name, ticks * FRAME_DIVISOR, action);
            run_phase(m, p, budget / FRAME_DIVISOR, false, timed).map(|(row, _)| row)
        })
        .collect()
}

/// Test fixture: a spin loop of period 3 (`cmp r1,#0 ; moveq r2,r2 ; b loop`) with a
/// two-instruction timer handler (`add r4,r4,#1 ; bx lr`), so a tick has a known cost.
#[cfg(test)]
pub fn spin_machine() -> Machine {
    use nord_arm::machine::boot::TIMER_IRQ;
    let mut m = Machine::new();
    for (i, w) in [0xE351_0000u32, 0x01A0_2002, 0xEAFF_FFFC]
        .iter()
        .enumerate()
    {
        m.write32(0xC000_0000 + 4 * i as u32, *w);
    }
    m.write32(0xC000_0100, 0xE284_4001);
    m.write32(0xC000_0104, 0xE12F_FF1E);
    m.write32(m.layout.irq_table + 4 * TIMER_IRQ, 0xC000_0100);
    m.cpu.r[13] = 0x7000_8000;
    m.cpu.r[15] = 0xC000_0000;
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use nord_arm::machine::idle::MIN_SLICE;

    /// Executed instructions of one tick of the spin loop: a first slice, the probe of one loop
    /// period, then the remainder that is not a multiple of the period.
    fn spin_executed(budget: u64) -> u64 {
        MIN_SLICE + 3 + (budget - MIN_SLICE - 3) % 3
    }

    #[test]
    fn the_plan_scales_the_phase_lengths_and_keeps_every_phase() {
        assert_eq!(plan(100).map(|p| p.1), [800, 300, 200, 200, 200, 200]);
        assert_eq!(plan(1).map(|p| p.1), [8, 3, 2, 2, 2, 2]);
        assert!(plan(0).iter().all(|p| p.1 == 1), "never zero ticks");
        assert_eq!(
            plan(100).map(|p| p.0),
            ["load", "idle", "key1 on", "key1 off", "key8 on", "key8 off"]
        );
    }

    #[test]
    fn the_1ms_pass_counts_probes_skips_and_handler_instructions_per_phase() {
        let mut m = spin_machine();
        let out = ms_pass(&mut m, 1, 10_000, false).unwrap();
        assert_eq!(out.len(), 6);
        let per_tick = spin_executed(10_000);
        assert_eq!(
            per_tick, 67,
            "64-instruction slice + the 3-instruction probe"
        );
        for (o, ticks) in out.iter().zip([8u64, 3, 2, 2, 2, 2]) {
            let r = &o.row;
            // not vacuous: every phase ran ticks, probed, skipped and ran the handler
            assert_eq!(r.ticks, ticks, "{}", r.name);
            assert_eq!(r.stats.executed, per_tick * ticks, "{}", r.name);
            assert_eq!(r.stats.probe_executed, 3 * ticks, "{}", r.name);
            assert_eq!(
                (r.stats.probes, r.stats.skips),
                (ticks, ticks),
                "{}",
                r.name
            );
            assert_eq!(r.stats.skipped, (10_000 - per_tick) * ticks, "{}", r.name);
            assert_eq!((r.irq_insns, r.max_tick_executed), (2 * ticks, per_tick));
            assert_eq!(r.time_ns, None);
            assert_eq!(
                o.messages,
                digest_messages(&[]),
                "no messages on this machine"
            );
        }
        assert!((out[0].row.probe_share() - 100.0 * 3.0 / 67.0).abs() < 1e-9);
        // the clock advanced by the budget plus the handler of every tick: 6 phases, 19 ticks
        assert_eq!(m.cpu.insns, 19 * (10_000 + 2));
        assert_eq!(m.reg(4), 0, "the handler ran, call_preserving restored r4");
    }

    #[test]
    fn the_24khz_pass_runs_the_same_virtual_time_in_smaller_ticks() {
        let mut m = spin_machine();
        let rows = khz24_pass(&mut m, 1, 10_000, false).unwrap();
        let (budget, ticks) = (10_000 / FRAME_DIVISOR, [8u64, 3, 2, 2, 2, 2]);
        for (r, t) in rows.iter().zip(ticks) {
            assert_eq!(
                (r.ticks, r.budget),
                (t * FRAME_DIVISOR, budget),
                "{}",
                r.name
            );
            assert_eq!(
                r.stats.executed,
                spin_executed(budget) * r.ticks,
                "{}",
                r.name
            );
            assert_eq!(r.irq_insns, 2 * r.ticks, "{}", r.name);
        }
        // a smaller tick executes more per virtual instruction: the fixed work repeats
        let ms = ms_pass(&mut spin_machine(), 1, 10_000, false).unwrap();
        assert!(rows[0].stats.executed > ms[0].row.stats.executed * 20);
        assert_eq!(rows[0].executed_per_tick(), spin_executed(budget) as f64);
    }

    #[test]
    fn a_tick_that_does_not_end_on_its_budget_is_an_error_naming_the_phase() {
        let mut m = spin_machine();
        m.stop_pc = Some(0xC000_0000);
        let err = ms_pass(&mut m, 1, 10_000, false).unwrap_err();
        assert!(
            err.contains("load tick 0") && err.contains("StopPc"),
            "{err}"
        );
        let mut m = spin_machine();
        m.stop_pc = Some(0xC000_0000);
        assert!(khz24_pass(&mut m, 1, 10_000, false).is_err());
    }

    #[test]
    fn a_fork_is_an_independent_copy_of_the_same_state() {
        let mut m = spin_machine();
        let mut copy = fork(&m);
        assert_eq!(copy.state_hash(), m.state_hash());
        ms_pass(&mut copy, 1, 10_000, false).unwrap();
        assert_ne!(
            copy.cpu.insns, m.cpu.insns,
            "the copy ran, the original did not"
        );
        assert_eq!(m.cpu.insns, 0);
        // both reach the same state from the same start
        ms_pass(&mut m, 1, 10_000, false).unwrap();
        assert_eq!(
            (m.state_hash(), m.cpu.insns),
            (copy.state_hash(), copy.cpu.insns)
        );
    }

    #[test]
    fn a_timed_pass_records_thread_time_and_the_counters_stay_equal() {
        let plain = ms_pass(&mut spin_machine(), 1, 10_000, false).unwrap();
        let timed = ms_pass(&mut spin_machine(), 1, 10_000, true).unwrap();
        for (p, t) in plain.iter().zip(&timed) {
            assert!(t.row.time_ns.is_some_and(|ns| ns > 0), "{}", t.row.name);
            assert!(t.row.tick_us().unwrap() > 0.0 && t.row.fixed_us().is_some());
            let mut untimed = t.row.clone();
            untimed.time_ns = None;
            assert_eq!(untimed, p.row, "timing must not change a counter");
            assert_eq!((t.state, t.messages), (p.state, p.messages));
        }
    }
}
