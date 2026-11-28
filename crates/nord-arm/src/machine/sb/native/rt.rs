//! OWNER: W1-NCORE. The only `unsafe` of the ARM0 tiers: the C-ABI call of a compiled trace, the
//! `Send` wrapper of the JIT module, the module wrapper [`Jit`] and the runtime availability
//! probe.
//!
//! Spec (design note 9, WP W1-NCORE): `machine/sb/mod.rs` has `#![deny(unsafe_code)]`; this file
//! alone allows it, for exactly four things. (1) [`call`]: invoke `unsafe extern "C" fn(*mut
//! Cpu, *mut NativeCtl) -> u32`. (2) [`Jit`]: owns the `JITModule`; `define` / `define_native`
//! compile an IR function and do the code-pointer transmute, `run` is the safe entry (the
//! handle carries the owner and generation, so a function of another module or of a module
//! that was reset is refused instead of called), `free_all` resets the module. W2-NDP and
//! W2-NMEM run their native tests through it and W3-NINT's `NativeBackend` wraps the same type,
//! so no other file needs `unsafe`. (3) [`SendCell`]: a single-owner wrapper that is `Send`
//! because the module is only ever used by the one thread that owns the `Machine` (I10,
//! `Machine: Send`). (4) [`probe`]: allocate executable memory, run a one-instruction function,
//! report whether it worked; the answer feeds `native::available()` and is never persisted
//! (D129).
//!
//! Trust boundary: the IR handed to `define` comes from this crate's lowerers and the Cranelift
//! verifier runs on every function (`enable_verifier` is on), but a lowerer that emits a wild
//! store is a bug no type can catch; `run` is safe in the Rust sense (liveness, ABI and
//! pointer refresh are checked), exactness is the job of the differential tests.

#![allow(unsafe_code)]

use super::abi::{NativeCtl, CTL_EXIT, EXIT_GENERIC};
use super::codegen::{build_function, trace_signature};
use crate::cpu::Cpu;
use crate::machine::sb::uop::Sb;
use crate::mem::raw::RawMem;
use crate::mem::Mem;
use cranelift_codegen::ir::{types, Function, InstBuilder, MemFlags, Type, UserFuncName};
use cranelift_codegen::isa::{CallConv, OwnedTargetIsa};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, Linkage, Module};
use std::sync::atomic::{AtomicU32, Ordering};

/// The compiled-trace entry point.
pub type NativeFn = unsafe extern "C" fn(*mut Cpu, *mut NativeCtl) -> u32;

/// Code bytes one [`Jit`] accepts before `define` refuses (the driver then resets the module at
/// a pass boundary, design note 9 "Code memory").
///
/// ASSUMED: 32 MiB, the design's cap; about 100 k traces at 300 bytes, far above the 4 M-uop
/// trace cache cap. A real firmware run would confirm the code size per trace (gate G3).
pub const CODE_CAP: usize = 32 << 20;

/// The value the probe function returns and stores in `ctl.exit`.
///
/// VERIFIED: arbitrary tag below `i32::MAX`, returned and stored in `ctl.exit`, both compared by
/// `probe_once` and `the_probe_is_deterministic_and_its_tag_fits_an_i32_constant`.
const PROBE_TAG: u32 = 0x2A5A_5A2A;

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

/// A compiled function of one [`Jit`] generation.
#[derive(Clone, Copy, Debug)]
pub struct Native {
    f: NativeFn,
    owner: u32,
    generation: u32,
    /// Default call convention and the `(ptr, ptr) -> i32` signature: callable through `run`.
    c_abi: bool,
}

impl Native {
    /// The raw entry point (valid until the owning module is reset or dropped).
    pub fn raw(&self) -> NativeFn {
        self.f
    }

    /// The code address, for link slots. Only meaningful for linking when the function was
    /// defined with a call convention the caller of the link understands.
    pub fn addr(&self) -> usize {
        self.f as usize
    }
}

/// What one native entry reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeRun {
    /// Exit index of the trace, or [`EXIT_GENERIC`].
    pub exit: u32,
    /// The new `r15` the epilogue stored in `ctl.target`.
    pub target: u32,
}

impl NativeRun {
    /// Native code left with the state exact before an instruction it does not handle.
    pub fn is_generic(&self) -> bool {
        self.exit == EXIT_GENERIC
    }
}

static NEXT_OWNER: AtomicU32 = AtomicU32::new(1);

fn new_module(isa: &OwnedTargetIsa) -> JITModule {
    JITModule::new(JITBuilder::with_isa(isa.clone(), default_libcall_names()))
}

/// The Cranelift module of the native tier: compiles IR functions into executable memory.
pub struct Jit {
    module: SendCell<Option<JITModule>>,
    ctx: SendCell<Context>,
    isa: OwnedTargetIsa,
    ptr: Type,
    call_conv: CallConv,
    owner: u32,
    generation: u32,
    named: u32,
    code_bytes: usize,
    cap: usize,
}

