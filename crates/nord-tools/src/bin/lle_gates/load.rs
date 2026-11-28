//! Host background load of `lle-gates`: sampled while this process is idle, so the figure is what
//! OTHER processes use. Linux reads `/proc/stat`, Windows `GetSystemTimes`; elsewhere the load is
//! unknown, and an unknown load is not below the limit (as in `arm_sb_ab.rs`).

use std::time::Duration;

/// Highest background load, in percent of all CPU time, at which timings are valid (the repository's
/// A/B protocol, `docs/research/arm0-jit-design.md` section 12).
pub const LOAD_LIMIT_PCT: f64 = 25.0;

/// Whether a sampled load allows a valid timing; `None` (unknown) does not.
pub fn load_ok(load: Option<f64>, limit: f64) -> bool {
    matches!(load, Some(l) if l <= limit)
}

/// Busy percent between two `(idle, total)` counter readings; `None` when time did not advance.
pub fn busy_pct(a: (u64, u64), b: (u64, u64)) -> Option<f64> {
    let total = b.1.checked_sub(a.1).filter(|t| *t > 0)?;
    let idle = b.0.saturating_sub(a.0).min(total);
    Some(100.0 * (total - idle) as f64 / total as f64)
}

/// `(idle, total)` of the aggregate `cpu` line of `/proc/stat` (idle counts `idle + iowait`).
pub fn parse_proc_stat(text: &str) -> Option<(u64, u64)> {
    let line = text.lines().find(|l| l.starts_with("cpu "))?;
    let f: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map(|x| x.parse().ok())
        .collect::<Option<_>>()?;
    if f.len() < 4 {
        return None;
    }
    // user nice system idle iowait irq softirq steal: guest time is already inside user
    let total: u64 = f.iter().take(8).sum();
    Some((f[3] + f.get(4).copied().unwrap_or(0), total))
}

#[cfg(windows)]
fn counters() -> Option<(u64, u64)> {
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct FileTime {
        lo: u32,
        hi: u32,
    }
    extern "system" {
        fn GetSystemTimes(idle: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
    }
    let (mut i, mut k, mut u) = (
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
    );
    // SAFETY: GetSystemTimes writes three FILETIME structs through valid pointers.
    if unsafe { GetSystemTimes(&mut i, &mut k, &mut u) } == 0 {
        return None;
    }
    let v = |t: FileTime| (u64::from(t.hi) << 32) | u64::from(t.lo);
    // kernel time includes idle time
    Some((v(i), v(k) + v(u)))
}

#[cfg(target_os = "linux")]
fn counters() -> Option<(u64, u64)> {
    parse_proc_stat(&std::fs::read_to_string("/proc/stat").ok()?)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn counters() -> Option<(u64, u64)> {
    None
}

/// Background load over `window`, sleeping in it (this process uses no CPU meanwhile); `None`
/// when the platform has no counter.
pub fn sample(window: Duration) -> Option<f64> {
    let a = counters()?;
    std::thread::sleep(window);
    busy_pct(a, counters()?)
}

/// The loads of one measurement: before it, before every run, after it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadLog {
    pub before: Option<f64>,
    pub runs: Vec<Option<f64>>,
    pub after: Option<f64>,
}

impl LoadLog {
    /// The highest sample, `None` when any sample is unknown or there is none.
    pub fn peak(&self) -> Option<f64> {
        let all: Vec<Option<f64>> = std::iter::once(self.before)
            .chain(self.runs.iter().copied())
            .chain(std::iter::once(self.after))
            .collect();
        let mut peak = 0.0f64;
        for l in all {
            peak = peak.max(l?);
        }
        Some(peak)
    }

    /// Runs (by index) whose sampled load is above `limit` or unknown.
    pub fn flagged_runs(&self, limit: f64) -> Vec<usize> {
        (0..self.runs.len())
            .filter(|i| !load_ok(self.runs[*i], limit))
            .collect()
    }

    /// Whether every sample is within `limit`: only then are timings valid.
    pub fn valid(&self, limit: f64) -> bool {
        self.peak().is_some_and(|p| p <= limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 1 2 3 4 5 6 7 8 9 10\n";

    #[test]
    fn the_aggregate_cpu_line_gives_idle_and_total() {
        assert_eq!(parse_proc_stat(STAT), Some((850, 1000)));
        assert_eq!(parse_proc_stat("cpu0 1 2 3 4\n"), None, "no aggregate line");
        assert_eq!(parse_proc_stat("cpu  1 2\n"), None, "too few fields");
        assert_eq!(parse_proc_stat("cpu  a b c d\n"), None, "not numbers");
    }

    #[test]
    fn busy_percent_is_the_non_idle_share_of_the_interval() {
        assert_eq!(busy_pct((850, 1000), (1050, 1400)), Some(50.0));
        assert_eq!(busy_pct((0, 10), (0, 10)), None, "time did not advance");
        assert_eq!(
            busy_pct((0, 10), (20, 20)),
            Some(0.0),
            "idle is clamped to the interval"
        );
    }

    #[test]
    fn an_unknown_load_is_not_ok_and_the_limit_is_inclusive() {
        assert!(load_ok(Some(25.0), LOAD_LIMIT_PCT));
        assert!(!load_ok(Some(25.1), LOAD_LIMIT_PCT));
        assert!(!load_ok(None, LOAD_LIMIT_PCT));
    }

    #[test]
    fn the_log_flags_runs_above_the_limit_and_invalidates_on_any_sample() {
        let log = LoadLog {
            before: Some(3.0),
            runs: vec![Some(5.0), Some(40.0), None],
            after: Some(4.0),
        };
        assert_eq!(log.flagged_runs(25.0), vec![1, 2]);
        assert_eq!(
            log.peak(),
            None,
            "one unknown sample makes the peak unknown"
        );
        assert!(!log.valid(25.0));
        let calm = LoadLog {
            runs: vec![Some(5.0); 3],
            ..log.clone()
        };
        assert_eq!(calm.peak(), Some(5.0));
        assert!(calm.valid(25.0) && !calm.valid(4.0));
        let after_busy = LoadLog {
            after: Some(60.0),
            ..calm
        };
        assert!(!after_busy.valid(25.0), "the sample after the run counts");
    }

    #[test]
    fn a_live_sample_is_a_percentage_or_unknown() {
        if let Some(l) = sample(Duration::from_millis(20)) {
            assert!((0.0..=100.0).contains(&l), "{l}");
        }
    }
}
