//! OWNER: W1-CNT. Deterministic counters of the ARM0 boot and phases for the tier decisions
//! (gate G0 of `docs/research/arm0-jit-design.md`, section 13).
//!
//!     cargo run --release -p nord-tools --bin arm-sb-bench -- \
//!         <bank0_CODE.bin> <bank0_SRAM.bin> <eeprom_image_256k.bin> [--golden <file>] [--time]
//!
//! Environment (as `arm-lle-bench`): `TICK_INSNS` (virtual instructions per 1 ms tick, default
//! 200000), `PHASE_SCALE` (percent of the phase lengths, default 100), `PREDECODE=0` (plain
//! `Cpu::step`, the oracle tier).
//!
//! Boots the OS v1.66 image with the full board, then runs the six phases of `arm-lle-bench`
//! (load, idle, key1 on, key1 off, key8 on, key8 off) twice from the same boot state:
//!
//! * 1 ms framing: ticks of `TICK_INSNS` virtual instructions, one timer interrupt each. Its state
//!   hashes and message digests are the golden check.
//! * 24 kHz framing: the same virtual time in `FRAME_DIVISOR` times as many ticks of
//!   `TICK_INSNS / FRAME_DIVISOR` instructions, so the executed instructions per tick at that rate
//!   are measured, not divided (gate G0 item c).
//!
//! Per phase and framing the table prints `probe_executed / executed` (G0 a), the executed
//! instructions per tick and the largest tick, probes and handler instructions per tick (the fixed
//! work of a tick that is not in `executed`) and the executed instructions per second at that tick
//! rate. `--time` adds the thread CPU time per tick and the fixed tick cost, `tick time - executed
//! insns x 5.3 ns` (G0 b); that column is non-deterministic and is not part of the golden check.
//!
//! Golden check: `--golden <file>` compares the `digest ...` lines of this run with those of the
//! file (a whole earlier output is a valid file, see `golden.rs`). Exit codes: 0 ok, 1 digest
//! mismatch, 2 bad arguments, 3 an input could not be read or a pass failed. Uses main APIs only
//! (`Machine`, `IdleStats`), so it runs on `main` and on this branch alike.

// The submodules live next to this file in `arm_sb_bench/` (a crate root resolves `mod x;`
// beside itself, so the path is spelled out).
#[path = "arm_sb_bench/args.rs"]
mod args;
#[path = "arm_sb_bench/golden.rs"]
mod golden;
#[path = "arm_sb_bench/measure.rs"]
mod measure;
#[path = "arm_sb_bench/report.rs"]
mod report;
#[path = "arm_sb_bench/rows.rs"]
mod rows;

use args::{parse_args, Args, FRAME_DIVISOR};
use nord_arm::machine::Machine;
use nord_sharc::cpu_clock::ThreadClock;
use rows::{BootInfo, Row};
use std::io::Write;

const EXIT_MISMATCH: i32 = 1;
const EXIT_USAGE: i32 = 2;
const EXIT_FAILED: i32 = 3;

fn main() {
    let code = run(
        std::env::args().skip(1),
        &|k| std::env::var(k).ok(),
        &mut std::io::stdout().lock(),
    );
    std::process::exit(code);
}

/// Parse, read the inputs, boot and measure; returns the exit code.
fn run(
    argv: impl IntoIterator<Item = String>,
    env: &dyn Fn(&str) -> Option<String>,
    out: &mut dyn Write,
) -> i32 {
    let args = match parse_args(argv, env) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return EXIT_USAGE;
        }
    };
    let read = |p: &std::path::Path| std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()));
    // everything that can fail cheaply fails before the boot
    let golden = match args.golden.as_deref().map(std::fs::read_to_string) {
        None => None,
        Some(Ok(text)) => Some(text),
        Some(Err(e)) => {
            eprintln!("arm-sb-bench: golden file: {e}");
            return EXIT_FAILED;
        }
    };
    let inputs = read(&args.code).and_then(|c| Ok((c, read(&args.sram)?, read(&args.eeprom)?)));
    let (code, sram, eeprom) = match inputs {
        Ok(i) => i,
        Err(e) => {
            eprintln!("arm-sb-bench: {e}");
            return EXIT_FAILED;
        }
    };
    match measure::boot(&code, &sram, &eeprom, args.predecode) {
        Ok((m, boot)) => execute(m, boot, &args, golden.as_deref(), out),
        Err(e) => {
            eprintln!("arm-sb-bench: boot failed: {e}");
            EXIT_FAILED
        }
    }
}

