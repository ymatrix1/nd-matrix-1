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
//! | `abi` `emit` `codegen` `rt` | W1-NCORE |
//! | `lower_dp` | W2-NDP |
//! | `lower_mem` `lower_flow` | W2-NMEM |
//! | `backend` `link` `difftest` | W3-NINT |

pub mod abi;
pub mod backend;
pub mod codegen;
#[cfg(test)]
mod difftest;
pub mod emit;
pub mod link;
pub mod lower_dp;
pub mod lower_flow;
pub mod lower_mem;
pub mod rt;

pub use backend::NativeBackend;
pub use emit::{NCtx, Unsupported};

/// Whether native code can run on this host (cached by the first caller after W1-NCORE; the
/// skeleton answers from [`rt::probe`], which is `false`).
pub fn available() -> bool {
    rt::probe()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_is_the_probe_and_the_backend_follows_it() {
        assert_eq!(available(), rt::probe());
        assert_eq!(NativeBackend::new().is_some(), available());
    }
}
