//! `fx_parity_null`: null depth of the engine FX path and of the `FirmwareFx::PARITY` preset
//! against the firmware model (the single port, oracle-checked word for word) for every FX case of
//! `fx_cases` that has a port, on one stimulus (`--stim noise|sweep|impulse|notes`, default noise).
//! One CSV line per case: `family,case,port,engine_null_db,parity_null_db`, then a summary per
//! family on stderr. Numbers only (`docs/research/program-ranking-fx.md`).

use nord_tools::fx_cases::cases;
use nord_tools::fx_parity::nulls;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Frames per case (a power of two, 1.5 s at the 43.4 kHz DSP rate).
/// ASSUMED: tool convention; long enough for the Mod 1 / Mod 2 LFOs to move the output.
const FRAMES: usize = 1 << 16;

/// Per-family minimum and maximum of the two null depths.
fn summary(rows: &[(String, f64, f64)]) -> String {
    let mut by: BTreeMap<&str, (f64, f64, f64, f64, usize)> = BTreeMap::new();
    for (fam, e, p) in rows {
        let s = by
            .entry(fam.as_str())
            .or_insert((f64::MAX, f64::MIN, f64::MAX, f64::MIN, 0));
        *s = (s.0.min(*e), s.1.max(*e), s.2.min(*p), s.3.max(*p), s.4 + 1);
    }
    let mut out = String::new();
    for (fam, (emin, emax, pmin, pmax, n)) in by {
        let _ = writeln!(
            out,
            "{fam}: {n} cases, engine null {emin:.1} .. {emax:.1} dB, parity null {pmin:.1} .. {pmax:.1} dB"
        );
    }
    out
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let stim = a
        .iter()
        .position(|s| s == "--stim")
        .and_then(|i| a.get(i + 1))
        .map_or("noise", String::as_str);
    println!("family,case,port,engine_null_db,parity_null_db");
    let mut rows = Vec::new();
    for c in cases().iter().filter(|c| !c.port.is_empty()) {
        let (e, p) = nulls(c, stim, FRAMES);
        println!("{},{},{},{e:.1},{p:.1}", c.family, c.label, c.port);
        rows.push((c.family.to_string(), e, p));
    }
    eprint!("{}", summary(&rows));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_reports_min_and_max_per_family() {
        let rows = vec![
            ("mod2".to_string(), -3.0, -300.0),
            ("mod2".to_string(), -1.0, -250.0),
            ("eq".to_string(), -7.0, -307.0),
        ];
        let s = summary(&rows);
        assert!(
            s.contains("eq: 1 cases, engine null -7.0 .. -7.0 dB, parity null -307.0 .. -307.0")
        );
        assert!(
            s.contains("mod2: 2 cases, engine null -3.0 .. -1.0 dB, parity null -300.0 .. -250.0")
        );
    }
}
