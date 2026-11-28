//! Wake-up latency of a spinning thread: how long after a publisher stamps "now" does a thread
//! that spins on an atomic see it. This is the jitter the pass barrier pays on every tick, so it
//! is the number to compare with and without [`super::RtPolicy`].
//!
//! No `Sleep`, no parking: the waiter spins on an atomic and reads [`Instant`] (which is
//! `QueryPerformanceCounter` on Windows). Measurement tool only, never on the audio path; it
//! allocates its sample buffer before the loop starts.
//!
//! Provenance: ASSUMED that one publish/observe round trip on a quiet core is a fair proxy for the
//! barrier's wake-up cost; EMPIRICAL numbers go into `docs/research/windows-rt-scheduling.md`
//! once measured on Windows (not yet: nothing was run there).
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Percentile summary of latency samples, in nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Percentiles {
    pub count: usize,
    pub min: u64,
    pub p50: u64,
    pub p90: u64,
    pub p99: u64,
    pub p999: u64,
    pub max: u64,
}

/// Nearest-rank percentile of an ascending slice: the smallest value with at least `q` of the
/// samples at or below it (`q` in `0.0..=1.0`). Empty slice: 0.
fn rank(sorted: &[u64], q: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((q * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len()) - 1;
    sorted[idx]
}

/// Sort `samples` in place and summarize them.
pub fn percentiles(samples: &mut [u64]) -> Percentiles {
    samples.sort_unstable();
    Percentiles {
        count: samples.len(),
        min: samples.first().copied().unwrap_or(0),
        p50: rank(samples, 0.50),
        p90: rank(samples, 0.90),
        p99: rank(samples, 0.99),
        p999: rank(samples, 0.999),
        max: samples.last().copied().unwrap_or(0),
    }
}

/// Spin until `ns` nanoseconds have passed (a gap between publishes, never a sleep).
fn spin_for(ns: u64) {
    let t = Instant::now();
    while (t.elapsed().as_nanos() as u64) < ns {
        std::hint::spin_loop();
    }
}

/// Measure `iterations` wake-ups: a waiter thread spins on a sequence counter, the calling thread
/// publishes a timestamp every `gap_ns`, the waiter records `now - timestamp`. `setup` runs first
/// on the waiter thread (apply an [`super::RtPolicy`] there to measure its effect).
///
/// Returns the raw samples in publish order (ns).
pub fn measure_wakeup<F>(iterations: usize, gap_ns: u64, setup: F) -> Vec<u64>
where
    F: FnOnce() + Send + 'static,
{
    let base = Instant::now();
    let seq = Arc::new(AtomicU64::new(0));
    let stamp = Arc::new(AtomicU64::new(0));
    let ack = Arc::new(AtomicU64::new(0));
    let waiter = {
        let (seq, stamp, ack) = (seq.clone(), stamp.clone(), ack.clone());
        std::thread::spawn(move || {
            setup();
            let mut out = Vec::with_capacity(iterations);
            ack.store(u64::MAX, Ordering::Release); // ready
            for i in 1..=iterations as u64 {
                while seq.load(Ordering::Acquire) < i {
                    std::hint::spin_loop();
                }
                let now = base.elapsed().as_nanos() as u64;
                out.push(now.saturating_sub(stamp.load(Ordering::Relaxed)));
                ack.store(i, Ordering::Release);
            }
            out
        })
    };
    while ack.load(Ordering::Acquire) != u64::MAX {
        std::hint::spin_loop();
    }
    for i in 1..=iterations as u64 {
        spin_for(gap_ns);
        stamp.store(base.elapsed().as_nanos() as u64, Ordering::Relaxed);
        seq.store(i, Ordering::Release);
        while ack.load(Ordering::Acquire) != i {
            std::hint::spin_loop();
        }
    }
    waiter.join().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_of_synthetic_samples() {
        // 1..=1000 ns, shuffled order must not matter
        let mut s: Vec<u64> = (1..=1000).rev().collect();
        let p = percentiles(&mut s);
        assert_eq!(p.count, 1000);
        assert_eq!((p.min, p.max), (1, 1000));
        assert_eq!(p.p50, 500);
        assert_eq!(p.p90, 900);
        assert_eq!(p.p99, 990);
        assert_eq!(p.p999, 999);
    }

    #[test]
    fn percentiles_edge_cases() {
        assert_eq!(percentiles(&mut []), Percentiles::default());
        let p = percentiles(&mut [7]);
        assert_eq!((p.min, p.p50, p.p999, p.max), (7, 7, 7, 7));
        // an outlier shows in the tail, not in the median
        let mut s = vec![10u64; 99];
        s.push(5_000);
        let p = percentiles(&mut s);
        assert_eq!((p.p50, p.p99, p.max), (10, 10, 5_000));
    }

    #[test]
    fn measure_wakeup_returns_one_ordered_sample_per_iteration() {
        let mut s = measure_wakeup(50, 2_000, || {});
        assert_eq!(s.len(), 50);
        let p = percentiles(&mut s);
        assert!(p.min <= p.p50 && p.p50 <= p.p99 && p.p99 <= p.max);
    }
}
