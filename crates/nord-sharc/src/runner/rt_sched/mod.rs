//! Real-time scheduling policy for the emulation threads (Phase 8 LLE): process priority class,
//! per-thread priority, MMCSS registration and core pinning, to cut the scheduling jitter of the
//! spin-barrier pass loop (`pass_sched`, budget 46.08 us per pass).
//!
//! **Scheduling changes timing, never output.** Nothing here can touch emulator state: the module
//! has no access to a core, a memory or a message, and the pass loop synchronizes on atomics
//! whatever the OS does with the threads. Results must stay bit-identical with the policy on or
//! off (`tests::policy_never_changes_results`).
//!
//! Off by default: [`rt_sched_enabled`] reads `NS4_RT_SCHED=1`. [`apply_to_current_thread`] is
//! called once at the start of each emulation thread, never from the pass loop (it allocates and
//! makes system calls). Calling it again on a thread with the same policy and role does nothing
//! (idempotent). The class is never `REALTIME_PRIORITY_CLASS` (the type has no such variant): see
//! `docs/research/windows-rt-scheduling.md`.
//!
//! Backends: [`SchedBackend`]. `windows.rs` is the Win32 one (cfg(windows)), [`NoopBackend`] is
//! the portable one, `fake.rs` records calls for the unit tests.
//!
//! Provenance: ASSUMED throughout; no number here was measured on Windows (nothing was run there).
mod apply;
#[cfg(test)]
mod fake;
pub mod latency;
pub mod plan;
#[cfg(any(windows, test))]
mod windows;

use std::fmt;

pub use apply::{apply_if_enabled, apply_to_current_thread, apply_with};
pub use plan::{plan_affinity, AffinityPlan, NoPin, PinMode, RESERVED_LOGICAL};

/// Environment variable that turns the policy on (`1`).
pub const ENV: &str = "NS4_RT_SCHED";

/// MMCSS task for the emulation threads (a stock Windows task; ASSUMED right for a spinning
/// audio-rate worker, confirm with the latency helper).
pub const MMCSS_TASK: &str = "Pro Audio";

/// Process priority class. There is no real-time class on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityClass {
    Normal,
    /// `HIGH_PRIORITY_CLASS`.
    High,
}

/// Thread priority within the class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadPriority {
    Normal,
    /// `THREAD_PRIORITY_HIGHEST`.
    Highest,
    /// `THREAD_PRIORITY_TIME_CRITICAL`.
    TimeCritical,
}

/// What a thread does in the pass loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadRole {
    /// The thread that publishes the tick and waits for all cores.
    Coordinator,
    /// The thread that runs SHARC core `i`.
    Core(usize),
}

/// The whole policy for one thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtPolicy {
    pub class: PriorityClass,
    pub priority: ThreadPriority,
    /// MMCSS task name, `None` = do not register.
    pub mmcss_task: Option<&'static str>,
    pub affinity: AffinityPlan,
}

impl RtPolicy {
    /// Change nothing.
    pub fn off() -> Self {
        Self {
            class: PriorityClass::Normal,
            priority: ThreadPriority::Normal,
            mmcss_task: None,
            affinity: AffinityPlan::Unpinned(NoPin::Disabled),
        }
    }

    /// High class, time-critical threads, "Pro Audio" MMCSS, with the given placement.
    pub fn audio(affinity: AffinityPlan) -> Self {
        Self {
            class: PriorityClass::High,
            priority: ThreadPriority::TimeCritical,
            mmcss_task: Some(MMCSS_TASK),
            affinity,
        }
    }
}

/// Why a system call failed or was not made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedError {
    /// The backend cannot do this on this platform.
    Unsupported,
    /// The plan pins but has no slot for core `core`.
    NoSlot { core: usize },
    /// The policy itself is malformed.
    InvalidPolicy(&'static str),
    /// An OS call failed (`GetLastError` code).
    Os { call: &'static str, code: u32 },
}

impl fmt::Display for SchedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => write!(f, "not supported on this platform"),
            Self::NoSlot { core } => write!(f, "affinity plan has no slot for core {core}"),
            Self::InvalidPolicy(why) => write!(f, "invalid policy: {why}"),
            Self::Os { call, code } => write!(f, "{call} failed (error {code})"),
        }
    }
}

impl std::error::Error for SchedError {}

