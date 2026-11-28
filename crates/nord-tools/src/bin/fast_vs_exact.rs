//! `fast_vs_exact` (bean `ns4-lle3`): error of the fast-f32 whole-pass module against the exact
//! whole-pass module on a real SHARC core, pass by pass.
//!
//! ```text
//! cargo run --release -p nord-tools --bin fast_vs_exact -- \
//!   --image IMG --preset org|pno [--msgs FILE] --exact ORG2 --fast ORG2F \
//!   [--passes 300] [--warmup 16] [--csv FILE] [--scan LO:HI]
//! ```
//!
//! Two cores boot from the same image and preset and get the same ARM messages before the same
//! passes (`PassJob`, as in `lle_sharc_bench`); one runs the exact module (`--exact TAG`), the other
//! the approximate-kernel module (`--fast TAG`, not bit-exact: host `f32` arithmetic). After every
//! pass it compares, as `f32` words with non-finite words counted as 0:
//!
//! - **lanes**: the output lane words, 12 per set at the pointer in `DM(0xB0159 + set)`, sets 0..7;
//! - **buffers**: the pass buffers `DM(0xB0000..0xB2FFF)`;
//! - **state**: every data word in `--scan` (default `0x80000..0x100000`) outside those buffers,
//!   the module state the next pass starts from (so error that does not reach the output yet).
//!
//! Only words that read as floats of ordinary size (zero, or magnitude 1e-12..1e9) enter the
//! energies and errors; integer, pointer and flag words that differ are counted separately.
//!
//! Per group and pass: max abs error, error and signal energy (SNR in dB relative to the exact
//! output) and the number of bitwise-different words; plus the first diverging pass, whether the
//! step count (control flow) differs, and quartile SNRs for drift. Provenance: EMPIRICAL (a
//! measurement of one preset and message script, not a proof for other input).
use nord_sharc::runner::aot_audio::snr_db;
use nord_sharc::runner::live_presets::{boot_core, preset_named};
use nord_sharc::runner::msg::parse_msg_text;
use nord_sharc::runner::pass_sched::{Engine, PassJob};
use nord_sharc::runner::periph_run::PeriphOpts;
use std::fmt::Write as _;

/// Output lane pointer of set 0 (`sharc-pass --lane`), and the number of sets and words read.
const OUT_POINTERS: u32 = 0xB_0159;
const OUT_SETS: u32 = 8;
const LANE_WORDS: u32 = 12;
/// The pass buffers.
const BUFFERS: (u32, u32) = (0xB_0000, 0xB_3000);

/// Error of one group of words after one pass.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Err1 {
    /// Energy of the exact words and of the difference.
    sig: f64,
    err: f64,
    max_abs: f64,
    /// Largest `|error| / |exact|` over float words with an exact magnitude of at least 1e-12.
    max_rel: f64,
    /// Words that differ bitwise, the part of them outside the float domain (integers, pointers,
    /// flags: control state), and the words compared.
    diff: u64,
    other_diff: u64,
    /// Words where the fast core holds a NaN or infinity and the cores differ. Counted for every
    /// word, also when the exact word is outside the float domain (the energies map non-finite to 0).
    nonfinite_fast: u64,
    n: u64,
}

impl Err1 {
    fn add(&mut self, exact: u32, fast: u32) {
        self.n += 1;
        self.diff += u64::from(exact != fast);
        self.nonfinite_fast += u64::from(exact != fast && !f32::from_bits(fast).is_finite());
        let e = finite(exact);
        if !in_domain(exact) {
            // not a float word of the module: counted, kept out of the energies
            self.other_diff += u64::from(exact != fast);
            return;
        }
        let d = e - finite(fast);
        self.sig += e * e;
        self.err += d * d;
        self.max_abs = self.max_abs.max(d.abs());
        if e.abs() >= 1e-12 {
            self.max_rel = self.max_rel.max(d.abs() / e.abs());
        }
    }

    fn merge(&mut self, o: &Err1) {
        self.sig += o.sig;
        self.err += o.err;
        self.max_abs = self.max_abs.max(o.max_abs);
        self.max_rel = self.max_rel.max(o.max_rel);
        self.diff += o.diff;
        self.other_diff += o.other_diff;
        self.nonfinite_fast += o.nonfinite_fast;
        self.n += o.n;
    }

    fn snr(&self) -> f64 {
        snr_db(self.sig, self.err)
    }
}

