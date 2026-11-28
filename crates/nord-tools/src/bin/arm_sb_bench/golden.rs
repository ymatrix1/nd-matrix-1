//! The golden digest file of `--golden`.
//!
//! The file is line oriented and only lines that start with `digest ` count, so the whole output
//! of a reference run (`arm-sb-bench ... > golden.txt`, made with tier 1 on the current main:
//! design note section 8, layer L2) is a valid golden file. Everything else, the counter tables
//! included, is ignored: counters change when idle v2 or a tier is switched on, the digests must
//! not. The file holds state hashes and message digests only, no firmware bytes (D20, D128).

const PREFIX: &str = "digest ";

/// The digest lines of `text` (the golden file or the output of a run), trailing whitespace cut.
pub fn digest_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.starts_with(PREFIX))
        .map(|l| l.trim_end().to_string())
        .collect()
}

/// Compare the digest lines of the golden file with those of this run. `Err` holds one readable
/// line per difference (a differing, missing or extra line, or an empty golden file).
pub fn compare(golden: &str, got: &[String]) -> Result<(), Vec<String>> {
    let want = digest_lines(golden);
    if want.is_empty() {
        return Err(vec![format!(
            "the golden file has no `{}` lines, nothing to compare",
            PREFIX.trim_end()
        )]);
    }
    let mut diffs = Vec::new();
    for i in 0..want.len().max(got.len()) {
        match (want.get(i), got.get(i)) {
            (Some(w), Some(g)) if w == g => {}
            (Some(w), Some(g)) => diffs.push(format!("line {}: golden `{w}`, got `{g}`", i + 1)),
            (Some(w), None) => {
                diffs.push(format!("line {}: golden `{w}`, missing in this run", i + 1))
            }
            (None, Some(g)) => diffs.push(format!("line {}: extra in this run `{g}`", i + 1)),
            (None, None) => {}
        }
    }
    if diffs.is_empty() {
        Ok(())
    } else {
        Err(diffs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn only_digest_lines_count_and_a_whole_output_is_a_golden_file() {
        let text =
            "cpu: x\nphase ticks\ndigest boot state 0123\nload 800 1 2\ndigest load state 4567  \n";
        assert_eq!(
            digest_lines(text),
            lines(&["digest boot state 0123", "digest load state 4567"])
        );
        let got = lines(&["digest boot state 0123", "digest load state 4567"]);
        assert_eq!(compare(text, &got), Ok(()));
        // a digest-looking word in the middle of a line is not a digest line
        assert!(digest_lines("see digest x 1\n").is_empty());
    }

    #[test]
    fn a_changed_missing_or_extra_line_is_reported_with_its_position() {
        let golden = "digest a 1\ndigest b 2\ndigest c 3\n";
        let changed = compare(golden, &lines(&["digest a 1", "digest b 9", "digest c 3"]));
        let d = changed.unwrap_err();
        assert_eq!(d.len(), 1);
        assert!(
            d[0].starts_with("line 2:")
                && d[0].contains("digest b 2")
                && d[0].contains("digest b 9")
        );
        let short = compare(golden, &lines(&["digest a 1"])).unwrap_err();
        assert_eq!(short.len(), 2);
        assert!(short[0].contains("missing in this run"));
        let long = compare(
            golden,
            &lines(&["digest a 1", "digest b 2", "digest c 3", "digest d 4"]),
        );
        assert!(long.unwrap_err()[0].contains("extra in this run"));
    }

    #[test]
    fn a_golden_file_without_digest_lines_never_passes() {
        let err = compare("nothing here\n", &lines(&["digest a 1"])).unwrap_err();
        assert!(err[0].contains("no `digest` lines"), "{err:?}");
        assert!(compare("", &[]).is_err());
    }
}
