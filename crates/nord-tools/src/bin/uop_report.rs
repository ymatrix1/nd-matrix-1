//! Deterministic counter report of the SHARC uop engine (D260, `docs/specs/sharc-uop.md`).
//!
//! ```text
//! cargo run --release -p nord-tools --bin uop-report -- [--passes N] [--warmup N] [--label TEXT]
//!     [--engine uop|uop-checked|uop-native] [--uop-families BITS] [--ranges extent|probe]
//!     [--image FILE --preset NAME [--msgs FILE]]
//! ```
//!
//! Boots a core, arms the engine (`runner::uop_pass::arm_for`: the code of the booted core is
//! translated, the pass idle pc is the stop pc), runs `--warmup` plus `--passes` passes on this
//! thread and prints one JSON object with the counters of the measured passes (`"window":
//! "measured"`: the warm-up is reset away) and the modeled cycles. The field names are stable
//! (`runner/uop_report`, schema 1): `tools/uop_gates.py` reads this output and evaluates the
//! structural gates. The report carries counts only, never a time, so two runs print identical
//! text. On stderr, the build line, the one-line summary and the block-length histogram (steps
//! per block entry, also in the JSON as `block_steps`).
//!
//! Without `--image` the core is the synthetic poll program of `runner::uop_report::measure` (ten
//! NOPs and a jump, 11 steps per pass, no firmware): the schema and wiring check. With
//! `--image FILE --preset NAME` (`pno`, `org`, ... as in `lle_sharc_bench`; the image is the
//! user's own raw core SRAM, never committed) the same loop reports the real pass of that core,
//! e.g. ORG Core2 for gate G2. `--msgs FILE` delivers ARM messages as `lle_sharc_bench` does. The
//! default engine is `uop`; `uop-native` needs `--features nord-sharc/uop-native`. `--ranges`
//! picks the code to translate: `extent` (default, the non-zero extent of the VISA window: it also
//! takes data tables for code, so stores into them count as `smc`) or `probe` (the pcs of
//! `warmup + passes` interpreter passes on a second boot of the same core: no data page, so G2
//! `smc` can be 0; code the probe never ran stays in the interpreter).
//!
//! `--engine uop-checked` replays every pass on the interpreter and fails (exit 2) on a difference.
//! A core with peripheral models cannot be replayed; such passes are not compared and are counted
//! apart (`"replay"` in the JSON), because a run that compared nothing has shown nothing.
//!
//! Exit code 0 on success, 2 on a usage error, a boot or arming failure or a difference found by
//! `uop-checked`, 3 when a pass did not return to its poll loop or `uop-checked` could not compare
//! a pass (the report is still printed: its counters are not a steady state, or not a check).

// the command line, and the run behind it
#[path = "uop_report/args.rs"]
mod args;
#[path = "uop_report/run.rs"]
mod run;

use args::parse_args;
use run::{run, unsound, Run};

/// What the tool prints and how it exits.
#[derive(Debug, PartialEq, Eq)]
struct Output {
    /// The JSON line for stdout, when there is a report.
    stdout: Option<String>,
    /// The lines for stderr, each printed after `uop-report: `.
    stderr: Vec<String>,
    code: i32,
}

/// The output and exit code of an outcome (see the exit codes of the header).
fn render(outcome: Result<Run, String>) -> Output {
    match outcome {
        Ok(Run {
            measured,
            note,
            histogram,
        }) => {
            let mut stderr = vec![note];
            stderr.extend(histogram);
            if let Some(e) = &measured.msg_error {
                stderr.push(format!("message not delivered: {e}"));
            }
            let why = unsound(&measured);
            let code = if why.is_some() { 3 } else { 0 };
            stderr.extend(why);
            Output {
                stdout: Some(measured.report.to_json()),
                stderr,
                code,
            }
        }
        Err(e) => Output {
            stdout: None,
            stderr: vec![e],
            code: 2,
        },
    }
}

fn main() {
    let out = render(parse_args(std::env::args().skip(1)).and_then(|a| run(&a)));
    if let Some(json) = &out.stdout {
        println!("{json}");
    }
    for line in &out.stderr {
        eprintln!("uop-report: {line}");
    }
    if out.code != 0 {
        std::process::exit(out.code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use args::tests::args;
    use nord_sharc::runner::uop_report::measure::Measured;

    fn outcome(a: &[&str]) -> Result<Run, String> {
        parse_args(args(a)).and_then(|a| run(&a))
    }

    #[test]
    fn a_good_run_prints_the_json_and_exits_zero() {
        let o = render(outcome(&["--passes", "3"]));
        assert_eq!(o.code, 0);
        let json = o.stdout.expect("the report");
        assert!(json.starts_with("{\"schema\":1,") && json.contains("\"window\":\"measured\""));
        assert!(o.stderr[0].contains("built"), "{:?}", o.stderr);
        assert!(
            o.stderr
                .iter()
                .any(|l| l.starts_with("steps per block entry")),
            "the histogram is on stderr: {:?}",
            o.stderr
        );
    }

    #[test]
    fn a_usage_error_exits_two_and_prints_no_report() {
        for bad in [
            &["--bogus"][..],
            &["--passes", "0"],
            &["--engine", "interp"],
        ] {
            let o = render(outcome(bad));
            assert_eq!((o.stdout, o.code), (None, 2), "{bad:?}");
            assert!(!o.stderr.is_empty());
        }
    }

    #[test]
    fn a_run_that_is_not_a_steady_state_exits_three_and_still_prints_the_report() {
        let mut run = outcome(&["--passes", "2"]).unwrap();
        run.measured = Measured {
            all_idle: false,
            ..run.measured
        };
        let o = render(Ok(run));
        assert_eq!(o.code, 3);
        assert!(o.stdout.is_some(), "the counters are printed all the same");
        assert!(
            o.stderr.last().unwrap().contains("poll loop"),
            "{:?}",
            o.stderr
        );
    }

    #[test]
    fn an_undelivered_message_is_said_on_stderr() {
        let mut run = outcome(&["--passes", "2"]).unwrap();
        run.measured.msg_error = Some("no such address".into());
        let o = render(Ok(run));
        assert_eq!(o.code, 0, "the pass itself was a steady state");
        assert!(
            o.stderr
                .iter()
                .any(|l| l == "message not delivered: no such address"),
            "{:?}",
            o.stderr
        );
    }
}
