//! Per filter type gain of the oracle filter over the native one, from the oracle table
//! `docs/research/data/synth-filter-gain-oracle.csv` (D194 capture, D195 reduction).
//!
//! Each row is one factory program run through the real ARM stream into ORG core 2 and through the
//! native engine, filter on and off (`tools/synth_e2e_oracle.py report`); the gap is the oracle
//! filtered / unfiltered RMS ratio minus the native one in dB. This module groups the rows by
//! filter model and reduces them to the table of `nord_core::synth::filter_gain`.

use nord_core::synth::filter_gain::filter_gain_db;
use nord_core::synth::FilterModel;
use std::collections::BTreeMap;

/// A program whose gap differs from its type median by more than this is reported and excluded from
/// the mean (the resonance 0.95 `Glow Chords Whl` row, `+15.2` against an LP12 median `+4.9`).
/// ASSUMED: 8 dB, any value between 5 and 10 gives the same set on the 18 rows.
pub const OUTLIER_DB: f64 = 8.0;

/// One oracle row.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub program: String,
    pub model: String,
    pub resonance: f64,
    pub cutoff_hz: f64,
    pub oracle_db: f64,
    pub native_db: f64,
    pub gap_db: f64,
}

/// Reduction of one filter type.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeGain {
    pub model: String,
    pub n: usize,
    pub mean_db: f64,
    pub min_db: f64,
    pub max_db: f64,
    pub std_db: f64,
    pub excluded: Vec<String>,
}

fn col(header: &[&str], name: &str) -> Result<usize, String> {
    header
        .iter()
        .position(|h| *h == name)
        .ok_or_else(|| format!("column {name} missing"))
}

/// Parses the CSV text. The stored `gap_db` must equal `oracle - native` within 0.02 dB.
pub fn parse_rows(text: &str) -> Result<Vec<Row>, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let head: Vec<&str> = lines.next().ok_or("empty csv")?.split(',').collect();
    let (p, m, r, c, o, n, g) = (
        col(&head, "program")?,
        col(&head, "filter_model")?,
        col(&head, "resonance")?,
        col(&head, "cutoff_hz")?,
        col(&head, "oracle_filter_gain_db")?,
        col(&head, "native_filter_gain_db")?,
        col(&head, "gap_db")?,
    );
    let mut rows = Vec::new();
    for l in lines {
        let f: Vec<&str> = l.split(',').collect();
        let num = |i: usize| -> Result<f64, String> {
            f.get(i)
                .ok_or_else(|| format!("short row: {l}"))?
                .parse()
                .map_err(|_| format!("bad number in {l}"))
        };
        let text = |i: usize| -> Result<String, String> {
            f.get(i)
                .map(|s| s.to_string())
                .ok_or_else(|| format!("short row: {l}"))
        };
        let row = Row {
            program: text(p)?,
            model: text(m)?,
            resonance: num(r)?,
            cutoff_hz: num(c)?,
            oracle_db: num(o)?,
            native_db: num(n)?,
            gap_db: num(g)?,
        };
        if (row.oracle_db - row.native_db - row.gap_db).abs() > 0.02 {
            return Err(format!("{}: gap_db is not oracle - native", row.program));
        }
        rows.push(row);
    }
    Ok(rows)
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

/// Groups by filter model (sorted by name), excluding outliers from the statistics.
pub fn reduce(rows: &[Row]) -> Vec<TypeGain> {
    let mut by: BTreeMap<&str, Vec<&Row>> = BTreeMap::new();
    for r in rows {
        by.entry(r.model.as_str()).or_default().push(r);
    }
    by.into_iter()
        .map(|(model, rs)| {
            let med = median(&mut rs.iter().map(|r| r.gap_db).collect::<Vec<_>>());
            let (keep, out): (Vec<&Row>, Vec<&Row>) = rs
                .iter()
                .copied()
                .partition(|r| (r.gap_db - med).abs() <= OUTLIER_DB);
            let g: Vec<f64> = keep.iter().map(|r| r.gap_db).collect();
            let n = g.len() as f64;
            let mean = g.iter().sum::<f64>() / n;
            let var = g.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
            TypeGain {
                model: model.to_string(),
                n: g.len(),
                mean_db: mean,
                min_db: g.iter().copied().fold(f64::INFINITY, f64::min),
                max_db: g.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                std_db: var.sqrt(),
                excluded: out.iter().map(|r| r.program.clone()).collect(),
            }
        })
        .collect()
}