/// A word as `f32`, non-finite as 0 (the buffers mix floats with integers and flags).
fn finite(w: u32) -> f64 {
    let f = f32::from_bits(w);
    if f.is_finite() {
        f64::from(f)
    } else {
        0.0
    }
}

/// Whether the word reads as a float state value: zero, or a finite `f32` with a magnitude in
/// `1e-12..=1e9`. Integers, pointers and flags read as denormals or huge values and are left out of
/// the energies (their differences are still counted).
fn in_domain(w: u32) -> bool {
    let f = f32::from_bits(w);
    f == 0.0 || (f.is_finite() && (1e-12..=1e9).contains(&f.abs()))
}

/// What one pass measured.
#[derive(Clone, Copy, Debug, Default)]
struct PassRow {
    steps: (u64, u64),
    /// Data-memory reads that failed on either core (read as 0).
    unreadable: u64,
    lanes: Err1,
    bufs: Err1,
    state: Err1,
}

/// One data-memory word; a failed read counts in `bad` and reads as 0 (reported, never silent).
fn read(job: &mut PassJob, addr: u32, bad: &mut u64) -> u32 {
    job.core.dm_read(addr).unwrap_or_else(|_| {
        *bad += 1;
        0
    })
}

fn words(job: &mut PassJob, lo: u32, hi: u32, bad: &mut u64) -> Vec<u32> {
    (lo..hi).map(|a| read(job, a, bad)).collect()
}

fn lanes(job: &mut PassJob, bad: &mut u64) -> Vec<u32> {
    let mut out = Vec::new();
    for set in 0..OUT_SETS {
        let base = read(job, OUT_POINTERS + set, bad);
        out.extend((0..LANE_WORDS).map(|k| read(job, base.wrapping_add(k), bad)));
    }
    out
}

fn compare(a: &[u32], b: &[u32]) -> Err1 {
    let mut e = Err1::default();
    a.iter().zip(b).for_each(|(&x, &y)| e.add(x, y));
    e
}

fn measure(ex: &mut PassJob, fa: &mut PassJob, scan: (u32, u32), steps: (u64, u64)) -> PassRow {
    let mut bad = 0;
    let mut state = Err1::default();
    // the scan skips the pass buffers, which are their own group
    for (lo, hi) in [
        (scan.0, BUFFERS.0.max(scan.0)),
        (BUFFERS.1.max(scan.0), scan.1),
    ] {
        let hi = hi.min(scan.1);
        if lo < hi {
            let (a, b) = (words(ex, lo, hi, &mut bad), words(fa, lo, hi, &mut bad));
            state.merge(&compare(&a, &b));
        }
    }
    let lanes = compare(&lanes(ex, &mut bad), &lanes(fa, &mut bad));
    let (a, b) = (
        words(ex, BUFFERS.0, BUFFERS.1, &mut bad),
        words(fa, BUFFERS.0, BUFFERS.1, &mut bad),
    );
    PassRow {
        steps,
        unreadable: bad,
        lanes,
        bufs: compare(&a, &b),
        state,
    }
}

/// First pass index whose group `pick` has a differing word.
/// The `n` state words with the largest relative error after the last pass, as text lines
/// `address exact fast rel` (words outside the float domain are listed as raw hex).
fn top_diffs(ex: &mut PassJob, fa: &mut PassJob, scan: (u32, u32), n: usize) -> String {
    let mut bad = 0;
    let (a, b) = (
        words(ex, scan.0, scan.1, &mut bad),
        words(fa, scan.0, scan.1, &mut bad),
    );
    let mut d: Vec<(f64, u32, u32, u32)> = (0..a.len())
        .filter(|&i| a[i] != b[i] && !(BUFFERS.0..BUFFERS.1).contains(&(scan.0 + i as u32)))
        .map(|i| {
            let (e, f) = (f32::from_bits(a[i]), f32::from_bits(b[i]));
            let rel = if e.is_finite() && f.is_finite() && e != 0.0 {
                f64::from((e - f).abs() / e.abs())
            } else {
                f64::INFINITY
            };
            (rel, scan.0 + i as u32, a[i], b[i])
        })
        .collect();
    d.sort_by(|x, y| y.0.total_cmp(&x.0));
    let mut out = format!(
        "{} differing state words after the last pass; largest relative error (as f32):
",
        d.len()
    );
    for (rel, addr, e, f) in d.into_iter().take(n) {
        let _ = writeln!(
            out,
            "  0x{addr:x}: exact {e:#010x} ({}) fast {f:#010x} ({}) rel {rel:.2e}",
            f32::from_bits(e),
            f32::from_bits(f)
        );
    }
    out
}

