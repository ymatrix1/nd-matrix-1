//! `lle-gates`: one reproducible measurement of the LLE engines against the gates of
//! `docs/research/sharc-compiler-design.md` section 12 and `docs/research/arm0-jit-design.md`
//! section 13. Usage and the measurement protocol: `docs/research/lle-gates-howto.md`.
//!
//! ```text
//! cargo run --release -p nord-tools --bin lle_gates -- [--fw .reference/fw_unpacked] [--bank 0]
//! cargo run --release -p nord-tools --bin lle_gates -- --synthetic     # no firmware: wiring check
//! ```
//!
//! Per SHARC core (ORG Core2, PNO Core2) and per engine (interpreter, AOT, uop tier 1, uop checked,
//! uop native with `--features nord-sharc/uop-native`) and per ARM0 tier, the engines run one
//! after the other, round by round, after one discarded warm-up round; the report is a table and
//! one JSON object with mean, median, p99 and max per pass (or tick) as median [min..max] over the
//! runs, the share of the 46.08 us budget, the G2 step split, `maintain_calls` and `smc`, and for
//! ARM the share of ticks under 1 ms. The host background load is sampled before, between and after
//! the runs; above 25 % the timings are withheld (exit 4) unless `--indicative` prints them marked.
//! Two engines whose medians differ by less than the larger min-max band are "within noise".
//!
//! Firmware is read from the user's `--fw` directory (git-ignored `.reference/`, never committed);
//! when a needed file is missing the part is skipped with a message naming it (exit 0). Exit codes:
//! 0 report printed or skipped, 2 usage, 3 a run failed, 4 timings refused for host load.

// the statistics, load sampling, alternation, engines, gates and report behind the tool
#[path = "lle_gates/args.rs"]
mod args;
#[path = "lle_gates/arm.rs"]
mod arm;
#[path = "lle_gates/emit.rs"]
mod emit;
#[path = "lle_gates/gates.rs"]
mod gates;
#[path = "lle_gates/load.rs"]
mod load;
#[path = "lle_gates/report.rs"]
mod report;
#[path = "lle_gates/run.rs"]
mod run;
#[path = "lle_gates/sched.rs"]
mod sched;
#[path = "lle_gates/sharc.rs"]
mod sharc;
#[path = "lle_gates/stats.rs"]
mod stats;
// the booting and arming of one core, shared with `lle_sharc_bench` (not copied)
#[path = "lle_sharc_bench/job.rs"]
#[allow(dead_code)]
mod core_job;

fn main() {
    let args = match args::parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("lle-gates: {e}");
            std::process::exit(2);
        }
    };
    let window = std::time::Duration::from_millis(args.load_window_ms);
    let out = run::run(&args, &|| load::sample(window));
    println!("{}", out.text);
    if let Some(path) = &args.json {
        if let Err(e) = std::fs::write(path, out.json.to_string()) {
            eprintln!("lle-gates: {}: {e}", path.display());
            std::process::exit(3);
        }
    }
    if let Some(path) = &args.uop_reports {
        if let Err(e) = std::fs::write(path, &out.uop_reports) {
            eprintln!("lle-gates: {}: {e}", path.display());
            std::process::exit(3);
        }
    }
    if out.code != 0 {
        std::process::exit(out.code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_usage_text_names_every_option_the_parser_takes() {
        for opt in [
            "--fw",
            "--bank",
            "--synthetic",
            "--only",
            "--engines",
            "--tiers",
            "--runs",
            "--passes",
            "--warmup",
            "--tick-insns",
            "--phase-scale",
            "--org-image",
            "--pno-image",
            "--eeprom",
            "--aot-tag-org",
            "--aot-tag-pno",
            "--uop-ranges",
            "--load-limit",
            "--load-window-ms",
            "--indicative",
            "--json",
            "--uop-reports",
        ] {
            assert!(args::USAGE.contains(opt), "{opt}");
        }
    }

    #[test]
    fn the_synthetic_tool_run_exits_zero_and_prints_a_table() {
        let a = args::parse_args(
            [
                "--synthetic",
                "--runs",
                "1",
                "--passes",
                "10",
                "--engines",
                "uop",
            ]
            .map(String::from),
        )
        .unwrap();
        let o = run::run(&a, &|| Some(1.0));
        assert_eq!(o.code, 0, "{}", o.text);
        assert!(o.text.contains("== synthetic poll core"));
    }
}