/// Run both passes on the booted machine, print the report and check the golden digests.
fn execute(
    mut m: Machine,
    boot: BootInfo,
    args: &Args,
    golden: Option<&str>,
    out: &mut dyn Write,
) -> i32 {
    let (budget, timed) = (args.tick_insns, args.time);
    let mut m24 = measure::fork(&m);
    let _ = writeln!(
        out,
        "arm-sb-bench: counters, tick_insns={budget} phase_scale={} predecode={}",
        args.phase_scale,
        if args.predecode { "on" } else { "off" }
    );
    if timed {
        let clock = if ThreadClock::is_cpu_time() {
            "thread CPU time"
        } else {
            "wall time"
        };
        let _ = writeln!(
            out,
            "--time: {clock} per tick (non-deterministic, not part of the golden check)"
        );
    }
    let _ = writeln!(out, "{}", report::boot_line(&boot));
    let ms = match measure::ms_pass(&mut m, args.phase_scale, budget, timed) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("arm-sb-bench: 1 ms pass failed: {e}");
            return EXIT_FAILED;
        }
    };
    let rows: Vec<Row> = ms.iter().map(|o| o.row.clone()).collect();
    let title = format!("framing 1 ms: ticks of {budget} virtual insns (tick rate 1 kHz)");
    let _ = write!(out, "{}", report::table(&title, &rows, 1_000, timed));
    let digests = report::digest_lines(budget, args.phase_scale, &boot, &ms);
    for l in &digests {
        let _ = writeln!(out, "{l}");
    }
    let mut code = 0;
    if let Some(text) = golden {
        match golden::compare(text, &digests) {
            Ok(()) => {
                let _ = writeln!(out, "golden: {} digest lines equal", digests.len());
            }
            Err(diffs) => {
                for d in &diffs {
                    let _ = writeln!(out, "golden MISMATCH {d}");
                }
                eprintln!(
                    "arm-sb-bench: {} digest line(s) differ from the golden file",
                    diffs.len()
                );
                code = EXIT_MISMATCH;
            }
        }
    }
    match measure::khz24_pass(&mut m24, args.phase_scale, budget, timed) {
        Ok(rows) => {
            let title = format!(
                "framing 24 kHz: same virtual time, ticks of {} virtual insns (tick rate 24 kHz)",
                budget / FRAME_DIVISOR
            );
            let _ = write!(out, "{}", report::table(&title, &rows, 24_000, timed));
        }
        Err(e) => {
            eprintln!("arm-sb-bench: 24 kHz pass failed: {e}");
            if code == 0 {
                code = EXIT_FAILED;
            }
        }
    }
    let _ = writeln!(out, "exec/tick: IdleStats.executed per tick, probe instructions included; irq/tick: handler instructions per tick, not in executed; Minsn/s: exec/tick x tick rate");
    code
}

#[cfg(test)]
mod tests {
    use super::*;
    use measure::spin_machine;

    fn config(time: bool) -> Args {
        Args {
            code: "c".into(),
            sram: "s".into(),
            eeprom: "e".into(),
            tick_insns: 10_000,
            phase_scale: 1,
            predecode: true,
            golden: None,
            time,
        }
    }

    fn boot_of(m: &Machine) -> BootInfo {
        BootInfo {
            stats: Default::default(),
            virtual_insns: 0,
            irqs: 0,
            pc: m.cpu.pc(),
            state: m.state_hash(),
        }
    }

    /// Run `execute` on the spin machine; returns the exit code and the output text.
    fn go(golden: Option<&str>, time: bool) -> (i32, String) {
        let m = spin_machine();
        let boot = boot_of(&m);
        let mut out = Vec::new();
        let code = execute(m, boot, &config(time), golden, &mut out);
        (code, String::from_utf8(out).unwrap())
    }