fn first_diff(rows: &[PassRow], pick: impl Fn(&PassRow) -> &Err1) -> Option<usize> {
    rows.iter().position(|r| pick(r).diff > 0)
}

/// Merged error over `rows[lo..hi]`.
fn window(rows: &[PassRow], lo: usize, hi: usize, pick: impl Fn(&PassRow) -> &Err1) -> Err1 {
    let mut e = Err1::default();
    rows[lo.min(rows.len())..hi.min(rows.len())]
        .iter()
        .for_each(|r| e.merge(pick(r)));
    e
}

/// Mean error energy per pass of a window (the drift measure: it must not grow over the run).
fn mean_err(rows: &[PassRow], lo: usize, hi: usize, pick: impl Fn(&PassRow) -> &Err1) -> f64 {
    let n = hi.min(rows.len()).saturating_sub(lo).max(1);
    window(rows, lo, hi, pick).err / n as f64
}

/// SNR in dB; "undefined" when the exact words carry no energy (silent output), not `inf`.
fn snr_text(e: &Err1) -> String {
    if e.sig == 0.0 {
        "undefined (exact energy 0)".into()
    } else {
        format!("{:.1}", e.snr())
    }
}

fn row_text(label: &str, g: &str, e: &Err1) -> String {
    format!(
        "{label} | {g} | {} | {:.3e} | {:.3e} | {} | {} / {} ({} outside the float domain, {} non-finite in fast)\n",
        e.n,
        e.max_abs,
        e.max_rel,
        snr_text(e),
        e.diff,
        e.n,
        e.other_diff,
        e.nonfinite_fast
    )
}

/// The report text for the measured passes (`warmup` rows come first and are included).
fn report(rows: &[PassRow], warmup: usize) -> String {
    let mut out = String::new();
    let total = rows.len();
    let _ = writeln!(out, "passes {total} (first {warmup} are warm-up)");
    let ctl = rows.iter().filter(|r| r.steps.0 != r.steps.1).count();
    let _ = writeln!(
        out,
        "passes with a different step count (control flow): {ctl}"
    );
    let bad: u64 = rows.iter().map(|r| r.unreadable).sum();
    let warn = if bad > 0 {
        "  <-- the comparison below is unreliable"
    } else {
        ""
    };
    let _ = writeln!(
        out,
        "data-memory reads that failed (read as 0, either core): {bad}{warn}"
    );
    for (name, pick) in [
        ("lanes", (|r: &PassRow| &r.lanes) as fn(&PassRow) -> &Err1),
        ("buffers", |r| &r.bufs),
        ("state", |r| &r.state),
    ] {
        let first = first_diff(rows, pick);
        let _ = writeln!(
            out,
            "{name}: first diverging pass {}; whole-run {}",
            first.map_or("none".into(), |p| p.to_string()),
            row_text("run", name, &window(rows, 0, total, pick)).trim_end()
        );
        let q = total.div_ceil(4).max(1);
        let w: Vec<String> = (0..4)
            .map(|i| {
                let e = window(rows, i * q, (i + 1) * q, pick);
                format!(
                    "{:.1} dB (mean err energy {:.2e})",
                    e.snr(),
                    mean_err(rows, i * q, (i + 1) * q, pick)
                )
            })
            .collect();
        let _ = writeln!(out, "{name}: quartile SNR {}", w.join(" | "));
        let (m0, m2, m3) = (
            mean_err(rows, 0, q, pick),
            mean_err(rows, 2 * q, 3 * q, pick),
            mean_err(rows, 3 * q, total, pick),
        );
        let verdict = if m3 > 2.0 * m2.max(f64::MIN_POSITIVE) {
            "GROWING (last-quartile mean error energy over 2x the third)".to_string()
        } else {
            format!(
                "bounded (last quartile within 2x the third; last/first quartile {:.1}x)",
                m3 / m0.max(f64::MIN_POSITIVE)
            )
        };
        let _ = writeln!(out, "{name}: drift {verdict}");
    }
    let _ = writeln!(
        out,
        "pass | group | words | max abs err | max rel err (exact >= 1e-12, this pass) | SNR dB | differing words"
    );
    // the first pass (after boot, with the first messages), early passes, and the last
    let last = total.saturating_sub(1);
    let mut picks = vec![0, 1, 2, 5, warmup, 100.min(last), 200.min(last), last];
    picks.retain(|&p| p < total);
    picks.dedup();
    for p in picks {
        for (g, e) in [
            ("lanes", rows[p].lanes),
            ("buffers", rows[p].bufs),
            ("state", rows[p].state),
        ] {
            out += &row_text(&format!("#{p} (exact energy {:.2e})", e.sig), g, &e);
        }
    }
    out
}

