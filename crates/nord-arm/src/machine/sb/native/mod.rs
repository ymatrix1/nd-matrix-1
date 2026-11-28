//! OWNER: W1-NCORE. Tier 3: native code from the superblock IR through Cranelift (feature
//! `sb-native`, off by default).
//!
//! Spec: design note section 9. One `Sb` becomes one `extern "C" fn(*mut Cpu, *mut NativeCtl) ->
//! u32`; registers and flags are Cranelift variables, memory goes through the page table with
//! side exits to the interpreter, exits store the dirty state and leave through the driver.
//! [`available`] is the runtime decision: when it is false (an unproven target, no executable
//! memory, a hardened runtime, an unsupported host) there is no backend and everything runs on
//! tier 2, exactly. It is true only on a target listed in [`NATIVE_TARGET_PROVEN`] where the
//! host probe ([`host_runs_native`]) also passes.
//!
//! | File | Owner |
//! |---|---|
//! | `abi` `emit` `codegen` `rt` `spike` (and the test files `rt_tests`, `emit_tests`) | W1-NCORE |
//! | `lower_dp` | W2-NDP |
//! | `lower_mem` `lower_flow` | W2-NMEM |
//! | `backend` `link` `difftest` | W3-NINT |
//!
//! Plumbing for the later packages: [`rt::Jit`] compiles (`compile(sb)`, `define_native(func)`)
//! and runs (`run(&native, cpu, mem, limit)`, the only safe entry), [`emit::NCtx`] is the
//! emission vocabulary (registers, flags, exits, the trace and the lowerer's position in it,
//! predicated blocks, the loop back-edge), `emit::testing` the helpers of native tests,
//! [`tail_calls_available`] gates chaining v2.

pub mod abi;
pub mod backend;
pub mod codegen;
#[cfg(test)]
mod difftest;
pub mod emit;
#[cfg(test)]
mod emit_tests;
pub mod link;
pub mod lower_dp;
pub mod lower_flow;
pub mod lower_mem;
pub mod rt;
#[cfg(test)]
mod rt_tests;
pub mod spike;

pub use backend::NativeBackend;
pub use emit::{NCtx, Unsupported};

use std::sync::OnceLock;

/// Targets on which the native tier is proven, so [`available`] may be true there.
///
/// VERIFIED for Windows x86_64: the whole `machine::sb::native` suite (module, executable memory,
/// C-ABI call, exits, `Send`, the tail-call spike) ran on a Windows 11 x86_64 host (cranelift
/// 0.116.1). Ubuntu x86_64 and macOS (x86_64 and arm64) are ASSUMED to work (the same
/// Cranelift code path) and stay out until the CI run of the same suite is green on them; the
/// spike (`spike::ping_pong`) and [`host_runs_native`] run on every host regardless, so that CI
/// run produces the evidence. Add a target here only with that evidence, in the same commit as
/// `spike::TAIL_TARGET_PROVEN` if chaining is proven too (a test checks the order). Design note
/// 9: an unproven target keeps `available()` false.
pub const NATIVE_TARGET_PROVEN: bool = cfg!(all(windows, target_arch = "x86_64"));

/// Whether this host can run native code at all, proven target or not: executable memory can be
/// allocated and a compiled function called ([`rt::probe`]). The first caller probes, every
/// later caller reads the cached answer; nothing is persisted (D129). The spike and the evidence
/// tests use this; the product asks [`available`].
pub fn host_runs_native() -> bool {
    static ANSWER: OnceLock<bool> = OnceLock::new();
    *ANSWER.get_or_init(rt::probe)
}

/// The decision of [`available`]: native code runs only on a proven target whose host passes the
/// probe.
const fn gate(proven: bool, host_runs: bool) -> bool {
    proven && host_runs
}

/// Whether the native tier may be used on this host: the target is proven
/// ([`NATIVE_TARGET_PROVEN`]) and the host runs native code ([`host_runs_native`]). A false
/// answer means no `NativeBackend` exists and the driver stays on tier 2.
pub fn available() -> bool {
    gate(NATIVE_TARGET_PROVEN, host_runs_native())
}

/// Whether trace chaining may use `return_call_indirect` (design note 9, stage 2 of T3): native
/// code is available, the target is one the spike proved ([`spike::TAIL_TARGET_PROVEN`]) and the
/// ping-pong of [`spike::tail_call_ping_pong`] ran clean on this host. False means chaining v1,
/// the dispatcher return. Cached like [`host_runs_native`].
pub fn tail_calls_available() -> bool {
    static ANSWER: OnceLock<bool> = OnceLock::new();
    *ANSWER.get_or_init(|| {
        available()
            && spike::TAIL_TARGET_PROVEN
            && std::panic::catch_unwind(|| spike::tail_call_ping_pong(spike::PROBE_ITERATIONS))
                .ok()
                .flatten()
                .is_some_and(|r| r.is_clean())
    })
}

#[cfg(test)]
mod tests {
    use super::emit::testing::assert_fallback;
    use super::*;

    #[test]
    fn availability_is_the_gate_over_the_proven_target_and_the_host_probe() {
        assert_eq!(available(), gate(NATIVE_TARGET_PROVEN, rt::probe()));
        assert_eq!(available(), host_runs_native() && NATIVE_TARGET_PROVEN);
        assert_eq!(
            host_runs_native(),
            host_runs_native(),
            "the answer is cached"
        );
        assert_eq!(NativeBackend::new().is_some(), available());
    }

    #[test]
    fn an_unproven_target_keeps_native_unavailable_whatever_the_host_can_do() {
        for (proven, host) in [(false, false), (false, true), (true, false), (true, true)] {
            assert_eq!(gate(proven, host), proven && host, "{proven} {host}");
        }
        assert!(
            !gate(false, true),
            "a host that runs native code is not enough"
        );
        assert!(
            !gate(true, false),
            "a proven target on a host without executable memory"
        );
    }

    #[test]
    fn where_native_code_is_unavailable_there_is_neither_a_backend_nor_chaining() {
        if available() {
            assert!(rt::Jit::new().is_some() && NativeBackend::new().is_some());
        } else {
            assert_fallback();
        }
    }

    #[test]
    fn chaining_needs_native_code_and_a_proven_target() {
        if tail_calls_available() {
            assert!(available() && spike::TAIL_TARGET_PROVEN);
        }
        if !spike::TAIL_TARGET_PROVEN {
            assert!(
                !tail_calls_available(),
                "an unproven target keeps chaining v1"
            );
        }
        assert!(
            !spike::TAIL_TARGET_PROVEN || NATIVE_TARGET_PROVEN,
            "a target proven for tail calls is proven for native code first"
        );
    }
}