    #[test]
    fn the_report_has_the_boot_line_both_tables_and_the_digest_lines() {
        let (code, text) = go(None, false);
        assert_eq!(code, 0);
        assert!(
            text.starts_with(
                "arm-sb-bench: counters, tick_insns=10000 phase_scale=1 predecode=on\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("\nboot: main loop at pc 0xc0000000"),
            "{text}"
        );
        assert!(text.contains("framing 1 ms: ticks of 10000 virtual insns"));
        assert!(text.contains("framing 24 kHz: same virtual time, ticks of 416 virtual insns"));
        // 2 header digest lines + 2 per phase; one row per phase in each table
        assert_eq!(golden::digest_lines(&text).len(), 2 + 2 * 6);
        for name in ["load", "idle", "key1 on", "key1 off", "key8 on", "key8 off"] {
            assert_eq!(
                text.lines().filter(|l| l.starts_with(name)).count(),
                2,
                "{name}"
            );
        }
        assert!(!text.contains("fixed us"), "no timing without --time");
    }

    #[test]
    fn two_untimed_runs_print_identical_bytes() {
        assert_eq!(go(None, false), go(None, false));
    }

    #[test]
    fn a_run_passes_its_own_output_as_the_golden_file() {
        let (_, first) = go(None, false);
        let (code, second) = go(Some(&first), false);
        assert_eq!(code, 0);
        assert!(second.contains("golden: 14 digest lines equal"), "{second}");
    }

    #[test]
    fn a_digest_mismatch_exits_non_zero_and_names_the_line() {
        let (_, first) = go(None, false);
        // flip one hex digit of the state hash of the load phase
        let mut bad = String::new();
        for l in first.lines() {
            bad += &if l.starts_with("digest load state ") {
                let (head, last) = l.split_at(l.len() - 1);
                format!("{head}{}\n", if last == "0" { "1" } else { "0" })
            } else {
                format!("{l}\n")
            };
        }
        assert_ne!(bad, first);
        let (code, text) = go(Some(&bad), false);
        assert_eq!(code, EXIT_MISMATCH);
        assert!(
            text.contains("golden MISMATCH line 3: golden `digest load state"),
            "{text}"
        );
        // the mismatch report does not start with `digest`, so it never pollutes a golden file
        assert_eq!(golden::digest_lines(&text).len(), 14);
        // a different configuration is a mismatch on the first line
        let other = first.replace("tick_insns=10000", "tick_insns=20000");
        let (code, text) = go(Some(&other), false);
        assert_eq!(code, EXIT_MISMATCH);
        assert!(text.contains("golden MISMATCH line 1:"), "{text}");
        // a golden file without digest lines never passes
        assert_eq!(go(Some("nothing"), false).0, EXIT_MISMATCH);
    }

    #[test]
    fn the_time_option_adds_the_fixed_cost_columns_and_keeps_the_digests() {
        let (code, timed) = go(None, true);
        assert_eq!(code, 0);
        assert!(
            timed.contains("--time:") && timed.contains("fixed us"),
            "{timed}"
        );
        let (_, plain) = go(None, false);
        assert_eq!(golden::digest_lines(&timed), golden::digest_lines(&plain));
    }

    #[test]
    fn a_failing_pass_is_exit_3_and_prints_no_digests() {
        let mut m = spin_machine();
        m.stop_pc = Some(0xC000_0000);
        let boot = boot_of(&m);
        let mut out = Vec::new();
        assert_eq!(
            execute(m, boot, &config(false), None, &mut out),
            EXIT_FAILED
        );
        // nothing is printed as a result of a pass that did not run
        assert!(!String::from_utf8(out).unwrap().contains("digest "));
    }

    #[test]
    fn bad_arguments_and_unreadable_inputs_are_told_apart_from_a_mismatch() {
        let no_env = |_: &str| None;
        let mut out = Vec::new();
        let cli =
            |a: &[&str], out: &mut Vec<u8>| run(a.iter().map(|s| s.to_string()), &no_env, out);
        assert_eq!(cli(&["only-one"], &mut out), EXIT_USAGE);
        assert_eq!(cli(&["--bogus", "c", "s", "e"], &mut out), EXIT_USAGE);
        let missing = "target/does/not/exist.bin";
        assert_eq!(cli(&[missing, missing, missing], &mut out), EXIT_FAILED);
        assert_eq!(
            cli(&["--golden", missing, missing, missing, missing], &mut out),
            EXIT_FAILED
        );
        assert!(
            out.is_empty(),
            "nothing is measured before the inputs are read"
        );
    }
}
