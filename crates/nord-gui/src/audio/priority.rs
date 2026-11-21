//! Scheduling class of the audio callback thread.
//!
//! cpal 0.15.3 (`host/wasapi/stream.rs`, `boost_current_thread_priority`) tries to raise its
//! WASAPI thread to `THREAD_PRIORITY_TIME_CRITICAL` but passes the thread *id* as a `HANDLE`
//! (`HANDLE(GetCurrentThreadId() as isize)`), so `SetThreadPriority` fails and the result is
//! discarded. The callback thread then runs at normal priority and no MMCSS class, and any other
//! process can preempt it for a whole scheduler quantum in the middle of a block. [`boost_current_thread`]
//! does it correctly: `SetThreadPriority(GetCurrentThread(), TIME_CRITICAL)` and the MMCSS task
//! `"Pro Audio"` (`AvSetMmThreadCharacteristicsW`), and reports what the OS now says.
//!
//! The first callback calls it once (a one-time system call, never per block). Windows only; on
//! other hosts the report is empty and nothing changes.

/// What the OS reports for the thread after (and before) [`boost_current_thread`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThreadBoost {
    /// `GetThreadPriority` before the call (cpal's own attempt included); `None` off Windows.
    pub before: Option<i32>,
    /// `GetThreadPriority` after the call.
    pub after: Option<i32>,
    /// The MMCSS task was joined.
    pub mmcss: bool,
}

impl ThreadBoost {
    /// Packs the report into one atomic word (`before` and `after` offset by 16, bit 31 = MMCSS).
    pub fn pack(self) -> u32 {
        let field = |p: Option<i32>| p.map_or(0u32, |v| (v + 16).clamp(1, 31) as u32);
        field(self.before) | (field(self.after) << 5) | (u32::from(self.mmcss) << 31)
    }

    /// Inverse of [`Self::pack`].
    pub fn unpack(w: u32) -> Self {
        let field = |v: u32| (v != 0).then(|| v as i32 - 16);
        Self {
            before: field(w & 31),
            after: field((w >> 5) & 31),
            mmcss: w >> 31 == 1,
        }
    }

    /// `true` once the thread runs at or above `THREAD_PRIORITY_HIGHEST` (2).
    pub fn is_raised(&self) -> bool {
        self.after.is_some_and(|p| p >= 2)
    }

    /// Text for the status line and the exit log.
    pub fn summary(&self) -> String {
        match self.after {
            None => "thread priority: n/a".into(),
            Some(a) => format!(
                "thread priority {} (was {}), MMCSS {}",
                a,
                self.before.map_or("?".into(), |b| b.to_string()),
                if self.mmcss { "Pro Audio" } else { "no" }
            ),
        }
    }
}

/// Raises the calling thread (see the module docs). Call from the audio callback's first block.
#[cfg(windows)]
pub fn boost_current_thread() -> ThreadBoost {
    use windows_sys::Win32::System::Threading::{
        AvSetMmThreadCharacteristicsW, GetCurrentThread, GetThreadPriority, SetThreadPriority,
        THREAD_PRIORITY_TIME_CRITICAL,
    };
    // ASSUMED: "Pro Audio" is the stock MMCSS task of the audio class (HKLM\SOFTWARE\Microsoft\n    // Windows NT\CurrentVersion\Multimedia\SystemProfile\Tasks); it is the name audio APIs commonly
    // use. Confirmed per run by the `mmcss` flag in the meter summary, not by a firmware source.
    // SAFETY: `GetCurrentThread` returns a pseudo handle valid for the calling thread; the other
    // calls take that handle and plain integers, and the MMCSS task name is a static NUL terminated
    // UTF-16 literal (`w!`, no allocation).
    unsafe {
        let thread = GetCurrentThread();
        let before = GetThreadPriority(thread);
        let mut task = 0u32;
        let mmcss =
            !AvSetMmThreadCharacteristicsW(windows_sys::w!("Pro Audio"), &mut task).is_null();
        SetThreadPriority(thread, THREAD_PRIORITY_TIME_CRITICAL);
        ThreadBoost {
            before: Some(before),
            after: Some(GetThreadPriority(thread)),
            mmcss,
        }
    }
}

/// Off Windows nothing is changed.
#[cfg(not(windows))]
pub fn boost_current_thread() -> ThreadBoost {
    ThreadBoost::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_survives_packing() {
        for b in [
            ThreadBoost::default(),
            ThreadBoost {
                before: Some(0),
                after: Some(15),
                mmcss: true,
            },
            ThreadBoost {
                before: Some(-2),
                after: Some(2),
                mmcss: false,
            },
        ] {
            assert_eq!(ThreadBoost::unpack(b.pack()), b);
        }
        assert!(ThreadBoost::unpack(0).summary().contains("n/a"));
    }

    #[cfg(windows)]
    #[test]
    fn a_normal_thread_is_raised_to_time_critical_with_mmcss() {
        // A fresh thread starts at normal priority (0) like cpal's callback thread does.
        let b = std::thread::spawn(boost_current_thread).join().unwrap();
        assert_eq!(b.before, Some(0), "{b:?}");
        assert_eq!(b.after, Some(15), "TIME_CRITICAL, {b:?}");
        assert!(b.is_raised());
        assert!(b.summary().contains("priority 15"), "{}", b.summary());
    }

    #[cfg(not(windows))]
    #[test]
    fn off_windows_nothing_changes() {
        assert_eq!(boost_current_thread(), ThreadBoost::default());
    }
}
