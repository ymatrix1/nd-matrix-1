//! OWNER: W1-NCORE. Tier 3: native code from the superblock IR through Cranelift (feature
//! `sb-native`, off by default).
//!
//! Spec: design note section 9. One `Sb` becomes one `extern "C" fn(*mut Cpu, *mut NativeCtl) ->
//! u32`; registers and flags are Cranelift variables, memory goes through the page table with
//! side exits to the interpreter, exits store the dirty state and leave through the driver.
//! [`available`] is the runtime probe: when it is false (no executable memory, a hardened
//! runtime, an unsupported host) there is no backend and everything runs on tier 2, exactly.
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
//! emission vocabulary, `emit::testing` the helpers of native tests, [`tail_calls_available`]
//! gates chaining v2.

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

/// Whether native code can run on this host: executable memory can be allocated and a compiled
/// function called. The first caller runs [`rt::probe`], every later caller reads the cached
/// answer; nothing is persisted (D129). A false answer means no `NativeBackend` exists and the
/// driver stays on tier 2.
pub fn available() -> bool {
    static ANSWER: OnceLock<bool> = OnceLock::new();
    *ANSWER.get_or_init(rt::probe)
}

/// Whether trace chaining may use `return_call_indirect` (design note 9, stage 2 of T3): native
/// code is available, the target is one the spike proved ([`spike::TAIL_TARGET_PROVEN`]) and the
/// ping-pong of [`spike::tail_call_ping_pong`] ran clean on this host. False means chaining v1,
/// the dispatcher return. Cached like [`available`].
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
    use super::*;

    #[test]
    fn availability_is_the_probe_and_the_backend_follows_it() {
        assert_eq!(available(), rt::probe());
        assert_eq!(available(), available(), "the answer is cached");
        assert_eq!(NativeBackend::new().is_some(), available());
    }

    #[test]
    fn where_native_code_is_unavailable_there_is_neither_a_backend_nor_chaining() {
        if available() {
            assert!(rt::Jit::new().is_some());
        } else {
            assert!(NativeBackend::new().is_none());
            assert!(!tail_calls_available());
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
    }
}