fn csv(rows: &[PassRow]) -> String {
    let mut s = String::from("pass,steps_exact,steps_fast");
    for g in ["lanes", "bufs", "state"] {
        let _ = write!(
            s,
            ",{g}_sig,{g}_err,{g}_max_abs,{g}_max_rel,{g}_diff,{g}_nonfinite_fast"
        );
    }
    s.push('\n');
    for (i, r) in rows.iter().enumerate() {
        let _ = write!(s, "{i},{},{}", r.steps.0, r.steps.1);
        for e in [r.lanes, r.bufs, r.state] {
            let _ = write!(
                s,
                ",{:e},{:e},{:e},{:e},{},{}",
                e.sig, e.err, e.max_abs, e.max_rel, e.diff, e.nonfinite_fast
            );
        }
        s.push('\n');
    }
    s
}

struct Args {
    image: String,
    preset: String,
    msgs: Option<String>,
    exact: String,
    fast: String,
    passes: u32,
    warmup: u32,
    csv: Option<String>,
    scan: (u32, u32),
}

fn num(s: &str) -> Result<u32, String> {
    let r = match s.strip_prefix("0x") {
        Some(h) => u32::from_str_radix(h, 16),
        None => s.parse(),
    };
    r.map_err(|e| format!("{s:?}: {e}"))
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        image: String::new(),
        preset: String::new(),
        msgs: None,
        exact: String::new(),
        fast: String::new(),
        passes: 300,
        warmup: 16,
        csv: None,
        scan: (0x8_0000, 0x10_0000),
    };
    let mut it = args.iter();
    while let Some(k) = it.next() {
        let mut val = || it.next().cloned().ok_or(format!("{k} needs a value"));
        match k.as_str() {
            "--image" => a.image = val()?,
            "--preset" => a.preset = val()?,
            "--msgs" => a.msgs = Some(val()?),
            "--exact" => a.exact = val()?,
            "--fast" => a.fast = val()?,
            "--csv" => a.csv = Some(val()?),
            "--passes" => a.passes = num(&val()?)?,
            "--warmup" => a.warmup = num(&val()?)?,
            "--scan" => {
                let v = val()?;
                let (lo, hi) = v.split_once(':').ok_or("--scan LO:HI")?;
                a.scan = (num(lo)?, num(hi)?);
            }
            _ => return Err(format!("unknown argument {k:?}")),
        }
    }
    if a.image.is_empty() || a.preset.is_empty() || a.exact.is_empty() || a.fast.is_empty() {
        return Err("usage: fast_vs_exact --image IMG --preset org|pno [--msgs FILE] --exact TAG --fast TAG".into());
    }
    Ok(a)
}

