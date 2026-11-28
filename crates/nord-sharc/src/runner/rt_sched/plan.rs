//! Affinity plan: which logical processor each emulation thread is pinned to.
//!
//! Pure logic, no OS calls. Every SHARC core thread and (optionally) the coordinator thread spin
//! on the pass barrier, so each must own a logical processor; the first [`RESERVED_LOGICAL`]
//! logical processors stay free for the system and the audio driver (their interrupts and DPCs
//! land on processor 0 by default on Windows: ASSUMED, see `docs/research/windows-rt-scheduling.md`).
//!
//! When the machine cannot give every thread its own non-reserved logical processor the plan
//! does not pin at all and says why ([`NoPin`]): a spinning thread parked on the audio driver's
//! processor costs more than the migrations it would save.
//!
//! Provenance: the reserved count and the SMT ordering are ASSUMED (design choice; confirm with
//! the wake-up latency helper in `latency.rs` on real hardware, see the research note).
use super::SchedError;

/// Logical processors left to the system and the audio driver. ASSUMED: one physical core on an
/// SMT machine (Windows numbers the two siblings of a core next to each other), two cores without.
pub const RESERVED_LOGICAL: usize = 2;

/// A thread affinity mask is one `usize`: bits 0..63 of the current processor group. Machines
/// with more logical processors are planned as if they had 64 (processor groups: follow-up).
pub const MAX_MASK_CPUS: usize = usize::BITS as usize;

/// Why a plan does not pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoPin {
    /// No thread to place.
    NoThreads,
    /// `needed` threads do not fit in the `logical` processors left after `reserved` are kept free.
    TooFewLogical {
        logical: usize,
        needed: usize,
        reserved: usize,
    },
    /// The caller asked for no pinning.
    Disabled,
}

/// How the OS is told about the placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinMode {
    /// `SetThreadAffinityMask` with one bit: the thread can run nowhere else.
    Hard,
    /// `SetThreadIdealProcessor`: a hint the scheduler may override.
    Ideal,
}

/// The placement of every thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AffinityPlan {
    /// No pinning, with the reason.
    Unpinned(NoPin),
    /// One logical processor per thread.
    Pinned {
        /// Logical processor of SHARC core `i`.
        cores: Vec<usize>,
        /// Logical processor of the coordinator thread, when it was planned.
        coordinator: Option<usize>,
        mode: PinMode,
    },
}

/// Plan `cores` core threads (plus one coordinator when `coordinator`) on `logical` processors.
///
/// `smt_pairs`: logical processors `2k` and `2k+1` are siblings of one physical core, so the
/// plan takes the even ones first (one thread per physical core) and the odd ones only when it
/// runs out. Threads are placed from `reserved` upward, core 0 first, the coordinator last.
pub fn plan_affinity(
    logical: usize,
    cores: usize,
    coordinator: bool,
    reserved: usize,
    smt_pairs: bool,
    mode: PinMode,
) -> AffinityPlan {
    let needed = cores + usize::from(coordinator);
    if needed == 0 {
        return AffinityPlan::Unpinned(NoPin::NoThreads);
    }
    let logical = logical.min(MAX_MASK_CPUS);
    if needed > logical.saturating_sub(reserved) {
        return AffinityPlan::Unpinned(NoPin::TooFewLogical {
            logical,
            needed,
            reserved,
        });
    }
    let mut order: Vec<usize> = (reserved..logical).collect();
    if smt_pairs {
        order.sort_by_key(|c| (c % 2, *c));
    }
    AffinityPlan::Pinned {
        cores: order[..cores].to_vec(),
        coordinator: coordinator.then(|| order[cores]),
        mode,
    }
}

impl AffinityPlan {
    /// Logical processor of SHARC core `core`: `Ok(None)` when the plan does not pin, an error
    /// when the plan pins but has no slot for that core.
    pub fn cpu_for_core(&self, core: usize) -> Result<Option<usize>, SchedError> {
        match self {
            Self::Unpinned(_) => Ok(None),
            Self::Pinned { cores, .. } => cores
                .get(core)
                .copied()
                .map(Some)
                .ok_or(SchedError::NoSlot { core }),
        }
    }