/// What happened to one part of the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Done,
    /// Not requested, not supported, or already in place; no call was made.
    Skipped(SkipReason),
    /// The call was made and failed; the other steps still ran.
    Failed(SchedError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    NotRequested,
    Unsupported,
    /// The thread is already registered with MMCSS.
    AlreadyJoined,
    /// The plan does not pin, with its reason.
    NoPin(NoPin),
}

/// Result of [`apply_to_current_thread`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    pub class: Step,
    pub priority: Step,
    pub mmcss: Step,
    pub affinity: Step,
    /// The logical processor the thread was pinned to.
    pub cpu: Option<usize>,
    /// This thread already had exactly this policy: nothing was called.
    pub already: bool,
}

impl Applied {
    /// No step failed (skips are fine).
    pub fn ok(&self) -> bool {
        ![self.class, self.priority, self.mmcss, self.affinity]
            .iter()
            .any(|s| matches!(s, Step::Failed(_)))
    }
}

/// The OS calls. `Err(Unsupported)` means "skip", any other error is reported as a failed step.
pub trait SchedBackend {
    /// Logical processors visible to the process.
    fn logical_processors(&self) -> usize;
    /// Process-wide.
    fn set_priority_class(&self, class: PriorityClass) -> Result<(), SchedError>;
    fn set_thread_priority(&self, priority: ThreadPriority) -> Result<(), SchedError>;
    fn join_mmcss(&self, task: &str) -> Result<(), SchedError>;
    fn pin_current_thread(&self, cpu: usize, mode: PinMode) -> Result<(), SchedError>;
}

/// Portable backend: every call is "unsupported", so every step is skipped.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopBackend;

impl SchedBackend for NoopBackend {
    fn logical_processors(&self) -> usize {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    }
    fn set_priority_class(&self, _: PriorityClass) -> Result<(), SchedError> {
        Err(SchedError::Unsupported)
    }
    fn set_thread_priority(&self, _: ThreadPriority) -> Result<(), SchedError> {
        Err(SchedError::Unsupported)
    }
    fn join_mmcss(&self, _: &str) -> Result<(), SchedError> {
        Err(SchedError::Unsupported)
    }
    fn pin_current_thread(&self, _: usize, _: PinMode) -> Result<(), SchedError> {
        Err(SchedError::Unsupported)
    }
}

/// The backend of this platform: Win32 on Windows, [`NoopBackend`] elsewhere.
#[cfg(windows)]
pub fn default_backend() -> impl SchedBackend {
    windows::WindowsBackend
}
/// The backend of this platform: Win32 on Windows, [`NoopBackend`] elsewhere.
#[cfg(not(windows))]
pub fn default_backend() -> impl SchedBackend {
    NoopBackend
}

/// `NS4_RT_SCHED` value to switch: only `1` turns it on.
pub fn enabled_from(value: Option<&str>) -> bool {
    value.map(str::trim) == Some("1")
}

/// Is the policy switched on in the environment?
pub fn rt_sched_enabled() -> bool {
    enabled_from(std::env::var(ENV).ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_switch_accepts_only_one() {
        assert!(enabled_from(Some("1")));
        assert!(enabled_from(Some(" 1 ")));
        for v in [None, Some(""), Some("0"), Some("true"), Some("2")] {
            assert!(!enabled_from(v), "{v:?}");
        }
    }

    #[test]
    fn default_backend_reports_processors() {
        assert!(default_backend().logical_processors() >= 1);
        assert!(NoopBackend.logical_processors() >= 1);
    }

    #[test]
    fn policies_and_applied_summary() {
        let off = RtPolicy::off();
        assert_eq!(
            (off.class, off.priority, off.mmcss_task),
            (PriorityClass::Normal, ThreadPriority::Normal, None)
        );
        let on = RtPolicy::audio(AffinityPlan::Unpinned(NoPin::Disabled));
        assert_eq!(on.mmcss_task, Some("Pro Audio"));
        let bad = Applied {
            class: Step::Done,
            priority: Step::Failed(SchedError::Unsupported),
            mmcss: Step::Done,
            affinity: Step::Done,
            cpu: None,
            already: false,
        };
        assert!(!bad.ok());
        assert_eq!(
            SchedError::NoSlot { core: 3 }.to_string(),
            "affinity plan has no slot for core 3"
        );
    }
}