fn job(a: &Args, tag: &str, name: &str) -> Result<PassJob, String> {
    let data = std::fs::read(&a.image).map_err(|e| format!("{}: {e}", a.image))?;
    let preset = preset_named(&a.preset).ok_or_else(|| format!("preset {:?}", a.preset))?;
    let mut core = boot_core(&data, &preset, &PeriphOpts::default())?;
    core.enable_aot_tag(tag)?;
    let msgs = match &a.msgs {
        Some(f) => parse_msg_text(&std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?)?,
        None => Vec::new(),
    };
    Ok(PassJob::from_preset(name, core, &preset, msgs, Engine::Aot))
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a = parse_args(&args)?;
    let (mut ex, mut fa) = (job(&a, &a.exact, "exact")?, job(&a, &a.fast, "fast")?);
    let (mut ne, mut nf) = (0, 0);
    let mut rows = Vec::new();
    for p in 0..a.warmup + a.passes {
        ex.before_pass(p, &mut ne)?;
        fa.before_pass(p, &mut nf)?;
        let (se, _) = ex.run_pass();
        let (sf, _) = fa.run_pass();
        rows.push(measure(&mut ex, &mut fa, a.scan, (se, sf)));
    }
    if let Some(f) = &a.csv {
        std::fs::write(f, csv(&rows)).map_err(|e| format!("{f}: {e}"))?;
    }
    print!("{}", report(&rows, a.warmup as usize));
    print!("{}", top_diffs(&mut ex, &mut fa, a.scan, 12));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(sig: f64, err: f64, diff: u64) -> PassRow {
        let e = Err1 {
            sig,
            err,
            max_abs: err.sqrt(),
            max_rel: 0.0,
            diff,
            other_diff: 0,
            nonfinite_fast: 0,
            n: 10,
        };
        PassRow {
            steps: (5, 5),
            unreadable: 0,
            lanes: e,
            bufs: e,
            state: e,
        }
    }

    #[test]
    fn err1_accumulates_energy_max_error_and_differing_words() {
        let mut e = Err1::default();
        e.add(1.0f32.to_bits(), 1.0f32.to_bits());
        e.add(2.0f32.to_bits(), 1.5f32.to_bits());
        e.add(f32::NAN.to_bits(), 0);
        assert_eq!((e.n, e.diff), (3, 2));
        assert!((e.sig - 5.0).abs() < 1e-12 && (e.err - 0.25).abs() < 1e-12);
        assert!((e.max_abs - 0.5).abs() < 1e-12);
        assert!((e.snr() - 10.0 * (5.0f64 / 0.25).log10()).abs() < 1e-9);
        assert!(Err1::default().snr().is_nan() || Err1::default().snr().is_infinite());
    }

    #[test]
    fn a_nonfinite_fast_word_is_counted_even_when_the_exact_word_is_not_a_float() {
        let mut e = Err1::default();
        // exact is an integer (outside the float domain), fast holds a NaN
        e.add(0x1234_5678, f32::NAN.to_bits());
        e.add(0.0f32.to_bits(), f32::INFINITY.to_bits());
        e.add(1.0f32.to_bits(), 1.0f32.to_bits());
        assert_eq!((e.nonfinite_fast, e.diff), (2, 2));
        assert!(row_text("x", "g", &e).contains("2 non-finite in fast"));
    }

    #[test]
    fn silent_exact_output_reports_an_undefined_snr_and_the_columns_match_the_header() {
        let e = Err1::default();
        assert!(row_text("x", "g", &e).contains("undefined"));
        let mut f = Err1::default();
        f.add(2.0f32.to_bits(), 1.0f32.to_bits());
        // max rel is 0.5, printed in the max rel column
        assert!(row_text("x", "g", &f).contains("5.000e-1"));
    }

    #[test]
    fn unreadable_reads_are_reported() {
        let mut rows: Vec<PassRow> = (0..4).map(|_| row(1.0, 0.0, 0)).collect();
        assert!(report(&rows, 0).contains("failed (read as 0, either core): 0\n"));
        rows[1].unreadable = 7;
        assert!(report(&rows, 0).contains(": 7  <-- the comparison below is unreliable"));
    }

    #[test]
    fn first_divergence_and_drift_are_detected() {
        let flat: Vec<PassRow> = (0..40)
            .map(|i| row(1.0, if i >= 3 { 1e-12 } else { 0.0 }, u64::from(i >= 3)))
            .collect();
        assert_eq!(first_diff(&flat, |r| &r.lanes), Some(3));
        assert!(report(&flat, 0).contains("lanes: drift bounded"));
        let growing: Vec<PassRow> = (0..40)
            .map(|i| row(1.0, 1e-12 * f64::from(i + 1).powi(3), 1))
            .collect();
        assert!(report(&growing, 0).contains("lanes: drift GROWING"));
        let none: Vec<PassRow> = (0..8).map(|_| row(1.0, 0.0, 0)).collect();
        assert_eq!(first_diff(&none, |r| &r.state), None);
        assert!(report(&none, 0).contains("first diverging pass none"));
    }

    #[test]
    fn arguments_parse_and_require_the_essentials() {
        let v = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        let a = parse_args(&v(
            "--image i --preset org --exact A --fast B --passes 5 --scan 0x10:0x20",
        ))
        .unwrap();
        assert_eq!((a.passes, a.warmup, a.scan), (5, 16, (0x10, 0x20)));
        assert!(parse_args(&v("--image i")).is_err());
        assert!(parse_args(&v("--bogus 1")).is_err());
    }

    #[test]
    fn csv_has_a_header_and_one_line_per_pass() {
        let c = csv(&[row(1.0, 0.0, 0), row(1.0, 1e-9, 2)]);
        assert_eq!(c.lines().count(), 3);
        assert!(c.starts_with("pass,steps_exact,steps_fast,lanes_sig"));
    }
}
