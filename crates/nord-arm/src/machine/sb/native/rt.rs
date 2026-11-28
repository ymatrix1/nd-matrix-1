//! OWNER: W1-NCORE. The only `unsafe` of the ARM0 tiers: the C-ABI call of a compiled trace, the
//! `Send` wrapper of the JIT module, and the runtime availability probe.
//!
//! Spec (design note 9, WP W1-NCORE): `machine/sb/mod.rs` has `#![deny(unsafe_code)]`; this file
//! alone allows it, for exactly three things. (1) [`call`]: invoke `unsafe extern "C" fn(*mut
//! Cpu, *mut NativeCtl) -> u32`. WP W1-NCORE also adds here the module wrapper `Jit` (owns the
//! `JITModule`, `define(func) -> Option<NativeFn>` does the code-pointer transmute, `free_all`
//! resets it): W2-NDP and W2-NMEM run their native-against-T2 tests through it and W3-NINT's
//! `NativeBackend` wraps the same type, so no other file needs `unsafe`. (2) [`SendCell`]: a
//! single-owner wrapper that is `Send` because the module is only ever used by the one thread
//! that owns the `Machine` (I10, `Machine: Send`). (3) [`probe`]: allocate executable memory,
//! run a one-instruction function, report whether it worked; the answer feeds
//! `native::available()` and is never persisted (D129).
//!
//! The skeleton's [`probe`] answers `false` without touching executable memory.

#![allow(unsafe_code)]

use super::abi::NativeCtl;
use crate::cpu::Cpu;

/// The compiled-trace entry point.
pub type NativeFn = unsafe extern "C" fn(*mut Cpu, *mut NativeCtl) -> u32;

/// Call `f` on `cpu` and `ctl`; returns the exit index.
///
/// # Safety
/// `f` must be a function compiled from a trace for this layout of `Cpu` and `NativeCtl`, its
/// code memory must be live and executable, and `ctl.table` / `ctl.arena` must come from the
/// `Mem` of the machine that owns `cpu`, refreshed since the last mutable use of that `Mem`.
pub unsafe fn call(f: NativeFn, cpu: &mut Cpu, ctl: &mut NativeCtl) -> u32 {
    // SAFETY: the caller upholds the contract above; both pointers come from live `&mut`.
    unsafe { f(cpu as *mut Cpu, ctl as *mut NativeCtl) }
}

/// A value that is only touched by the thread that owns the machine (the JIT module holds raw
/// code pointers, which are not `Send` on their own).
pub struct SendCell<T>(pub T);

// SAFETY: a `SendCell` lives inside `NativeBackend`, which lives inside the `Machine`; the
// `Machine` is moved between threads only whole, never shared, and the code the cell owns is
// not tied to a thread.
unsafe impl<T> Send for SendCell<T> {}

/// Whether native code can run here: executable memory can be allocated and called.
pub fn probe() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::raw::RawMem;

    /// Stands in for a compiled trace: `r1 = r0 + 1`, `insns += 1`, exit 3.
    extern "C" fn fake_trace(cpu: *mut Cpu, ctl: *mut NativeCtl) -> u32 {
        // SAFETY: `call` passes pointers made from live `&mut` borrows.
        let (cpu, ctl) = unsafe { (&mut *cpu, &mut *ctl) };
        cpu.r[1] = cpu.r[0].wrapping_add(1);
        cpu.insns += 1;
        ctl.exit = 3;
        3
    }

    #[test]
    fn a_c_abi_function_is_called_with_the_cpu_and_the_control_block() {
        let (mut cpu, mut ctl) = (
            Cpu::new(),
            NativeCtl::new(
                RawMem {
                    table: std::ptr::null(),
                    arena: std::ptr::null_mut(),
                },
                10,
            ),
        );
        cpu.r[0] = 41;
        // SAFETY: `fake_trace` has the contract of a compiled trace and touches no memory.
        let exit = unsafe { call(fake_trace, &mut cpu, &mut ctl) };
        assert_eq!((exit, cpu.r[1], cpu.insns, ctl.exit), (3, 42, 1, 3));
    }

    #[test]
    fn the_cell_is_send_and_the_probe_is_false_until_the_native_work_lands() {
        fn is_send<T: Send>() {}
        is_send::<SendCell<*mut u8>>();
        assert!(!probe());
    }
}
