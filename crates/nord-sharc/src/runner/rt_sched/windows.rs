//! Win32 backend (cfg(windows)) plus the pure helpers it uses, which compile and are tested on
//! every platform.
//!
//! Raw FFI like `cpu_clock.rs` (no new dependency: `windows-sys` is only a transitive crate of
//! other dependencies here, and its MIT/Apache-2.0 terms are in `about.toml`'s accepted list, so
//! switching later is possible). `avrt.lib` ships with the Windows SDK.
//!
//! Provenance: ASSUMED. The constants are the documented Win32 values (`winbase.h`, `WinBase.h`
//! `HIGH_PRIORITY_CLASS`, `THREAD_PRIORITY_*`) typed from memory of the headers; no check here can
//! confirm them (Linux only) and the call semantics are taken from each function's documentation.
//! Nothing in this file was run on Windows; `cargo check --target x86_64-pc-windows-msvc` only
//! shows that it compiles.
use super::{PriorityClass, SchedError, ThreadPriority};

/// `NORMAL_PRIORITY_CLASS`.
pub const NORMAL_PRIORITY_CLASS: u32 = 0x20;
/// `HIGH_PRIORITY_CLASS`. `REALTIME_PRIORITY_CLASS` (0x100) is deliberately not defined.
pub const HIGH_PRIORITY_CLASS: u32 = 0x80;
pub const THREAD_PRIORITY_NORMAL: i32 = 0;
pub const THREAD_PRIORITY_HIGHEST: i32 = 2;
pub const THREAD_PRIORITY_TIME_CRITICAL: i32 = 15;

pub fn class_value(c: PriorityClass) -> u32 {
    match c {
        PriorityClass::Normal => NORMAL_PRIORITY_CLASS,
        PriorityClass::High => HIGH_PRIORITY_CLASS,
    }
}

pub fn priority_value(p: ThreadPriority) -> i32 {
    match p {
        ThreadPriority::Normal => THREAD_PRIORITY_NORMAL,
        ThreadPriority::Highest => THREAD_PRIORITY_HIGHEST,
        ThreadPriority::TimeCritical => THREAD_PRIORITY_TIME_CRITICAL,
    }
}

/// Single-bit affinity mask for `cpu`; error beyond the 64 bits of one processor group.
pub fn affinity_mask(cpu: usize) -> Result<usize, SchedError> {
    if cpu >= usize::BITS as usize {
        return Err(SchedError::InvalidPolicy("cpu beyond the affinity mask"));
    }
    Ok(1usize << cpu)
}

/// NUL-terminated UTF-16 for the `W` calls.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
pub use imp::WindowsBackend;

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::runner::rt_sched::{PinMode, SchedBackend};

    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn GetCurrentThread() -> isize;
        fn GetLastError() -> u32;
        fn SetPriorityClass(process: isize, class: u32) -> i32;
        fn SetThreadPriority(thread: isize, priority: i32) -> i32;
        /// Returns the previous mask, 0 on failure.
        fn SetThreadAffinityMask(thread: isize, mask: usize) -> usize;
        /// Returns the previous ideal processor, `u32::MAX` on failure.
        fn SetThreadIdealProcessor(thread: isize, ideal: u32) -> u32;
    }

    #[link(name = "avrt")]
    extern "system" {
        /// Returns the MMCSS task handle, 0 on failure.
        fn AvSetMmThreadCharacteristicsW(task: *const u16, index: *mut u32) -> isize;
    }

    fn last(call: &'static str) -> SchedError {
        // SAFETY: GetLastError has no preconditions.
        SchedError::Os {
            call,
            code: unsafe { GetLastError() },
        }
    }

    /// The Win32 backend. Stateless: the MMCSS handle is not kept (the registration ends with the
    /// thread: ASSUMED from the `AvSetMmThreadCharacteristics` documentation).
    #[derive(Debug, Default, Clone, Copy)]
    pub struct WindowsBackend;

    impl SchedBackend for WindowsBackend {
        fn logical_processors(&self) -> usize {
            std::thread::available_parallelism().map_or(1, |n| n.get())
        }

        fn set_priority_class(&self, class: PriorityClass) -> Result<(), SchedError> {
            // SAFETY: the process pseudo-handle is always valid; the class is a documented value.
            let ok = unsafe { SetPriorityClass(GetCurrentProcess(), class_value(class)) };
            if ok != 0 {
                Ok(())
            } else {
                Err(last("SetPriorityClass"))
            }
        }

        fn set_thread_priority(&self, p: ThreadPriority) -> Result<(), SchedError> {
            // SAFETY: the thread pseudo-handle is valid for the calling thread.
            let ok = unsafe { SetThreadPriority(GetCurrentThread(), priority_value(p)) };
            if ok != 0 {
                Ok(())
            } else {
                Err(last("SetThreadPriority"))
            }
        }

        fn join_mmcss(&self, task: &str) -> Result<(), SchedError> {
            let name = wide(task);
            let mut index = 0u32; // 0 = a new task index
                                  // SAFETY: `name` is NUL-terminated and outlives the call; `index` is a valid out ptr.
            let h = unsafe { AvSetMmThreadCharacteristicsW(name.as_ptr(), &mut index) };
            if h != 0 {
                Ok(())
            } else {
                Err(last("AvSetMmThreadCharacteristicsW"))
            }
        }

        fn pin_current_thread(&self, cpu: usize, mode: PinMode) -> Result<(), SchedError> {
            match mode {
                PinMode::Hard => {
                    let mask = affinity_mask(cpu)?;
                    // SAFETY: valid thread pseudo-handle, single-bit mask.
                    let prev = unsafe { SetThreadAffinityMask(GetCurrentThread(), mask) };
                    if prev != 0 {
                        Ok(())
                    } else {
                        Err(last("SetThreadAffinityMask"))
                    }
                }
                PinMode::Ideal => {
                    affinity_mask(cpu)?;
                    // SAFETY: valid thread pseudo-handle.
                    let prev = unsafe { SetThreadIdealProcessor(GetCurrentThread(), cpu as u32) };
                    if prev != u32::MAX {
                        Ok(())
                    } else {
                        Err(last("SetThreadIdealProcessor"))
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_and_no_realtime_class() {
        assert_eq!(class_value(PriorityClass::High), 0x80);
        assert_ne!(class_value(PriorityClass::High), 0x100, "never REALTIME");
        assert_eq!(class_value(PriorityClass::Normal), 0x20);
        assert_eq!(priority_value(ThreadPriority::TimeCritical), 15);
        assert_eq!(priority_value(ThreadPriority::Highest), 2);
        assert_eq!(priority_value(ThreadPriority::Normal), 0);
    }

    #[test]
    fn mask_is_one_bit_and_bounded() {
        assert_eq!(affinity_mask(0), Ok(1));
        assert_eq!(affinity_mask(5), Ok(32));
        assert_eq!(affinity_mask(63), Ok(1usize << 63));
        assert!(affinity_mask(64).is_err());
    }

    #[test]
    fn wide_string_is_nul_terminated_utf16() {
        let w = wide("Pro Audio");
        assert_eq!(w.len(), 10);
        assert_eq!(w[9], 0);
        assert_eq!(w[0], u16::from(b'P'));
    }
}
