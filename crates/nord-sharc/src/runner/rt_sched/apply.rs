//! [`apply_with`]: the policy applied to the calling thread, idempotent per thread.
use super::*;
use std::cell::RefCell;

#[derive(Clone, PartialEq, Eq)]
struct Key {
    class: PriorityClass,
    priority: ThreadPriority,
    mmcss: Option<&'static str>,
    cpu: Option<usize>,
    mode: Option<PinMode>,
}

#[derive(Default)]
struct ThreadState {
    done: Option<(Key, Applied)>,
    mmcss_joined: bool,
}

thread_local! {
    static STATE: RefCell<ThreadState> = RefCell::new(ThreadState::default());
}

fn step(r: Result<(), SchedError>) -> Step {
    match r {
        Ok(()) => Step::Done,
        Err(SchedError::Unsupported) => Step::Skipped(SkipReason::Unsupported),
        Err(e) => Step::Failed(e),
    }
}

/// Apply `policy` to the calling thread through `backend`.
///
/// `Err` only for a malformed policy or a role the plan has no slot for. A failing system call is
/// reported in the step, and the remaining steps still run (scheduling is best effort).
pub fn apply_with<B: SchedBackend + ?Sized>(
    backend: &B,
    policy: &RtPolicy,
    role: ThreadRole,
) -> Result<Applied, SchedError> {
    if policy.mmcss_task == Some("") {
        return Err(SchedError::InvalidPolicy("empty MMCSS task name"));
    }
    let cpu = match role {
        ThreadRole::Core(i) => policy.affinity.cpu_for_core(i)?,
        ThreadRole::Coordinator => policy.affinity.cpu_for_coordinator(),
    };
    let key = Key {
        class: policy.class,
        priority: policy.priority,
        mmcss: policy.mmcss_task,
        cpu,
        mode: policy.affinity.mode(),
    };
    if let Some(prev) = STATE.with(|s| {
        s.borrow()
            .done
            .as_ref()
            .filter(|(k, _)| *k == key)
            .map(|(_, a)| *a)
    }) {
        return Ok(Applied {
            already: true,
            ..prev
        });
    }
    let class = if policy.class == PriorityClass::Normal {
        Step::Skipped(SkipReason::NotRequested)
    } else {
        step(backend.set_priority_class(policy.class))
    };
    let priority = if policy.priority == ThreadPriority::Normal {
        Step::Skipped(SkipReason::NotRequested)
    } else {
        step(backend.set_thread_priority(policy.priority))
    };
    let joined = STATE.with(|s| s.borrow().mmcss_joined);
    let (mmcss, now_joined) = match policy.mmcss_task {
        None => (Step::Skipped(SkipReason::NotRequested), false),
        Some(_) if joined => (Step::Skipped(SkipReason::AlreadyJoined), true),
        Some(task) => {
            let s = step(backend.join_mmcss(task));
            (s, s == Step::Done)
        }
    };
    let affinity = match (cpu, &policy.affinity) {
        (Some(c), plan) => match plan.mode() {
            Some(mode) => step(backend.pin_current_thread(c, mode)),
            None => Step::Skipped(SkipReason::NotRequested),
        },
        (None, AffinityPlan::Unpinned(why)) => Step::Skipped(SkipReason::NoPin(*why)),
        (None, AffinityPlan::Pinned { .. }) => Step::Skipped(SkipReason::NotRequested),
    };
    let applied = Applied {
        class,
        priority,
        mmcss,
        affinity,
        cpu: (affinity == Step::Done).then_some(cpu).flatten(),
        already: false,
    };
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.mmcss_joined |= now_joined;
        // a failed attempt is not remembered, so the next call retries it
        s.done = applied.ok().then_some((key, applied));
    });
    Ok(applied)
}

/// [`apply_with`] on this platform's backend.
pub fn apply_to_current_thread(policy: &RtPolicy, role: ThreadRole) -> Result<Applied, SchedError> {
    apply_with(&default_backend(), policy, role)
}

