//! Text output of `arm-sb-bench`: the boot line, the counter tables and the digest lines.
//!
//! Without `--time` the whole output is a function of the firmware image and the arguments (no
//! clock, no host name), so two runs print identical bytes.

use super::measure::Out;
use super::rows::{BootInfo, Row};

/// One table of counters per framing (`hz` is the tick rate the framing stands for). With
/// `timed` the thread CPU time columns follow (non-deterministic).
pub fn table(title: &str, rows: &[Row], hz: u64, timed: bool) -> String {
    let mut out = format!("{title}\n");
    out += &format!(
        "{:<9} {:>7} {:>13} {:>12} {:>7} {:>10} {:>9} {:>9} {:>9} {:>10}",
        "phase",
        "ticks",
        "executed",
        "probe_exec",
        "probe %",
        "exec/tick",
        "max/tick",
        "probe/tk",
        "irq/tick",
        "Minsn/s"
    );
    out += if timed { "  tick us  fixed us\n" } else { "\n" };
    for r in rows {
        out += &format!(
            "{:<9} {:>7} {:>13} {:>12} {:>6.1}% {:>10.1} {:>9} {:>9.2} {:>9.1} {:>10.2}",
            r.name,
            r.ticks,
            r.stats.executed,
            r.stats.probe_executed,
            r.probe_share(),
            r.executed_per_tick(),
            r.max_tick_executed,
            r.probes_per_tick(),
            r.irq_per_tick(),
            r.minsn_per_s(hz)
        );
        if let (Some(t), Some(f)) = (r.tick_us(), r.fixed_us()) {
            out += &format!(" {t:>8.1} {f:>9.1}");
        }
        out.push('\n');
    }
    out
}

/// The boot summary: what ran, what idle skipping removed, and the probe share of the boot.
pub fn boot_line(b: &BootInfo) -> String {
    format!(
        "boot: main loop at pc {:#x} after {} virtual insns ({} executed, {} skipped, probe_exec {} = {:.1}%), {} wait-loop irqs",
        b.pc,
        b.virtual_insns,
        b.stats.executed,
        b.stats.skipped,
        b.stats.probe_executed,
        b.probe_share(),
        b.irqs
    )
}

/// The lines a golden file holds: the configuration, the boot state hash, then the state hash and
/// the message digest of every phase (the format of the `digest` lines of `arm-lle-bench`).
pub fn digest_lines(
    tick_insns: u64,
    phase_scale: u64,
    boot: &BootInfo,
    outs: &[Out],
) -> Vec<String> {
    let mut v = vec![
        format!("digest config tick_insns={tick_insns} phase_scale={phase_scale}"),
        format!("digest boot state {:016x}", boot.state),
    ];
    for o in outs {
        v.push(format!("digest {} state {:016x}", o.row.name, o.state));
        v.push(format!(
            "digest {} messages {:016x}",
            o.row.name, o.messages
        ));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::sample_row;
    use nord_arm::machine::idle::IdleStats;

    fn boot_info() -> BootInfo {
        BootInfo {
            stats: IdleStats {
                executed: 1_000,
                probe_executed: 250,
                skipped: 9_000,
                ..IdleStats::default()
            },
            virtual_insns: 10_000,
            irqs: 6,
            pc: 0xC00A_0248,
            state: 0x0960_ef4e_efb7_fad5,
        }
    }

    #[test]
    fn the_table_has_a_title_a_header_and_one_line_per_phase() {
        let rows = [
            sample_row("load", 10, 1_000, 500),
            sample_row("idle", 10, 340, 100),
        ];
        let t = table("1 ms framing", &rows, 1_000, false);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines.len(), 4, "{t}");
        assert_eq!(lines[0], "1 ms framing");
        assert!(lines[1].starts_with("phase") && lines[1].contains("probe %"));
        assert!(lines[2].starts_with("load") && lines[3].starts_with("idle"));
        assert!(
            lines[2].contains("50.0%") && lines[2].contains("100.0"),
            "{}",
            lines[2]
        );
        assert!(
            lines[3].contains("29.4%") && lines[3].contains("34.0"),
            "{}",
            lines[3]
        );
        assert!(
            !lines[1].contains("fixed us"),
            "no timing columns without --time"
        );
    }

    #[test]
    fn the_rate_column_follows_the_framing() {
        let rows = [sample_row("idle", 1, 3_384, 0)];
        let at_1k = table("a", &rows, 1_000, false);
        let at_24k = table("b", &rows, 24_000, false);
        assert!(at_1k.lines().nth(2).unwrap().ends_with("3.38"), "{at_1k}");
        assert!(
            at_24k.lines().nth(2).unwrap().ends_with("81.22"),
            "{at_24k}"
        );
    }

    #[test]
    fn a_timed_table_adds_the_tick_time_and_the_fixed_cost() {
        let mut r = sample_row("idle", 2, 6_768, 0);
        r.time_ns = Some(126_000);
        let t = table("timed", &[r.clone()], 1_000, true);
        assert!(
            t.lines().nth(1).unwrap().ends_with("tick us  fixed us"),
            "{t}"
        );
        let row = t.lines().nth(2).unwrap();
        assert!(row.ends_with("63.0      45.1"), "{row}");
        // an untimed row in a timed table prints no time cells instead of zeros
        r.time_ns = None;
        let t = table("timed", &[r], 1_000, true);
        assert!(!t.lines().nth(2).unwrap().ends_with("45.1"));
    }

    #[test]
    fn the_output_is_deterministic_without_timing() {
        let rows = [sample_row("load", 3, 300, 30)];
        assert_eq!(
            table("t", &rows, 24_000, false),
            table("t", &rows, 24_000, false)
        );
        assert_eq!(boot_line(&boot_info()), boot_line(&boot_info()));
    }

    #[test]
    fn the_boot_line_reports_the_split_between_executed_and_skipped_work() {
        let line = boot_line(&boot_info());
        assert!(
            line.contains("pc 0xc00a0248") && line.contains("10000 virtual insns"),
            "{line}"
        );
        assert!(line.contains("1000 executed, 9000 skipped"), "{line}");
        assert!(line.contains("probe_exec 250 = 25.0%") && line.contains("6 wait-loop irqs"));
    }

    #[test]
    fn the_digest_lines_are_the_config_the_boot_hash_and_two_lines_per_phase() {
        let outs = [Out {
            row: sample_row("key1 on", 1, 1, 0),
            state: 0x68a8_abb3_7e4e_1d0b,
            messages: 0x221c_6f69_751f_1f8e,
        }];
        assert_eq!(
            digest_lines(200_000, 100, &boot_info(), &outs),
            [
                "digest config tick_insns=200000 phase_scale=100",
                "digest boot state 0960ef4eefb7fad5",
                "digest key1 on state 68a8abb37e4e1d0b",
                "digest key1 on messages 221c6f69751f1f8e",
            ]
        );
    }
}
