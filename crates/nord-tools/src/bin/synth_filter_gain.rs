//! Reduces the oracle filter table to a gain per filter type and checks the compiled-in table of
//! `nord_core::synth::filter_gain` against it (D195).
//!
//! Usage: `synth_filter_gain [oracle.csv]` (default `docs/research/data/synth-filter-gain-oracle.csv`).
//! Prints one line per filter type: programs, mean gap, spread, the table value and the
//! difference; exits with an error when a type is off by more than 0.06 dB.
//! The oracle capture itself is `tools/synth_e2e_oracle.py report` (unchanged, D132 / D194).

use nord_tools::synth_filter_gain::{parse_rows, reduce, table_error};

fn main() -> Result<(), String> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "docs/research/data/synth-filter-gain-oracle.csv".into());
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    let gains = reduce(&parse_rows(&text)?);
    println!("type      n   mean   min    max    std   table  diff  excluded");
    let mut worst = 0.0f64;
    for g in &gains {
        let err = table_error(g);
        worst = worst.max(err.map_or(0.0, f64::abs));
        println!(
            "{:<9}{:>2}{:>7.2}{:>7.2}{:>7.2}{:>6.2}{:>7.2}{:>6.2}  {}",
            g.model,
            g.n,
            g.mean_db,
            g.min_db,
            g.max_db,
            g.std_db,
            err.map_or(f64::NAN, |e| g.mean_db + e),
            err.unwrap_or(f64::NAN),
            g.excluded.join(";")
        );
    }
    if worst > 0.06 {
        return Err(format!("table differs from the oracle by {worst:.2} dB"));
    }
    Ok(())
}