/// Call site helper: does nothing (`Ok(None)`) unless `NS4_RT_SCHED=1`.
pub fn apply_if_enabled(
    policy: &RtPolicy,
    role: ThreadRole,
) -> Result<Option<Applied>, SchedError> {
    if rt_sched_enabled() {
        apply_to_current_thread(policy, role).map(Some)
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{Call, FakeBackend};
    use super::*;

    fn pinned(cores: usize) -> RtPolicy {
        RtPolicy::audio(plan_affinity(
            8,
            cores,
            true,
            RESERVED_LOGICAL,
            false,
            PinMode::Hard,
        ))
    }

    #[test]
    fn full_policy_calls_everything_once_in_order() {
        let b = FakeBackend::default();
        let a = apply_with(&b, &pinned(4), ThreadRole::Core(1)).unwrap();
        assert!(a.ok() && !a.already);
        assert_eq!(a.cpu, Some(3));
        assert_eq!(
            b.calls(),
            vec![
                Call::Class(PriorityClass::High),
                Call::Priority(ThreadPriority::TimeCritical),
                Call::Mmcss("Pro Audio".into()),
                Call::Pin(3, PinMode::Hard),
            ]
        );
    }

    #[test]
    fn coordinator_gets_its_own_slot() {
        let b = FakeBackend::default();
        let a = apply_with(&b, &pinned(4), ThreadRole::Coordinator).unwrap();
        assert_eq!(a.cpu, Some(6));
    }

    #[test]
    fn applying_twice_is_idempotent() {
        let b = FakeBackend::default();
        let p = pinned(2);
        let first = apply_with(&b, &p, ThreadRole::Core(0)).unwrap();
        let n = b.calls().len();
        let second = apply_with(&b, &p, ThreadRole::Core(0)).unwrap();
        assert_eq!(b.calls().len(), n, "second apply must not call the OS");
        assert!(second.already);
        assert_eq!(
            Applied {
                already: false,
                ..second
            },
            first
        );
    }

    #[test]
    fn a_new_policy_reapplies_but_joins_mmcss_once() {
        let b = FakeBackend::default();
        apply_with(&b, &pinned(2), ThreadRole::Core(0)).unwrap();
        let a = apply_with(&b, &pinned(2), ThreadRole::Core(1)).unwrap();
        assert!(!a.already);
        assert_eq!(a.mmcss, Step::Skipped(SkipReason::AlreadyJoined));
        let joins = b
            .calls()
            .iter()
            .filter(|c| matches!(c, Call::Mmcss(_)))
            .count();
        assert_eq!(joins, 1);
    }

    #[test]
    fn noop_backend_skips_every_step() {
        let a = apply_with(&NoopBackend, &pinned(2), ThreadRole::Core(0)).unwrap();
        assert!(a.ok());
        for s in [a.class, a.priority, a.mmcss, a.affinity] {
            assert_eq!(s, Step::Skipped(SkipReason::Unsupported));
        }
        assert_eq!(a.cpu, None);
    }

    #[test]
    fn off_policy_makes_no_call() {
        let b = FakeBackend::default();
        let a = apply_with(&b, &RtPolicy::off(), ThreadRole::Core(0)).unwrap();
        assert!(b.calls().is_empty());
        assert_eq!(
            a.affinity,
            Step::Skipped(SkipReason::NoPin(NoPin::Disabled))
        );
    }

    #[test]
    fn too_few_logical_processors_reports_the_reason_and_still_sets_priority() {
        let b = FakeBackend::default();
        let p = RtPolicy::audio(plan_affinity(4, 8, false, 2, false, PinMode::Hard));
        let a = apply_with(&b, &p, ThreadRole::Core(5)).unwrap();
        assert!(matches!(
            a.affinity,
            Step::Skipped(SkipReason::NoPin(NoPin::TooFewLogical { .. }))
        ));
        assert_eq!(a.priority, Step::Done);
        assert!(!b.calls().iter().any(|c| matches!(c, Call::Pin(..))));
    }

    #[test]
    fn error_paths() {
        let b = FakeBackend::default();
        // role without a slot in a pinned plan
        assert_eq!(
            apply_with(&b, &pinned(2), ThreadRole::Core(7)),
            Err(SchedError::NoSlot { core: 7 })
        );
        // malformed policy
        let mut p = pinned(2);
        p.mmcss_task = Some("");
        assert!(matches!(
            apply_with(&b, &p, ThreadRole::Core(0)),
            Err(SchedError::InvalidPolicy(_))
        ));
        assert!(b.calls().is_empty());
        // OS failure: reported, other steps still run, not remembered (next call retries)
        let b = FakeBackend::failing("SetThreadPriority", 5);
        let a = apply_with(&b, &pinned(2), ThreadRole::Core(0)).unwrap();
        assert_eq!(
            a.priority,
            Step::Failed(SchedError::Os {
                call: "SetThreadPriority",
                code: 5
            })
        );
        assert!(!a.ok());
        assert_eq!(
            (a.class, a.mmcss, a.affinity),
            (Step::Done, Step::Done, Step::Done)
        );
        let n = b.calls().len();
        let again = apply_with(&b, &pinned(2), ThreadRole::Core(0)).unwrap();
        assert!(!again.already);
        assert!(b.calls().len() > n);
        assert!(SchedError::Os { call: "x", code: 1 }
            .to_string()
            .contains("error 1"));
    }

    /// Scheduling changes timing, never output: a deterministic multi-thread computation gives
    /// the same result whatever policy each thread applied (structural: this module has no way
    /// to reach emulator state; the test pins the contract for the integration).
    #[test]
    fn policy_never_changes_results() {
        fn run(policy: Option<RtPolicy>) -> Vec<u64> {
            let handles: Vec<_> = (0..4usize)
                .map(|i| {
                    let policy = policy.clone();
                    std::thread::spawn(move || {
                        if let Some(p) = &policy {
                            apply_with(&FakeBackend::default(), p, ThreadRole::Core(i)).unwrap();
                        }
                        let mut x = 0x9E37_79B9_7F4A_7C15u64 ^ i as u64;
                        for _ in 0..10_000 {
                            x ^= x << 13;
                            x ^= x >> 7;
                            x ^= x << 17;
                        }
                        x
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        }
        let off = run(None);
        assert_eq!(off, run(Some(RtPolicy::off())));
        assert_eq!(off, run(Some(pinned(4))));
    }
}
