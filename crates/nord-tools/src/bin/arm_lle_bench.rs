//! ARM0 LLE benchmark (`ns4-lle2`): boot the user's OS v1.66 ARM image with panel, I2C EEPROM
//! (program memory) and UART2 MIDI in the native Rust core, then time 1 ms ticks through phases
//! like `tools/arm_org_stream.py` (load, idle, one key, eight keys). Each timed tick contains the
//! MIDI send (`action`, first tick of a phase), the tick run (idle skipping on), and the message
//! drain with the DSP return sync. The same phases run again with full execution on a clone;
//! the state hash AND the message content are compared after every phase.
//!
//!     cargo run --release -p nord-tools --bin arm-lle-bench -- \
//!         <bank0_CODE.bin> <bank0_SRAM.bin> <eeprom_image_256k.bin>
//!
//! Environment: `PHASE_SCALE` (percent of the tick counts, default 100), `NO_FULL=1` skips the
//! full-execution comparison, `NO_RESERVE=1` skips the arena reservation.

use nord_arm::machine::idle::IdleStats;
use nord_arm::machine::system::Message;
use nord_arm::machine::uart::{note_off, note_on};
use nord_arm::machine::{Machine, Stop};
use std::time::Instant;

/// ASSUMED: ARM instructions per 1 ms tick (`tools/arm_steady.py` INSNS_PER_TICK).
const TICK: u64 = 200_000;
const KEYS8: [u8; 8] = [60, 62, 64, 65, 67, 69, 71, 72];
/// Arena pages reserved before the timed run (64 MB) so on-demand mapping never reallocates.
const RESERVE_PAGES: usize = 16_384;

/// Phase name, tick count, action run on its first tick.
type PhasePlan<'a> = (&'static str, usize, &'a dyn Fn(&mut Machine));

/// Per-tick measurements of one phase.
struct Row {
    name: &'static str,
    ms: Vec<f64>,
    executed: Vec<u64>,
    device_reads: Vec<u64>,
    arena_growth: Vec<usize>,
    msgs: Vec<Message>,
}

fn percentile(v: &[f64], p: f64) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s[((s.len() - 1) as f64 * p) as usize]
}

/// Longest run of consecutive ticks over 1 ms, and the largest backlog (ms of work behind real
/// time) if ticks were consumed at 1 per ms with leftover work carried over.
fn tail(ms: &[f64]) -> (usize, f64) {
    let (mut run, mut longest, mut backlog, mut worst) = (0usize, 0usize, 0.0f64, 0.0f64);
    for t in ms {
        if *t > 1.0 {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
        backlog = (backlog + t - 1.0).max(0.0);
        worst = worst.max(backlog);
    }
    (longest, worst)
}

fn run_phase(
    m: &mut Machine,
    name: &'static str,
    ticks: usize,
    idle: Option<&mut IdleStats>,
    action: &dyn Fn(&mut Machine),
) -> Row {
    let mut row = Row {
        name,
        ms: Vec::with_capacity(ticks),
        executed: Vec::new(),
        device_reads: Vec::new(),
        arena_growth: Vec::new(),
        msgs: Vec::new(),
    };
    let mut buf: Vec<Message> = Vec::with_capacity(4096);
    let mut none = IdleStats::default();
    let stats = idle.unwrap_or(&mut none);
    for i in 0..ticks {
        let (dev0, arena0, exec0) = (m.bus.device_reads, m.bus.mem.arena_bytes(), stats.executed);
        buf.clear();
        let t = Instant::now();
        if i == 0 {
            action(m);
        }
        let s = m.run_tick_idle(TICK, stats);
        m.drain_messages_into(&mut buf);
        row.ms.push(t.elapsed().as_secs_f64() * 1e3);
        assert!(
            matches!(s, Stop::Budget),
            "{name} tick {i}: {s:?} pc={:#x}",
            m.cpu.pc()
        );
        row.executed.push(stats.executed - exec0);
        row.device_reads.push(m.bus.device_reads - dev0);
        row.arena_growth.push(m.bus.mem.arena_bytes() - arena0);
        row.msgs.extend_from_slice(&buf);
    }
    row
}

/// Full execution (no skipping) of one phase on the comparison clone.
fn run_phase_full(
    m: &mut Machine,
    name: &'static str,
    ticks: usize,
    action: &dyn Fn(&mut Machine),
) -> Row {
    let mut row = Row {
        name,
        ms: Vec::new(),
        executed: Vec::new(),
        device_reads: Vec::new(),
        arena_growth: Vec::new(),
        msgs: Vec::new(),
    };
    let mut buf = Vec::new();
    for i in 0..ticks {
        buf.clear();
        let t = Instant::now();
        if i == 0 {
            action(m);
        }
        let s = m.run_tick(TICK);
        m.drain_messages_into(&mut buf);
        row.ms.push(t.elapsed().as_secs_f64() * 1e3);
        assert!(matches!(s, Stop::Budget), "{name} tick {i}: {s:?}");
        row.msgs.extend_from_slice(&buf);
    }
    row
}

fn phases(
    m: &mut Machine,
    scale: usize,
    mut stats: Option<&mut IdleStats>,
) -> (Vec<Row>, Vec<u64>) {
    let n = |x: usize| (x * scale / 100).max(1);
    let (mut rows, mut hashes) = (Vec::new(), Vec::new());
    let none = |_: &mut Machine| {};
    let on1 = |m: &mut Machine| m.send_midi(&note_on(KEYS8[0], 100, 0));
    let off1 = |m: &mut Machine| m.send_midi(&note_off(KEYS8[0], 0));
    let on8 = |m: &mut Machine| KEYS8.iter().for_each(|k| m.send_midi(&note_on(*k, 100, 0)));
    let off8 = |m: &mut Machine| KEYS8.iter().for_each(|k| m.send_midi(&note_off(*k, 0)));
    let plan: [PhasePlan; 6] = [
        ("load", n(800), &none),
        ("idle", n(300), &none),
        ("key1 on", n(200), &on1),
        ("key1 off", n(200), &off1),
        ("key8 on", n(200), &on8),
        ("key8 off", n(200), &off8),
    ];
    for (name, ticks, action) in plan {
        let row = match stats.as_deref_mut() {
            Some(s) => run_phase(m, name, ticks, Some(s), action),
            None => run_phase_full(m, name, ticks, action),
        };
        rows.push(row);
        hashes.push(m.state_hash());
    }
    (rows, hashes)
}

fn cpu_model() -> String {
    let id = std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "unknown CPU".into());
    let n = std::env::var("NUMBER_OF_PROCESSORS").unwrap_or_else(|_| "?".into());
    format!("{id} ({n} logical), {}", std::env::consts::OS)
}

