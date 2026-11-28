//! OWNER: W2-NDP (tests). The rig of the native differential tests of this family: a trace of
//! data-processing uops, compiled through `rt::Jit` with a test lowerer (`lower_dp` plus `PRED`
//! and `EXIT`, the two flow uops a trace needs), run natively and on the tier-2 executor
//! (`exec::run_sb`) from the same state, and compared on every register, all flags, the
//! retired-instruction count, the exit and its target.
//!
//! Native code exists only where `native::available()`: [`Rig::new`] is `None` elsewhere, after
//! asserting the fallback and printing why, so a test that needs native code skips with a
//! reason instead of failing. The lowering decisions are tested without it (`lower_dp.rs`).

use super::{cond_holds, lower_dp};
use crate::cpu::Cpu;
use crate::machine::sb::exec::run_sb;
use crate::machine::sb::native::codegen::build_function_with;
use crate::machine::sb::native::emit::testing::{assert_fallback, verified};
use crate::machine::sb::native::emit::{NCtx, Unsupported};
use crate::machine::sb::native::rt::{Jit, Native, NativeRun};
use crate::machine::sb::testkit::{edge_cpu, random_cpu, Rng};
use crate::machine::sb::uop::{k, pack_flags, unpack_flags, Exit, ExitKind, Sb, Uop};
use crate::machine::SysBus;
use crate::mem::Mem;

/// Where the traces of the rig sit and where their exit leads.
pub(super) const PC: u32 = 0x1000;
pub(super) const TARGET: u32 = 0x2000;

/// The lowerer of the rig: `PRED` (condition from [`cond_holds`], body through the lowerer
/// itself), `EXIT`, and every data-processing kind through [`lower_dp`].
pub(super) fn lower_test(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
    match u.kind {
        k::PRED => {
            let cond = u.cond;
            cx.predicated(u.imm as usize, |cx| cond_holds(cx, cond))
        }
        k::EXIT => cx.exit(u.exit_idx()),
        _ => lower_dp(cx, u),
    }
}

/// What a run proved it did, from the tier-2 side (the oracle), never from the native side.
#[derive(Clone, Debug, Default)]
pub(super) struct Tally {
    pub cases: u64,
    pub reg_changed: u64,
    pub flags_changed: u64,
    /// Flag bits (nibble layout) that changed in at least one case.
    pub bits_changed: u8,
    /// Flag bits seen set / clear in a result.
    pub seen_set: u8,
    pub seen_clear: u8,
}

impl Tally {
    fn note(&mut self, before: &Cpu, after: &Cpu) {
        let (fb, fa) = (pack_flags(before), pack_flags(after));
        self.cases += 1;
        self.reg_changed += u64::from(before.r[..15] != after.r[..15]);
        self.flags_changed += u64::from(fb != fa);
        self.bits_changed |= fb ^ fa;
        self.seen_set |= fa;
        self.seen_clear |= !fa & 0xF;
    }

    /// Every flag bit changed, was set and was clear in some case.
    pub fn all_flag_bits_exercised(&self) -> bool {
        self.flag_bits_exercised(0xF)
    }

    /// Exactly the flag bits of `mask` changed, and each was set and clear in some case (the
    /// others never changed: a logic op leaves V alone).
    pub fn flag_bits_exercised(&self, mask: u8) -> bool {
        self.bits_changed == mask && self.seen_set & mask == mask && self.seen_clear & mask == mask
    }
}

/// A native module with its memory and the tier-2 bus.
pub(super) struct Rig {
    jit: Jit,
    mem: Mem,
    bus: SysBus,
}

impl Rig {
    /// The rig, or `None` (after asserting the fallback and printing the reason) where native
    /// code is not available.
    pub fn new() -> Option<Rig> {
        if !crate::machine::sb::native::available() {
            assert_fallback();
            eprintln!("skipped: native code is not available on this host (native::available())");
            return None;
        }
        Some(Rig {
            jit: Jit::new().expect("available() implies a module"),
            mem: Mem::new(),
            bus: SysBus::new(),
        })
    }

    /// A trace at [`PC`] of `uops` followed by an `EXIT` to [`TARGET`]; the guest instruction
    /// count is the sum of the `n` of the uops.
    pub fn trace(uops: &[Uop]) -> Sb {
        let done: u16 = uops.iter().map(|u| u16::from(u.n)).sum();
        let mut all = uops.to_vec();
        all.push(Uop::exit(0, PC + 4 * u32::from(done)));
        let exit = Exit::new(ExitKind::Direct, done, PC + 4 * u32::from(done), TARGET);
        Sb::from_parts(PC, done, all, vec![exit], vec![(PC, 0)]).expect("a small trace")
    }

