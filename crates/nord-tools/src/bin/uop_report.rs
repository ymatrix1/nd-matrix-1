//! Deterministic counter report of the SHARC uop engine (D260, `docs/specs/sharc-uop.md`).
//!
//!     cargo run --release -p nord-tools --bin uop-report -- [--steps N] [--label TEXT]
//!
//! Arms the engine on a straight-line synthetic program (NOPs, no firmware), runs N steps
//! (default 1000) and prints one JSON object with the counters and the modeled cycles. The field
//! names are stable (`runner/uop_report.rs`, schema 1): `tools/uop_gates.py` reads this output
//! and evaluates the structural gates. The report carries counts only, never a time, so two runs
//! print identical text. With real firmware the same report comes from the pass runner that
//! owns the core; this binary is the schema and wiring check.

use nord_sharc::core::{Core, UopConfig};
use nord_sharc::runner::uop_report::UopReport;

/// Short-word address of the program (the VISA code window).
const CODE: u32 = 0x12_0000;

/// `(steps, label)` from the command line, or the usage text.
fn parse_args(args: impl Iterator<Item = String>) -> Result<(u32, String), String> {
    let (mut steps, mut label) = (1000u32, String::from("synthetic nop"));
    let mut args = args;
    while let Some(a) = args.next() {
        match (a.as_str(), args.next()) {
            ("--steps", Some(v)) => steps = v.parse().map_err(|_| format!("bad --steps {v}"))?,
            ("--label", Some(v)) => label = v,
            _ => return Err("usage: uop-report [--steps N] [--label TEXT]".into()),
        }
    }
    Ok((steps, label))
}

/// Runs `steps` NOPs under the armed engine and returns the report JSON.
fn report(steps: u32, label: &str) -> Result<String, String> {
    let n = steps as usize + 8;
    let mut image = vec![0u8; CODE as usize * 2 + n * 2];
    for k in 0..n {
        let o = (CODE as usize + k) * 2;
        image[o..o + 2].copy_from_slice(&0x0001u16.to_le_bytes()); // NOP
    }
    let mut core = Core::new(&image);
    core.pc = CODE;
    core.enable_uop(UopConfig::default(), &[(CODE, CODE + n as u32)])
        .map_err(|e| e.to_string())?;
    if let Some(fault) = core.run_uop(u64::from(steps)) {
        return Err(format!("fault {fault:?}"));
    }
    Ok(UopReport::from_core(label, &core).to_json())
}

fn main() {
    let result =
        parse_args(std::env::args().skip(1)).and_then(|(steps, label)| report(steps, &label));
    match result {
        Ok(json) => println!("{json}"),
        Err(e) => {
            eprintln!("uop-report: {e}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> impl Iterator<Item = String> {
        v.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn arguments_default_parse_and_reject_unknown_flags() {
        assert_eq!(parse_args(args(&[])), Ok((1000, "synthetic nop".into())));
        assert_eq!(
            parse_args(args(&["--steps", "50", "--label", "x"])),
            Ok((50, "x".into()))
        );
        assert!(parse_args(args(&["--steps", "many"])).is_err());
        assert!(parse_args(args(&["--bogus"])).is_err());
    }

    #[test]
    fn the_report_is_deterministic_and_counts_every_step_as_a_core_step_in_the_skeleton() {
        let a = report(40, "t").unwrap();
        assert_eq!(a, report(40, "t").unwrap());
        assert!(
            a.contains("\"schema\":1") && a.contains("\"steps_core_step\":40"),
            "{a}"
        );
    }
}