fn report(rows: &[Row]) {
    println!("\nphase      ticks  mean ms  p50     p99     max     >1ms          run  backlog ms  exec insns/tick  msgs");
    let all: Vec<f64> = rows.iter().flat_map(|r| r.ms.iter().copied()).collect();
    for r in rows.iter() {
        let mean = r.ms.iter().sum::<f64>() / r.ms.len() as f64;
        let over = r.ms.iter().filter(|x| **x > 1.0).count();
        let (run, backlog) = tail(&r.ms);
        println!(
            "{:<9} {:>6}  {:>7.3}  {:>6.3}  {:>6.3}  {:>6.3}  {:>3} ({:>4.1}%)  {:>4}  {:>10.2}  {:>14.0}  {:>6}",
            r.name,
            r.ms.len(),
            mean,
            percentile(&r.ms, 0.5),
            percentile(&r.ms, 0.99),
            percentile(&r.ms, 1.0),
            over,
            100.0 * over as f64 / r.ms.len() as f64,
            run,
            backlog,
            r.executed.iter().sum::<u64>() as f64 / r.ms.len() as f64,
            r.msgs.len()
        );
    }
    let (run, backlog) = tail(&all);
    println!(
        "all phases back to back: longest over-1ms run {run} ticks, worst backlog {backlog:.2} ms"
    );
    // what the over-1 ms ticks are: real instructions, device-window polling, or arena growth
    let mut heavy = Vec::new();
    for r in rows {
        for (i, t) in r.ms.iter().enumerate() {
            if *t > 1.0 {
                heavy.push((
                    r.name,
                    i,
                    *t,
                    r.executed[i],
                    r.device_reads[i],
                    r.arena_growth[i],
                ));
            }
        }
    }
    let by_dev = heavy.iter().filter(|h| h.4 > 1_000).count();
    let by_arena = heavy.iter().filter(|h| h.5 > 0).count();
    println!(
        "over-1ms ticks: {} (device-window reads > 1000: {by_dev}, arena grew: {by_arena})",
        heavy.len()
    );
    heavy.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    for h in heavy.iter().take(8) {
        println!(
            "  {:<9} tick {:>4}  {:>6.2} ms  exec {:>7}  device reads {:>7}  arena +{} KB",
            h.0,
            h.1,
            h.2,
            h.3,
            h.4,
            h.5 / 1024
        );
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let code = std::fs::read(&a[1]).expect("code");
    let sram = std::fs::read(&a[2]).expect("sram");
    let eeprom = std::fs::read(&a[3]).expect("eeprom image");
    let scale: usize = std::env::var("PHASE_SCALE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    println!("cpu: {}", cpu_model());
    if std::env::var("STEADY_BOOT").is_ok() {
        // same boot as `tools/arm_steady.py` (display stubs on, no board): null-call comparison
        let mut m = Machine::new();
        m.prepare(&code, &sram).expect("init array");
        let (n, stop) = m.boot_to_main_loop(1_500_000_000);
        println!(
            "steady boot: {stop:?} pc={:#x} ~{n} instructions, nulls={}",
            m.cpu.pc(),
            m.null_call_count
        );
        // the Python run checks its stop address once per 200k chunk and counts whole chunks:
        // run on to the next chunk boundary to compare like with like
        let more = m.run_chunked(None, 600_000, 200_000).0;
        println!(
            "steady boot + {more} insns past the main loop: nulls={}",
            m.null_call_count
        );
        return;
    }
    let mut m = Machine::new();
    let t = Instant::now();
    m.prepare_system(&code, &sram).expect("init array");
    m.bus.i2c.as_mut().unwrap().load_image(&eeprom);
    let mut boot_stats = IdleStats::default();
    let (n, irqs, stop) = m.boot_system(1_500_000_000, &mut boot_stats);
    println!(
        "boot: {stop:?} pc={:#x} {n} virtual insns ({} executed, {} skipped), {irqs} wait-loop irqs, {:.2}s wall",
        m.cpu.pc(),
        boot_stats.executed,
        boot_stats.skipped,
        t.elapsed().as_secs_f64()
    );
    m.install_dsp_ports();
    m.install_dsp_return();
    m.enable_real_timer();
    if std::env::var("NO_RESERVE").is_err() {
        m.bus.mem.reserve_pages(RESERVE_PAGES);
    }
    println!(
        "after boot: arena {} KB, heap_top {:#x}, auto-mapped pages {}, null calls {}, wfi {}, mode changes {}",
        m.bus.mem.arena_bytes() / 1024,
        m.heap_top,
        m.bus.mem.auto_mapped_count,
        m.null_call_count,
        m.wfi,
        m.cpu.mode_changes
    );
    let mut full = m.clone();
    let mut stats = IdleStats::default();
    let (rows, h_idle) = phases(&mut m, scale, Some(&mut stats));
    report(&rows);
    println!("idle stats: {stats:?}");
    println!(
        "after run: arena {} KB, heap_top {:#x}, auto-mapped pages {}, null calls {}, wfi {} (stats {}), irq injected with I bit set {}, dropped messages {}, pending words {}, DSP return reads {}",
        m.bus.mem.arena_bytes() / 1024,
        m.heap_top,
        m.bus.mem.auto_mapped_count,
        m.null_call_count,
        m.wfi,
        stats.wfi,
        m.irq_while_masked,
        m.bus.dropped_messages,
        m.bus.grouper.pending_words(),
        m.dsp_return.as_ref().map_or(0, |r| r.reads)
    );
    if std::env::var("NO_FULL").is_err() {
        let (frows, h_full) = phases(&mut full, scale, None);
        for ((r, f), (a, b)) in rows.iter().zip(&frows).zip(h_idle.iter().zip(&h_full)) {
            let mi = r.ms.iter().sum::<f64>() / r.ms.len() as f64;
            let mf = f.ms.iter().sum::<f64>() / f.ms.len() as f64;
            println!(
                "{:<9} full {:.3} ms/tick, skipping {:.3} ms/tick, state hash {}, messages {} vs {} content {}",
                r.name,
                mf,
                mi,
                if a == b { "EQUAL" } else { "DIFFERENT" },
                f.msgs.len(),
                r.msgs.len(),
                if f.msgs == r.msgs { "EQUAL" } else { "DIFFERENT" }
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_picks_sorted_rank() {
        let v = [5.0, 1.0, 3.0, 2.0, 4.0];
        assert_eq!(percentile(&v, 0.0), 1.0);
        assert_eq!(percentile(&v, 0.5), 3.0);
        assert_eq!(percentile(&v, 1.0), 5.0);
    }

    #[test]
    fn tail_finds_the_longest_run_and_the_backlog() {
        // 3.0, 3.0 over by 2 each: backlog 4, then two 0.2 ms ticks repay 0.8 each
        let (run, backlog) = tail(&[0.2, 3.0, 3.0, 0.2, 0.2, 0.2]);
        assert_eq!(run, 2);
        assert!((backlog - 4.0).abs() < 1e-9, "{backlog}");
        assert_eq!(tail(&[0.5, 0.9]), (0, 0.0));
    }
}