    /// Compile `sb` with the test lowerer; panics when the module refuses it.
    pub fn compile(&mut self, sb: &Sb) -> Native {
        let func = build_function_with(sb, self.jit.call_conv(), self.jit.ptr_type(), lower_test)
            .expect("the trace ends in an exit");
        self.jit
            .define_native(verified(func))
            .expect("the module accepts the function")
    }

    /// Bytes of machine code defined so far.
    pub fn code_bytes(&self) -> usize {
        self.jit.code_bytes()
    }

    /// Run the native function from `cpu` whatever the exit: the report and the state.
    pub fn run_raw(&mut self, n: &Native, cpu: &Cpu) -> (NativeRun, Cpu) {
        let mut after = cpu.clone();
        let out = self.jit.run(n, &mut after, &mut self.mem, 1000);
        (out, after)
    }

    /// Run the native function from `cpu`: the resulting state.
    pub fn run_native(&mut self, _sb: &Sb, n: &Native, cpu: &Cpu) -> Cpu {
        let mut after = cpu.clone();
        let out = self.jit.run(n, &mut after, &mut self.mem, 1000);
        assert!(!out.is_generic(), "the trace leaves through its exit");
        after
    }

    /// Run `sb` natively and on the tier-2 executor from `cpu` and compare everything; `Err`
    /// names the first difference. The oracle's counters go to `t`.
    pub fn check(&mut self, sb: &Sb, n: &Native, cpu: &Cpu, t: &mut Tally) -> Result<(), String> {
        let mut want = cpu.clone();
        let mut left = 1000u64;
        let out = run_sb(&mut want, &mut self.bus, sb, &mut left, false);
        let mut got = cpu.clone();
        let run = self.jit.run(n, &mut got, &mut self.mem, 1000);
        let same = got.r == want.r
            && pack_flags(&got) == pack_flags(&want)
            && got.insns == want.insns
            && got.thumb == want.thumb
            && u32::from(out.exit) == run.exit
            && out.target == run.target;
        if !same {
            return Err(format!(
                "uops {:?}\nfrom r {:x?} f {:04b}\nnative r {:x?} f {:04b} insns {} exit {} target {:#x}\ntier2  r {:x?} f {:04b} insns {} exit {} target {:#x}",
                &sb.uops[..sb.uops.len() - 1],
                cpu.r,
                pack_flags(cpu),
                got.r,
                pack_flags(&got),
                got.insns,
                run.exit,
                run.target,
                want.r,
                pack_flags(&want),
                want.insns,
                out.exit,
                out.target,
            ));
        }
        t.note(cpu, &want);
        Ok(())
    }
}

/// A random state at [`PC`]: registers from the biased or the edge distribution, flags from a
/// random nibble.
pub(super) fn random_state(rng: &mut Rng) -> Cpu {
    let mut cpu = if rng.flag() {
        edge_cpu(rng, PC)
    } else {
        random_cpu(rng, PC)
    };
    unpack_flags(&mut cpu, rng.below(16) as u8);
    cpu
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rig_compares_native_with_tier_2_and_notices_a_difference() {
        let Some(mut rig) = Rig::new() else {
            return;
        };
        // r3 = r1 + r2 with flags
        let mut u = Uop::new(k::dp(1, 4, true), PC);
        (u.rd, u.rn, u.rm) = (3, 1, 2);
        let sb = Rig::trace(&[u]);
        let n = rig.compile(&sb);
        let mut rng = Rng::new(11);
        let mut t = Tally::default();
        for _ in 0..200 {
            let cpu = random_state(&mut rng);
            rig.check(&sb, &n, &cpu, &mut t).unwrap();
        }
        assert_eq!(t.cases, 200);
        assert!(t.reg_changed > 150 && t.flags_changed > 50, "{t:?}");
        // a native function of another trace differs from this trace's tier-2 run
        let mut other = Uop::new(k::dp(1, 2, true), PC);
        (other.rd, other.rn, other.rm) = (3, 1, 2);
        let wrong = rig.compile(&Rig::trace(&[other]));
        let mut rng = Rng::new(12);
        let caught = (0..50).any(|_| {
            let cpu = random_state(&mut rng);
            rig.check(&sb, &wrong, &cpu, &mut Tally::default()).is_err()
        });
        assert!(caught, "the comparator must catch a different function");
    }
}