    /// Logical processor of the coordinator, `None` when unpinned or not planned.
    pub fn cpu_for_coordinator(&self) -> Option<usize> {
        match self {
            Self::Pinned { coordinator, .. } => *coordinator,
            Self::Unpinned(_) => None,
        }
    }

    /// The pin mode, `None` when unpinned.
    pub fn mode(&self) -> Option<PinMode> {
        match self {
            Self::Pinned { mode, .. } => Some(*mode),
            Self::Unpinned(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_cpus(p: &AffinityPlan) -> Vec<usize> {
        match p {
            AffinityPlan::Pinned {
                cores, coordinator, ..
            } => cores.iter().copied().chain(*coordinator).collect(),
            AffinityPlan::Unpinned(_) => Vec::new(),
        }
    }

    #[test]
    fn grid_never_overlaps_and_keeps_reserved_free() {
        for logical in [4usize, 8, 16, 32] {
            for cores in 1..=9usize {
                for coord in [false, true] {
                    for smt in [false, true] {
                        let p = plan_affinity(
                            logical,
                            cores,
                            coord,
                            RESERVED_LOGICAL,
                            smt,
                            PinMode::Hard,
                        );
                        let needed = cores + usize::from(coord);
                        let fits = needed <= logical - RESERVED_LOGICAL;
                        match &p {
                            AffinityPlan::Pinned { cores: c, .. } => {
                                assert!(fits, "{logical}/{cores}/{coord}");
                                assert_eq!(c.len(), cores);
                                let mut v = all_cpus(&p);
                                assert_eq!(v.len(), needed);
                                v.sort_unstable();
                                v.dedup();
                                assert_eq!(v.len(), needed, "overlap {logical}/{cores}/{smt}");
                                assert!(v.iter().all(|&x| x >= RESERVED_LOGICAL && x < logical));
                            }
                            AffinityPlan::Unpinned(NoPin::TooFewLogical { .. }) => {
                                assert!(!fits, "{logical}/{cores}/{coord}")
                            }
                            other => panic!("unexpected {other:?}"),
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn sequential_order_without_smt() {
        let p = plan_affinity(8, 4, true, 2, false, PinMode::Hard);
        assert_eq!(all_cpus(&p), vec![2, 3, 4, 5, 6]);
        assert_eq!(p.cpu_for_coordinator(), Some(6));
    }

    #[test]
    fn smt_takes_one_thread_per_physical_core_first() {
        // 16 logical = 8 physical; reserved 0,1 = physical core 0
        let p = plan_affinity(16, 8, false, 2, true, PinMode::Hard);
        assert_eq!(all_cpus(&p), vec![2, 4, 6, 8, 10, 12, 14, 3]);
    }

    #[test]
    fn few_cores_fall_back_with_a_reason() {
        let p = plan_affinity(4, 3, false, 2, false, PinMode::Hard);
        assert_eq!(
            p,
            AffinityPlan::Unpinned(NoPin::TooFewLogical {
                logical: 4,
                needed: 3,
                reserved: 2
            })
        );
        assert_eq!(p.cpu_for_core(0), Ok(None));
        assert_eq!(p.mode(), None);
    }

    #[test]
    fn zero_threads_and_mask_cap() {
        assert_eq!(
            plan_affinity(8, 0, false, 2, false, PinMode::Hard),
            AffinityPlan::Unpinned(NoPin::NoThreads)
        );
        let p = plan_affinity(256, 9, false, 2, false, PinMode::Ideal);
        assert!(all_cpus(&p).iter().all(|&c| c < MAX_MASK_CPUS));
        assert_eq!(p.mode(), Some(PinMode::Ideal));
    }

    #[test]
    fn lookup_errors_for_a_core_without_a_slot() {
        let p = plan_affinity(8, 2, false, 2, false, PinMode::Hard);
        assert_eq!(p.cpu_for_core(1), Ok(Some(3)));
        assert_eq!(p.cpu_for_core(2), Err(SchedError::NoSlot { core: 2 }));
        assert_eq!(p.cpu_for_coordinator(), None);
    }
}
