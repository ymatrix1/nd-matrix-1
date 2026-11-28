//! OWNER: W1-NCORE. The spike of tail-call chaining (design note 9, "Spike first"; graft 7,
//! risk R7): two compiled functions that hand control to each other with
//! `return_call_indirect` in the `Tail` calling convention, entered once from Rust through a
//! thunk in the host's C ABI.
//!
//! What it proves, on every CI host where [`super::host_runs_native`] is true (a target not yet
//! proven for [`super::available`] included, so the CI run is the evidence): executable memory
//! can be allocated and called, the host accepts `return_call_indirect` (Cranelift allows tail calls
//! only between `Tail` functions, so the chain is `Tail` and the entry is a C-ABI thunk), the
//! chain really is a jump (millions of hops run on a 256 KiB thread stack; a call chain would
//! overflow it), and the module moves to another thread (`Send`). Every function reads the next
//! target from a link slot at a fixed address, which is the shape of chaining v2: a slot patched
//! when the successor is compiled.
//!
//! Not a feature of the product: [`super::tail_calls_available`] keeps chaining v1 (return to
//! the dispatcher, about 20 cycles) on every host not listed in [`TAIL_TARGET_PROVEN`].

use super::abi::{CTL_EXIT, CTL_LIMIT, OFF_INSNS, OFF_R};
use super::codegen::trace_signature;
use super::rt::{Jit, Native};
use crate::cpu::Cpu;
use crate::mem::Mem;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, Function, InstBuilder, MemFlags, Type, UserFuncName};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Targets on which the ping-pong was run and passed, so chaining v2 may be switched on.
///
/// VERIFIED for Windows x86_64: `the_ping_pong_alternates_and_never_grows_the_stack`, run by the
/// work package on a Windows 11 x86_64 host (cranelift 0.116.1, 4,000,000 hops). Ubuntu x86_64
/// and macOS (x86_64 and arm64) are ASSUMED to work (the same Cranelift tail-call lowering) and
/// stay out of this list until the CI run of the same test is green on them; add a target here
/// only with that evidence.
pub const TAIL_TARGET_PROVEN: bool = cfg!(all(windows, target_arch = "x86_64"));

/// Hops of the probe `tail_calls_available` runs once per process.
///
/// ASSUMED: 100,000 is far past any stack a non-tail chain could use on a 256 KiB thread and
/// costs well under a millisecond; the test uses 4,000,000.
pub const PROBE_ITERATIONS: u64 = 100_000;

/// Exit value of a finished chain.
///
/// VERIFIED: arbitrary tag, compared by `SpikeReport::is_clean` against the value the compiled
/// function stored in `ctl.exit` and returned.
const DONE: u32 = 7;

/// Stack of the thread the chain runs on.
///
/// ASSUMED: 256 KiB holds the thread start-up and one thunk frame on every supported OS; a
/// 4,000,000-hop chain of real calls would need at least 64 MB.
const STACK: usize = 256 << 10;

/// What one ping-pong run observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpikeReport {
    /// Hops requested.
    pub iterations: u64,
    /// `cpu.insns` after the run (every hop adds one).
    pub insns: u64,
    /// Entries of the first function (`r0`) and of the second (`r1`).
    pub a_visits: u32,
    pub b_visits: u32,
    /// The exit value the chain returned.
    pub exit: u32,
    /// Low 32 bits of the stack pointer inside each function (`[first, second]`) at its first
    /// entry and at its last: equal in a chain of jumps, far apart in a chain of calls.
    pub sp_first: [u32; 2],
    pub sp_last: [u32; 2],
}

impl SpikeReport {
    /// Every hop counted once, the two functions alternated, the chain ended with its exit.
    pub fn is_clean(&self) -> bool {
        self.exit == DONE
            && self.insns == self.iterations
            && u64::from(self.a_visits) + u64::from(self.b_visits) == self.iterations
            && self.a_visits.abs_diff(self.b_visits) <= 1
            && self.a_visits >= self.b_visits
            && self.sp_first == self.sp_last
            && (self.sp_first[0] != 0) == (self.a_visits > 0)
            && (self.sp_first[1] != 0) == (self.b_visits > 0)
    }
}