/// The `nord-core` filter model of a CSV name.
pub fn model_of(name: &str) -> Option<FilterModel> {
    match name {
        "Lp12" => Some(FilterModel::Lp12),
        "Lp24" => Some(FilterModel::Lp24),
        "Ladder24" => Some(FilterModel::Ladder24),
        "BandPass" => Some(FilterModel::BandPass),
        "HighPass" => Some(FilterModel::HighPass),
        _ => None,
    }
}

/// Compiled-in table value minus the measured mean (dB), per type.
pub fn table_error(g: &TypeGain) -> Option<f64> {
    model_of(&g.model).map(|m| f64::from(filter_gain_db(m)) - g.mean_db)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &str = "program,filter_model,resonance,cutoff_hz,oracle_filter_gain_db,native_filter_gain_db,gap_db\n\
        a,Lp12,0.1,300,4.0,-1.0,5.0\n\
        b,Lp12,0.2,300,3.0,-1.0,4.0\n\
        c,Lp12,0.95,300,-7.0,-22.0,15.0\n\
        d,BandPass,0.5,100,-2.0,-2.0,0.0\n";

    #[test]
    fn rows_parse_and_the_gap_is_checked() {
        let rows = parse_rows(CSV).unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].program, "a");
        let bad = CSV.replace("5.0\n", "9.0\n");
        assert!(parse_rows(&bad)
            .unwrap_err()
            .contains("not oracle - native"));
        assert!(parse_rows("program\nx\n").is_err());
    }

    #[test]
    fn reduction_excludes_the_outlier_from_the_mean() {
        let g = reduce(&parse_rows(CSV).unwrap());
        let lp = g.iter().find(|t| t.model == "Lp12").unwrap();
        assert_eq!(lp.n, 2);
        assert!((lp.mean_db - 4.5).abs() < 1e-12);
        assert_eq!((lp.min_db, lp.max_db), (4.0, 5.0));
        assert!((lp.std_db - 0.5).abs() < 1e-12);
        assert_eq!(lp.excluded, vec!["c".to_string()]);
        let bp = g.iter().find(|t| t.model == "BandPass").unwrap();
        assert_eq!((bp.n, bp.mean_db), (1, 0.0));
    }

    #[test]
    fn table_matches_the_oracle_csv() {
        let text = include_str!("../../../docs/research/data/synth-filter-gain-oracle.csv");
        let g = reduce(&parse_rows(text).unwrap());
        let counts: Vec<(String, usize)> = g.iter().map(|t| (t.model.clone(), t.n)).collect();
        assert_eq!(
            counts,
            [
                ("BandPass", 1),
                ("HighPass", 4),
                ("Ladder24", 4),
                ("Lp12", 5),
                ("Lp24", 7)
            ]
            .map(|(m, n)| (m.to_string(), n))
        );
        for t in &g {
            let e = table_error(t).unwrap();
            assert!(e.abs() < 0.06, "{}: table off by {e} dB", t.model);
        }
        let lp12 = g.iter().find(|t| t.model == "Lp12").unwrap();
        assert_eq!(lp12.excluded, vec!["Glow Chords Whl".to_string()]);
    }

    #[test]
    fn model_names_map_to_filter_models() {
        assert_eq!(model_of("Lp24"), Some(FilterModel::Lp24));
        assert_eq!(model_of("nope"), None);
    }
}