impl Jit {
    /// A module for the host ISA, or `None` when Cranelift does not support the host.
    pub fn new() -> Option<Jit> {
        let mut flags = settings::builder();
        flags.set("opt_level", "speed").ok()?;
        flags.set("use_colocated_libcalls", "false").ok()?;
        flags.set("is_pic", "false").ok()?;
        // Tail calls (chaining v2) need frame pointers: without them the x64 emitter panics
        // inside `define_function` ("the current implementation relies on them being present",
        // cranelift-codegen 0.116.1 `isa/x64/inst/emit.rs`), found by the spike. Always on, so
        // a module can hold chained and unchained traces; the cost (one general register in a
        // trace) is ASSUMED small and is part of gate G3's measurement.
        flags.set("preserve_frame_pointers", "true").ok()?;
        let isa = cranelift_native::builder()
            .ok()?
            .finish(settings::Flags::new(flags))
            .ok()?;
        let module = new_module(&isa);
        Some(Jit {
            module: SendCell(Some(module)),
            ctx: SendCell(Context::new()),
            ptr: isa.pointer_type(),
            call_conv: isa.default_call_conv(),
            isa,
            owner: NEXT_OWNER.fetch_add(1, Ordering::Relaxed),
            generation: 0,
            named: 0,
            code_bytes: 0,
            cap: CODE_CAP,
        })
    }

    /// The pointer type of the host (`cpu` and `ctl` arguments).
    pub fn ptr_type(&self) -> Type {
        self.ptr
    }

    /// The default call convention of the host: the one `extern "C"` uses.
    pub fn call_conv(&self) -> CallConv {
        self.call_conv
    }

    /// An empty function with the native signature `(cpu, ctl) -> i32` in the host's C ABI.
    pub fn new_function(&self) -> Function {
        Function::with_name_signature(
            UserFuncName::default(),
            trace_signature(self.call_conv, self.ptr),
        )
    }

    /// Bytes of machine code defined since the module was created or last reset.
    pub fn code_bytes(&self) -> usize {
        self.code_bytes
    }

    /// Lower the byte cap (a test of the overflow path; production keeps [`CODE_CAP`]).
    pub fn set_code_cap(&mut self, bytes: usize) {
        self.cap = bytes;
    }

    /// True once `define` refuses new functions (the driver resets the module).
    pub fn is_full(&self) -> bool {
        self.code_bytes >= self.cap
    }

    /// Compile `func` and return its handle, or `None` when the module is full or Cranelift
    /// rejects the function (the trace then stays on tier 2).
    pub fn define_native(&mut self, func: Function) -> Option<Native> {
        if self.is_full() {
            return None;
        }
        let module = self.module.0.as_mut()?;
        let sig = func.signature.clone();
        let c_abi = sig.call_conv == self.call_conv
            && sig.params.len() == 2
            && sig.params.iter().all(|p| p.value_type == self.ptr)
            && sig.returns.len() == 1
            && sig.returns[0].value_type == types::I32;
        let name = format!("t{}", self.named);
        self.named += 1;
        let id = module.declare_function(&name, Linkage::Local, &sig).ok()?;
        self.ctx.0.clear();
        self.ctx.0.func = func;
        if module.define_function(id, &mut self.ctx.0).is_err() {
            module.clear_context(&mut self.ctx.0);
            return None;
        }
        let size = self
            .ctx
            .0
            .compiled_code()
            .map_or(0, |c| c.code_info().total_size as usize);
        module.clear_context(&mut self.ctx.0);
        module.finalize_definitions().ok()?;
        self.code_bytes += size.max(1);
        let code = module.get_finalized_function(id);
        // SAFETY: `code` is the start of the function just finalized in executable memory; the
        // pointer type has the size of a function pointer and the `NativeFn` type is only ever
        // called (through `call` or `run`) for functions with the matching signature (`c_abi`).
        let f = unsafe { std::mem::transmute::<*const u8, NativeFn>(code) };
        Some(Native {
            f,
            owner: self.owner,
            generation: self.generation,
            c_abi,
        })
    }

    /// [`Jit::define_native`] reduced to the raw entry point.
    pub fn define(&mut self, func: Function) -> Option<NativeFn> {
        self.define_native(func).map(|n| n.f)
    }

    /// Build the IR of `sb` (`codegen::build_function`) and compile it. `None` leaves the trace
    /// to the T2 executor: an unterminated function, a rejected function or a full module.
    pub fn compile(&mut self, sb: &Sb) -> Option<Native> {
        let func = build_function(sb, self.call_conv, self.ptr)?;
        self.define_native(func)
    }