/// How a chain function reaches its successor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chain {
    /// `return_call_indirect` in the `Tail` convention: a jump, the stack does not grow.
    Tail,
    /// `call_indirect` and `return` in the host's C ABI: a call per hop. The negative control of
    /// the spike (the stack grows, so [`SpikeReport::is_clean`] must say no).
    Calls(CallConv),
}

impl Chain {
    fn call_conv(self) -> CallConv {
        match self {
            Chain::Tail => CallConv::Tail,
            Chain::Calls(cc) => cc,
        }
    }

    fn signature(self, ptr: Type) -> cranelift_codegen::ir::Signature {
        trace_signature(self.call_conv(), ptr)
    }
}

/// A chain function: count a visit in `r[visit]`, `insns += 1`, finish when `insns >= limit`,
/// else continue through the link slot at `next_slot` the way `chain` says.
pub fn link_function(ptr: Type, chain: Chain, next_slot: usize, visit: usize) -> Function {
    let mut func = Function::with_name_signature(UserFuncName::default(), chain.signature(ptr));
    let mut fctx = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut func, &mut fctx);
    let (entry, fin, go) = (b.create_block(), b.create_block(), b.create_block());
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    b.seal_block(entry);
    let (cpu, ctl) = (b.block_params(entry)[0], b.block_params(entry)[1]);
    let m = MemFlags::trusted();
    let off = (OFF_R + 4 * visit) as i32;
    let n = b.ins().load(types::I32, m, cpu, off);
    let n = b.ins().iadd_imm(n, 1);
    b.ins().store(m, n, cpu, off);
    // the stack pointer of this hop: `r[2 + visit]` = latest, `r[4 + visit]` = first ever seen
    let sp = b.ins().get_stack_pointer(ptr);
    let sp = b.ins().ireduce(types::I32, sp);
    b.ins().store(m, sp, cpu, (OFF_R + 4 * (2 + visit)) as i32);
    let first_off = (OFF_R + 4 * (4 + visit)) as i32;
    let first = b.ins().load(types::I32, m, cpu, first_off);
    let unset = b.ins().icmp_imm(IntCC::Equal, first, 0);
    let first = b.ins().select(unset, sp, first);
    b.ins().store(m, first, cpu, first_off);
    let insns = b.ins().load(types::I64, m, cpu, OFF_INSNS as i32);
    let insns = b.ins().iadd_imm(insns, 1);
    b.ins().store(m, insns, cpu, OFF_INSNS as i32);
    let limit = b.ins().load(types::I64, m, ctl, CTL_LIMIT as i32);
    let reached = b
        .ins()
        .icmp(IntCC::UnsignedGreaterThanOrEqual, insns, limit);
    b.ins().brif(reached, fin, &[], go, &[]);
    b.seal_block(fin);
    b.switch_to_block(fin);
    let done = b.ins().iconst(types::I32, i64::from(DONE));
    b.ins().store(m, done, ctl, CTL_EXIT as i32);
    b.ins().return_(&[done]);
    b.seal_block(go);
    b.switch_to_block(go);
    let slot = b.ins().iconst(ptr, next_slot as i64);
    let callee = b.ins().load(ptr, m, slot, 0);
    let sig = b.import_signature(chain.signature(ptr));
    match chain {
        Chain::Tail => {
            b.ins().return_call_indirect(sig, callee, &[cpu, ctl]);
        }
        Chain::Calls(_) => {
            let call = b.ins().call_indirect(sig, callee, &[cpu, ctl]);
            let r = b.inst_results(call)[0];
            b.ins().return_(&[r]);
        }
    }
    b.finalize();
    func
}

/// The C-ABI entry: `call_indirect` of the chain function whose address is in `first_slot`.
pub fn entry_thunk(ptr: Type, c_abi: CallConv, chain: Chain, first_slot: usize) -> Function {
    let mut func =
        Function::with_name_signature(UserFuncName::default(), trace_signature(c_abi, ptr));
    let mut fctx = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut func, &mut fctx);
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    b.seal_block(entry);
    let (cpu, ctl) = (b.block_params(entry)[0], b.block_params(entry)[1]);
    let slot = b.ins().iconst(ptr, first_slot as i64);
    let callee = b.ins().load(ptr, MemFlags::trusted(), slot, 0);
    let sig = b.import_signature(chain.signature(ptr));
    let call = b.ins().call_indirect(sig, callee, &[cpu, ctl]);
    let r = b.inst_results(call)[0];
    b.ins().return_(&[r]);
    b.finalize();
    func
}

