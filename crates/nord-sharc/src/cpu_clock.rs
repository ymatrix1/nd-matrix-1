//! Thread CPU clock for benchmarks.
//!
//! Wall time on a shared host includes every moment the thread was descheduled by another build or
//! test. The thread CPU time excludes that, which should make an A/B on a loaded host usable
//! with fewer rounds (ASSUMED, not yet measured; residual noise: cache and SMT contention). CPU
//! time is a lower bound of the wall time on a loaded host: use wall time for every deadline and
//! "x of budget" claim.
//!
//! Windows x86-64: `QueryThreadCycleTime` (TSC-rate cycles of this thread), converted to ns with a
//! TSC-over-wall calibration done once (200 ms). Elsewhere there is no per-thread counter without
//! an extra dependency, so [`ThreadClock::is_cpu_time`] is `false` and the clock is wall time.
//! Not for the audio path: it is a measurement tool, never called from `process_block`.
//!
//! Provenance: ASSUMED that the thread cycle counter runs at TSC rate (the calibration measures
//! the ratio, so a constant-rate counter is all that is needed); the self-check in
//! `tests::cpu_time_does_not_count_a_sleep` shows the counter excludes a sleep.
use std::time::Instant;

#[cfg(all(windows, target_arch = "x86_64"))]
mod imp {
    extern "system" {
        fn GetCurrentThread() -> isize;
        fn QueryThreadCycleTime(thread: isize, cycles: *mut u64) -> i32;
    }

    pub const IS_CPU: bool = true;

    pub fn ticks() -> u64 {
        let mut c = 0u64;
        // SAFETY: both calls are plain Win32 queries on the current-thread pseudo-handle; `c` is a
        // valid out pointer for the duration of the call.
        let ok = unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut c) };
        assert!(ok != 0, "QueryThreadCycleTime failed");
        c
    }

    /// ns per tick: wall time over TSC ticks across a 200 ms sleep.
    pub fn calibrate() -> f64 {
        // SAFETY: `rdtsc` has no preconditions.
        let (t0, c0) = (std::time::Instant::now(), unsafe {
            std::arch::x86_64::_rdtsc()
        });
        std::thread::sleep(std::time::Duration::from_millis(200));
        // wrapping: the thread may move to a core whose TSC lags (VMs); the ratio stays usable
        let dc = unsafe { std::arch::x86_64::_rdtsc() }.wrapping_sub(c0);
        t0.elapsed().as_nanos() as f64 / dc.max(1) as f64
    }
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
mod imp {
    pub const IS_CPU: bool = false;

    /// Monotonic nanoseconds since the first call (the fallback clock already counts ns).
    pub fn ticks() -> u64 {
        use std::sync::OnceLock;
        use std::time::Instant;
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed().as_nanos() as u64
    }

    pub fn calibrate() -> f64 {
        1.0
    }
}

/// ns per clock tick (calibrated once per process).
pub fn ns_per_tick() -> f64 {
    static CAL: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *CAL.get_or_init(imp::calibrate)
}

/// Raw clock ticks of the calling thread (see [`ns_per_tick`]).
pub fn ticks() -> u64 {
    imp::ticks()
}

/// A stopwatch on the calling thread's CPU time (wall time where there is no such counter).
/// Start and read it on the same thread.
#[derive(Clone, Copy)]
pub struct ThreadClock {
    t0: u64,
    wall0: Instant,
}

impl ThreadClock {
    /// `true` when elapsed time is thread CPU time, `false` when it is wall time.
    pub const fn is_cpu_time() -> bool {
        imp::IS_CPU
    }

    /// Starts the stopwatch. The first call in a process takes ~200 ms (calibration).
    pub fn start() -> Self {
        let _ = ns_per_tick();
        Self {
            t0: imp::ticks(),
            wall0: Instant::now(),
        }
    }

    /// Elapsed nanoseconds on this thread.
    pub fn elapsed_ns(&self) -> u64 {
        ((imp::ticks().wrapping_sub(self.t0)) as f64 * ns_per_tick()) as u64
    }

    /// Elapsed milliseconds on this thread.
    pub fn elapsed_ms(&self) -> f64 {
        self.elapsed_ns() as f64 / 1e6
    }

    /// Elapsed wall nanoseconds since [`ThreadClock::start`], for printing next to the CPU time.
    pub fn wall_ns(&self) -> u64 {
        self.wall0.elapsed().as_nanos() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_is_positive_and_finite() {
        let n = ns_per_tick();
        assert!(n.is_finite() && n > 0.0, "ns per tick {n}");
    }

    #[test]
    fn busy_work_is_counted_and_close_to_wall() {
        let c = ThreadClock::start();
        let mut x = 1u64;
        while c.wall_ns() < 30_000_000 {
            x = std::hint::black_box(x.wrapping_mul(6364136223846793005).wrapping_add(1));
        }
        let (cpu, wall) = (c.elapsed_ns() as f64, c.wall_ns() as f64);
        assert!(cpu > 0.0);
        // the thread may be descheduled, so CPU time is at most about the wall time
        assert!(cpu <= wall * 1.2, "cpu {cpu} wall {wall}");
        // and a busy loop gets some CPU unless the host is saturated
        assert!(cpu >= wall * 0.05, "cpu {cpu} wall {wall}");
    }

    #[test]
    fn cpu_time_does_not_count_a_sleep() {
        if !ThreadClock::is_cpu_time() {
            return; // wall-time fallback counts sleeps by definition
        }
        let c = ThreadClock::start();
        std::thread::sleep(std::time::Duration::from_millis(60));
        let (cpu, wall) = (c.elapsed_ns(), c.wall_ns());
        assert!(wall >= 55_000_000, "wall {wall}");
        assert!(
            cpu < wall / 4,
            "a sleeping thread used cpu {cpu} of wall {wall}"
        );
    }

    #[test]
    fn elapsed_ms_matches_ns() {
        let c = ThreadClock::start();
        let ns = c.elapsed_ns();
        let ms = c.elapsed_ms();
        assert!(ms * 1e6 >= ns as f64);
    }
}