    /// Run `n` on `cpu` with the table and arena of `mem` (refreshed here, so a page mapped
    /// since the last entry cannot leave a stale pointer). `limit` is the absolute `cpu.insns`
    /// a loop back-edge may not run past.
    ///
    /// # Panics
    /// When `n` belongs to another module, to a generation before the last [`Jit::free_all`],
    /// or was defined with another signature or calling convention.
    pub fn run(&self, n: &Native, cpu: &mut Cpu, mem: &mut Mem, limit: u64) -> NativeRun {
        assert!(
            n.owner == self.owner && n.generation == self.generation,
            "native function of another or a reset module"
        );
        assert!(n.c_abi, "not a (cpu, ctl) -> i32 function in the C ABI");
        let mut ctl = NativeCtl::new(mem.raw_parts(), limit);
        // SAFETY: the handle is live (owner and generation match, checked above), it has the
        // trace signature, and `ctl` was filled from `mem`, which stays exclusively borrowed
        // for the whole call.
        let exit = unsafe { call(n.f, cpu, &mut ctl) };
        NativeRun {
            exit,
            target: ctl.target,
        }
    }

    /// Drop every compiled function and start a fresh module. Handles of earlier generations
    /// are refused by [`Jit::run`]; a raw `NativeFn` taken from them must not be called.
    pub fn free_all(&mut self) {
        let fresh = new_module(&self.isa);
        if let Some(old) = self.module.0.replace(fresh) {
            // SAFETY: `&mut self` means no native call of this module is running, and the
            // generation bump below makes every safe handle to it unusable.
            unsafe { old.free_memory() };
        }
        self.generation += 1;
        self.code_bytes = 0;
    }
}

impl Drop for Jit {
    fn drop(&mut self) {
        if let Some(m) = self.module.0.take() {
            // SAFETY: the owner is gone, so no safe handle can run these functions any more.
            // (cranelift-jit leaks the pages of a dropped module otherwise; on Windows its
            // `free_memory` still leaks them, design note risk R9 and follow-up F7.)
            unsafe { m.free_memory() };
        }
    }
}

/// Whether native code can run here: executable memory can be allocated and called. Never
/// panics, never persists its answer (D129); [`super::available`] caches it.
pub fn probe() -> bool {
    if !cfg!(all(
        target_pointer_width = "64",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )) {
        return false;
    }
    std::panic::catch_unwind(probe_once).unwrap_or(false)
}

fn probe_once() -> bool {
    let Some(mut jit) = Jit::new() else {
        return false;
    };
    let mut func = jit.new_function();
    let mut fctx = FunctionBuilderContext::new();
    {
        let mut b = FunctionBuilder::new(&mut func, &mut fctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        b.seal_block(entry);
        let ctl = b.block_params(entry)[1];
        let tag = b.ins().iconst(types::I32, i64::from(PROBE_TAG as i32));
        b.ins()
            .store(MemFlags::trusted(), tag, ctl, CTL_EXIT as i32);
        b.ins().return_(&[tag]);
        b.finalize();
    }
    let Some(n) = jit.define_native(func) else {
        return false;
    };
    let mut cpu = Cpu::new();
    let mut ctl = NativeCtl::new(
        RawMem {
            table: std::ptr::null(),
            arena: std::ptr::null_mut(),
        },
        0,
    );
    // SAFETY: the function was just compiled by this module (live) with the trace signature and
    // touches only `ctl.exit`; the null table and arena are never read.
    let exit = unsafe { call(n.f, &mut cpu, &mut ctl) };
    exit == PROBE_TAG && ctl.exit == PROBE_TAG
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::native::rt_tests::{adder, fresh};

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
        let (mut cpu, mut ctl) = fresh();
        cpu.r[0] = 41;
        // SAFETY: `fake_trace` has the contract of a compiled trace and touches no memory.
        let exit = unsafe { call(fake_trace, &mut cpu, &mut ctl) };
        assert_eq!((exit, cpu.r[1], cpu.insns, ctl.exit), (3, 42, 1, 3));
    }

    #[test]
    fn the_raw_entry_point_of_a_defined_function_is_callable_where_native_is_available() {
        if !super::super::available() {
            crate::machine::sb::native::emit::testing::assert_fallback();
            return;
        }
        let mut jit = Jit::new().expect("available() implies a module");
        let (mut cpu, mut ctl) = fresh();
        cpu.r[0] = 1;
        let raw = jit.define(adder(&jit, 1, 9)).unwrap();
        // SAFETY: `raw` was defined by the live `jit` with the trace signature and touches no
        // memory but `cpu` and `ctl`.
        let exit = unsafe { call(raw, &mut cpu, &mut ctl) };
        assert_eq!((exit, cpu.r[1], ctl.exit, cpu.insns), (9, 2, 9, 3));
        assert!(jit.code_bytes() > 0);
    }

    #[test]
    fn the_probe_is_deterministic_and_its_tag_fits_an_i32_constant() {
        assert_eq!(i64::from(PROBE_TAG as i32), i64::from(PROBE_TAG));
        assert_eq!(probe(), probe(), "the probe is deterministic");
        assert_eq!(probe(), super::super::host_runs_native());
    }
}