/// Run the ping-pong for `iterations` hops (at least 1). `None` where native code is unavailable
/// or the module refuses the functions (the clean fallback); `Some` carries what the chain did,
/// and a report that is not [`SpikeReport::is_clean`] is a miscompile of tail calls on this host.
pub fn tail_call_ping_pong(iterations: u64) -> Option<SpikeReport> {
    ping_pong(Chain::Tail, iterations)
}

/// [`tail_call_ping_pong`] with the chain style as a parameter (the negative control uses
/// [`Chain::Calls`]).
pub fn ping_pong(chain: Chain, iterations: u64) -> Option<SpikeReport> {
    if !super::host_runs_native() {
        return None;
    }
    let iterations = iterations.max(1);
    let mut jit = Jit::new()?;
    let slots: Box<[AtomicUsize; 2]> = Box::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
    let addr = |i: usize| &slots[i] as *const AtomicUsize as usize;
    let ptr = jit.ptr_type();
    let a = jit.define_native(link_function(ptr, chain, addr(1), 0))?;
    let b = jit.define_native(link_function(ptr, chain, addr(0), 1))?;
    let entry = jit.define_native(entry_thunk(ptr, jit.call_conv(), chain, addr(0)))?;
    slots[0].store(a.addr(), Ordering::SeqCst);
    slots[1].store(b.addr(), Ordering::SeqCst);
    let run = move |jit: Jit, entry: Native, slots: Box<[AtomicUsize; 2]>| {
        let (mut cpu, mut mem) = (Cpu::new(), Mem::new());
        let out = jit.run(&entry, &mut cpu, &mut mem, iterations);
        drop(slots);
        SpikeReport {
            iterations,
            insns: cpu.insns,
            a_visits: cpu.r[0],
            b_visits: cpu.r[1],
            exit: out.exit,
            sp_first: [cpu.r[4], cpu.r[5]],
            sp_last: [cpu.r[2], cpu.r[3]],
        }
    };
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || run(jit, entry, slots))
        .ok()?
        .join()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cranelift_codegen::settings;
    use cranelift_codegen::verify_function;

    fn verify(f: &Function) {
        if let Err(e) = verify_function(f, &settings::Flags::new(settings::builder())) {
            panic!("{e}\n{f}");
        }
    }

    #[test]
    fn the_chain_functions_and_the_thunk_verify_without_executable_memory() {
        let ptr = types::I64;
        let a = link_function(ptr, Chain::Tail, 0x1000, 0);
        verify(&a);
        let text = a.display().to_string();
        assert!(text.contains("return_call_indirect"), "{text}");
        assert!(
            text.contains("tail"),
            "the chain is in the Tail convention: {text}"
        );
        assert_eq!(text.matches("return ").count(), 1, "one finishing return");
        let t = entry_thunk(ptr, CallConv::SystemV, Chain::Tail, 0x1000);
        verify(&t);
        let text = t.display().to_string();
        assert!(
            text.contains("call_indirect") && !text.contains("return_call"),
            "{text}"
        );
    }

    #[test]
    fn the_report_flags_every_way_a_chain_can_go_wrong() {
        let ok = SpikeReport {
            iterations: 5,
            insns: 5,
            a_visits: 3,
            b_visits: 2,
            exit: DONE,
            sp_first: [0x1000, 0x2000],
            sp_last: [0x1000, 0x2000],
        };
        assert!(ok.is_clean());
        let one_hop = SpikeReport {
            iterations: 1,
            insns: 1,
            a_visits: 1,
            b_visits: 0,
            exit: DONE,
            sp_first: [0x1000, 0],
            sp_last: [0x1000, 0],
        };
        assert!(
            one_hop.is_clean(),
            "an unvisited function has no stack pointer"
        );
        let bad = [
            SpikeReport { exit: 0, ..ok },
            SpikeReport { insns: 4, ..ok },
            SpikeReport { a_visits: 4, ..ok },
            SpikeReport {
                a_visits: 5,
                b_visits: 0,
                ..ok
            },
            SpikeReport {
                a_visits: 2,
                b_visits: 3,
                ..ok
            },
            SpikeReport {
                sp_last: [0x1000u32.wrapping_sub(16 * 4_000_000), 0x2000],
                ..ok
            },
            SpikeReport {
                sp_first: [0, 0],
                sp_last: [0, 0],
                ..ok
            },
        ];
        for (i, r) in bad.iter().enumerate() {
            assert!(!r.is_clean(), "defect {i}: {r:?}");
        }
    }

    /// Run the spike. On a target that is not in [`TAIL_TARGET_PROVEN`] a Cranelift panic is
    /// the clean fallback (reported on stderr, chaining stays v1); on a proven one it is a
    /// failure. A report that comes back is always judged.
    fn spike(chain: Chain, hops: u64) -> Option<SpikeReport> {
        match std::panic::catch_unwind(|| ping_pong(chain, hops)) {
            Ok(report) => report,
            Err(e) if TAIL_TARGET_PROVEN => std::panic::resume_unwind(e),
            Err(_) => {
                eprintln!("tail-call spike: the module panicked on an unproven target");
                None
            }
        }
    }

    #[test]
    fn the_ping_pong_alternates_and_never_grows_the_stack() {
        match spike(Chain::Tail, 4_000_000) {
            None => {
                assert!(
                    !super::super::host_runs_native() || !TAIL_TARGET_PROVEN,
                    "a proven target must run the spike"
                );
                eprintln!("tail-call spike: not run (native unavailable or module refused)");
            }
            Some(r) => {
                assert!(r.is_clean(), "{r:?}");
                assert_eq!((r.a_visits, r.b_visits), (2_000_000, 2_000_000));
                eprintln!("tail-call spike: clean, {r:?}");
            }
        }
    }

    #[test]
    fn a_chain_of_plain_calls_is_caught_by_the_stack_pointer_check() {
        if !super::super::host_runs_native() {
            assert_eq!(ping_pong(Chain::Calls(CallConv::SystemV), 10), None);
            return;
        }
        let cc = Jit::new().unwrap().call_conv();
        let Some(calls) = spike(Chain::Calls(cc), 1000) else {
            assert!(!TAIL_TARGET_PROVEN, "a proven target must run the control");
            return;
        };
        // everything but the stack is as in a tail chain ...
        assert_eq!(
            (calls.exit, calls.insns, calls.a_visits, calls.b_visits),
            (DONE, 1000, 500, 500)
        );
        // ... and the calls did grow it: the first and the last entry of a function differ
        assert!(!calls.is_clean(), "{calls:?}");
        assert_ne!(calls.sp_first[0], calls.sp_last[0], "{calls:?}");
        if let Some(jump) = spike(Chain::Tail, 1000) {
            assert!(jump.is_clean(), "{jump:?}");
        }
    }

    #[test]
    fn a_chain_of_one_two_and_three_hops_ends_at_the_right_function() {
        if !super::super::host_runs_native() {
            assert_eq!(tail_call_ping_pong(3), None);
            return;
        }
        let want = [(1, 1, 0), (2, 1, 1), (3, 2, 1), (0, 1, 0)];
        for (n, a, b) in want {
            let Some(r) = spike(Chain::Tail, n) else {
                assert!(!TAIL_TARGET_PROVEN, "a proven target must run the spike");
                return;
            };
            assert!(r.is_clean(), "{r:?}");
            assert_eq!((r.a_visits, r.b_visits), (a, b), "{n} hops (0 counts as 1)");
        }
    }

    #[test]
    fn chaining_is_enabled_only_when_the_spike_ran_clean_on_a_proven_target() {
        let on = super::super::tail_calls_available();
        let clean = spike(Chain::Tail, PROBE_ITERATIONS).is_some_and(|r| r.is_clean());
        assert_eq!(on, super::super::available() && TAIL_TARGET_PROVEN && clean);
    }
}
