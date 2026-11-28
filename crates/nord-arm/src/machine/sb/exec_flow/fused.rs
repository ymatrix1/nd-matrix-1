//! OWNER: W2-XCORE. The arms of the fused uops (`k::CMP_GUARD_I`, `k::CMP_GUARD_R`,
//! `k::TST_GUARD_I`, `k::MOV_PAIR`, `k::LDR_PAIR`). Each is its unfused expansion
//! (`uop::fused::expand`, the definition): the same data-processing or memory arm run on the uop
//! the expansion names, in order, the second reading the state the first left, and `bus.pc` of
//! each load set by the memory arm from the pc of its half (`pc`, `pc + 4`, I5).
//!
//! The expansion is rebuilt here by field copy instead of through `expand` (no search of the
//! kind table, no `Option`); `the_fused_arms_equal_their_expansion_on_random_states` pins both
//! to each other and `... equal_the_interpreter ...` to `Cpu::step` on the two instructions.
//!
//! No fused uop stores, so none needs an early exit; a `CMP_GUARD_*` reports whether its guard
//! holds and the caller leaves through the exit (`rs`) of the uop.

use super::mutant;
use crate::machine::sb::bus::SbBus;
use crate::machine::sb::exec_alu::exec_dp;
use crate::machine::sb::exec_mem::exec_mem;
use crate::machine::sb::uop::{holds, k, Uop};

/// The compare or test half of a `CMP_GUARD_*` / `TST_GUARD_I`, then the guard: `true` when the
/// trace leaves through the exit of `u`.
#[inline(always)]
pub(super) fn cmp_guard(r: &mut [u32; 16], f: &mut u8, u: &Uop) -> bool {
    let kind = match u.kind {
        k::CMP_GUARD_I => k::CMP_IMM,
        k::CMP_GUARD_R => k::CMP_REG,
        _ => k::TST_IMM,
    };
    let mut a = Uop::new(kind, u.pc);
    (a.rn, a.rm, a.imm, a.sh) = (u.rn, u.rm, u.imm, u.sh);
    if mutant(420) && kind == k::TST_IMM {
        a.sh = 1; // the shifter carry is always the old C
    }
    if mutant(416) {
        exec_dp(r, &mut f.clone(), &a);
    } else {
        exec_dp(r, f, &a);
    }
    holds(u.cond, *f) != mutant(415)
}

/// `mov rd, #a ; mov rn, #b` with `a = imm & 0xFFFF`, `b = imm >> 16` (no flags).
#[inline(always)]
pub(super) fn mov_pair(r: &mut [u32; 16], u: &Uop) {
    debug_assert!(u.rd < 15 && u.rn < 15, "I2: MOV_PAIR never names r15");
    let (lo, hi) = (u.imm & 0xFFFF, u.imm >> 16);
    let (first, second) = if mutant(417) { (hi, lo) } else { (lo, hi) };
    r[u.rd as usize] = first;
    r[u.rn as usize] = second;
}

/// `ldr rd, [rn, #a] ; ldr rm, [rn, #b]` with `a = imm & 0xFFFF`, `b = imm >> 16`, `rs` the flags
/// of both and no write-back.
#[inline(always)]
pub(super) fn ldr_pair<B: SbBus>(r: &mut [u32; 16], bus: &mut B, u: &Uop) {
    let mut a = Uop::new(k::LDR_IMM, u.pc);
    (a.rd, a.rn, a.rs, a.imm) = (u.rd, u.rn, u.rs, u.imm & 0xFFFF);
    let mut b = Uop::new(k::LDR_IMM, u.pc.wrapping_add(4));
    (b.rd, b.rn, b.rs, b.imm) = (u.rm, u.rn, u.rs, u.imm >> 16);
    if mutant(418) {
        b.pc = u.pc;
    }
    if mutant(419) {
        b.rd = u.rd;
    }
    exec_mem(r, bus, &a);
    exec_mem(r, bus, &b);
}

#[cfg(test)]
pub(in crate::machine::sb) mod tests {
    use super::*;
    use crate::machine::sb::exec_mem::{F_P, F_U};
    use crate::machine::sb::testkit::oracle::oracle_trace_on;
    use crate::machine::sb::testkit::{random_cpu, CpuSnap, DiffRun, LogBus, Need, Rng};
    use crate::machine::sb::trace::testexec::suite;
    use crate::machine::sb::uop::fused::{expand, fuse_pair};
    use crate::machine::sb::uop::{pack_flags, unpack_flags};

    const PC: u32 = 0xC000_0000;
    const DATA: u32 = 0x9000_0000;

    /// `ldr rd, [rn, #off]` (pre-indexed, up, no write-back) of the lowerers.
    fn ldr(rd: u8, rn: u8, off: u32, pc: u32) -> Uop {
        let mut u = Uop::new(k::LDR_IMM, pc);
        (u.rd, u.rn, u.rs, u.imm) = (rd, rn, F_U | F_P, off);
        u
    }

    fn cmp(kind: u8, rn: u8, rm: u8, imm: u32, sh: u8, pc: u32) -> Uop {
        let mut u = Uop::new(kind, pc);
        (u.rn, u.rm, u.imm, u.sh) = (rn, rm, imm, sh);
        u
    }

    fn guard(cond: u8, exit: u8, pc: u32) -> Uop {
        let mut g = Uop::new(k::GUARD, pc);
        (g.cond, g.rs) = (cond, exit);
        g
    }

    /// A random pair of adjacent uops (kind chosen by `i % 5`) that `fuse_pair` accepts most of
    /// the time: a compare or test and its guard, two moves, or two loads off one base.
    pub(in crate::machine::sb) fn random_pair(rng: &mut Rng, i: u32, pc: u32) -> (Uop, Uop) {
        let (rn, rm) = (rng.below(15) as u8, rng.below(15) as u8);
        let cond = rng.below(14) as u8;
        let a = match i % 5 {
            0 => cmp(k::CMP_IMM, rn, 0, rng.biased(), 0, pc),
            1 => cmp(k::CMP_REG, rn, rm, 0, 0, pc),
            2 => cmp(k::TST_IMM, rn, 0, rng.biased(), rng.below(2) as u8, pc),
            3 => {
                let mut m = Uop::new(k::MOV_IMM, pc);
                (m.rd, m.imm) = (rn, rng.below(0x1_0000));
                m
            }
            _ => ldr(rn, rm, rng.below(0x400), pc),
        };
        let b = match i % 5 {
            0..=2 => guard(cond, 3, pc + 4),
            3 => {
                let mut m = Uop::new(k::MOV_IMM, pc + 4);
                (m.rd, m.imm) = (rm, rng.below(0x1_0000));
                m
            }
            _ => ldr(rng.below(15) as u8, rm, rng.below(0x400), pc + 4),
        };
        (a, b)
    }

    /// Run `uops` unfused with the arms of the other families, then report the nibble and
    /// whether a `GUARD` fired (a guard ends the sequence).
    fn run_unfused(r: &mut [u32; 16], f: &mut u8, bus: &mut LogBus, uops: &[Uop]) -> bool {
        for u in uops {
            match u.kind {
                k::GUARD => {
                    if holds(u.cond, *f) {
                        return true;
                    }
                }
                kind if k::family(kind) == k::Family::Dp => exec_dp(r, f, u),
                _ => exec_mem(r, bus, u),
            }
        }
        false
    }

    /// The two words a fused uop stands for, as the interpreter runs them (`imm12` is the rotated
    /// immediate field of the compare, `rn` and `rm` its registers).
    fn words(cond: u32, kind: u8, rn: u32, rm: u32, imm12: u32) -> [u32; 2] {
        let bcc = cond << 28 | 0x0A00_0000 | 0x20; // b +0x88 under `cond`
        match kind {
            k::CMP_GUARD_I => [0xE350_0000 | rn << 16 | imm12, bcc],
            k::CMP_GUARD_R => [0xE150_0000 | rn << 16 | rm, bcc],
            _ => [0xE310_0000 | rn << 16 | imm12, bcc],
        }
    }

    fn bus_with(words: &[u32]) -> LogBus {
        let mut bus = LogBus::new(9);
        bus.load_words(PC, words);
        bus
    }

    suite! {
        #[test]
        fn the_fused_arms_equal_their_expansion_on_random_states() {
            let mut rng = Rng::new(0xF05E);
            let (mut guards, mut passes, mut writes) = (0, 0, 0);
            for i in 0..6000u32 {
                let cpu = random_cpu(&mut rng, PC);
                let f0 = rng.below(16) as u8;
                let (a, b) = random_pair(&mut rng, i, PC);
                let Some(fused) = fuse_pair(&a, &b) else {
                    continue; // a pairing the packing refuses (its own tests live in uop/fused.rs)
                };
                let [ea, eb] = expand(&fused).expect("a fused uop expands");
                assert_eq!((ea.kind, eb.kind), (a.kind, b.kind), "{fused:?}");

                // the executor's own arms: unfused sequence against the fused arm
                let (mut want_r, mut want_f, mut want_bus) = (cpu.r, f0, LogBus::new(7));
                let fired_want = run_unfused(&mut want_r, &mut want_f, &mut want_bus, &[ea, eb]);
                let (mut got_r, mut got_f, mut got_bus) = (cpu.r, f0, LogBus::new(7));
                let fired_got = match fused.kind {
                    k::MOV_PAIR => {
                        mov_pair(&mut got_r, &fused);
                        false
                    }
                    k::LDR_PAIR => {
                        ldr_pair(&mut got_r, &mut got_bus, &fused);
                        false
                    }
                    _ => cmp_guard(&mut got_r, &mut got_f, &fused),
                };
                assert_eq!((got_r, got_f, fired_got), (want_r, want_f, fired_want), "{fused:?}");
                assert_eq!(got_bus.log, want_bus.log, "{fused:?}: accesses and their pcs");
                guards += usize::from(fired_got);
                passes += usize::from(!fired_got && fused.kind < k::MOV_PAIR);
                writes += usize::from(got_r != cpu.r);
            }
            assert!(guards > 300 && passes > 300 && writes > 1000, "{guards} {passes} {writes}");
        }

        #[test]
        fn the_fused_guards_equal_the_interpreter_on_the_two_instructions() {
            let mut rng = Rng::new(0xC0DE);
            let mut run = DiffRun::default();
            let (mut taken, mut fell) = (0, 0);
            for i in 0..4000u32 {
                let mut cpu = random_cpu(&mut rng, PC);
                let (rn, rm, cond) = (rng.below(15), rng.below(15), rng.below(14));
                let (rot, imm8) = (rng.below(16), rng.below(256));
                let imm = crate::cpu::alu::arm_imm(imm8, rot);
                let junk = rng.biased();
                cpu.r[rn as usize] = rng.pick(&[imm, imm.wrapping_add(1), imm.wrapping_sub(1), junk]);
                cpu.r[rm as usize] = if rng.flag() { cpu.r[rn as usize] } else { rng.biased() };
                let kind = [k::CMP_GUARD_I, k::CMP_GUARD_R, k::TST_GUARD_I][(i % 3) as usize];
                let w = words(cond, kind, rn, rm, rot << 8 | imm8);
                let want = oracle_trace_on(&cpu, bus_with(&w), 2);
                run.note(&cpu, &want);
                let mut u = Uop::new(kind, PC);
                (u.rn, u.rm, u.cond, u.rs, u.n) = (rn as u8, rm as u8, cond as u8, 1, 2);
                u.imm = if kind == k::CMP_GUARD_R { 0 } else { imm };
                u.sh = u8::from(kind == k::TST_GUARD_I && rot == 0);
                let (mut got, mut f) = (cpu.clone(), pack_flags(&cpu));
                let leaves = cmp_guard(&mut got.r, &mut f, &u);
                unpack_flags(&mut got, f);
                let (mut a, mut b) = (CpuSnap::of(&want.cpu, None), CpuSnap::of(&got, None));
                // the interpreter ran the branch too: the pc and the count are the loop's business
                (a.r[15], a.insns, b.r[15]) = (0, 0, 0);
                assert_eq!(a.diff(&b), None, "{kind} {w:08x?} on {:?}", CpuSnap::of(&cpu, None));
                let jumped = want.cpu.r[15] != PC + 8;
                assert_eq!(leaves, jumped, "{kind} {w:08x?}: the guard is the branch condition");
                taken += usize::from(jumped);
                fell += usize::from(!jumped);
            }
            assert!(taken > 600 && fell > 600, "{taken} {fell}");
            run.assert_exercised(
                "fused guards",
                &Need {
                    cases: 4000,
                    retired: 8000,
                    flag_changes: 2000,
                    jumps: 600,
                    max_idle_pct: 20,
                    ..Need::default()
                },
            );
        }

        #[test]
        fn a_load_pair_reads_in_order_at_the_pc_of_each_half_and_the_second_sees_the_first() {
            let mut bus = LogBus::new(0);
            bus.poke32(DATA, 0x1111_1111);
            bus.poke32(DATA + 4, 0x2222_2222);
            let mut r = [0u32; 16];
            r[1] = DATA;
            let fused = fuse_pair(&ldr(2, 1, 0, PC), &ldr(3, 1, 4, PC + 4)).unwrap();
            assert_eq!(fused.kind, k::LDR_PAIR);
            ldr_pair(&mut r, &mut bus, &fused);
            assert_eq!((r[2], r[3]), (0x1111_1111, 0x2222_2222));
            assert_eq!(
                bus.log.iter().map(|e| (e.1, e.3)).collect::<Vec<_>>(),
                [(DATA, PC), (DATA + 4, PC + 4)],
            );
            // the first load may clobber the shared base: the second load then uses the new value
            let fused = fuse_pair(&ldr(1, 1, 0, PC), &ldr(3, 1, 4, PC + 4)).unwrap();
            let mut r = [0u32; 16];
            r[1] = DATA;
            bus.poke32(DATA, DATA + 0x100);
            bus.poke32(DATA + 0x104, 0xABCD_0123);
            ldr_pair(&mut r, &mut bus, &fused);
            assert_eq!((r[1], r[3]), (DATA + 0x100, 0xABCD_0123));
        }

        #[test]
        fn a_move_pair_writes_the_first_destination_first_so_a_shared_one_keeps_the_second() {
            let mut r = [0u32; 16];
            let mut a = Uop::new(k::MOV_IMM, PC);
            (a.rd, a.imm) = (4, 0x1234);
            let mut b = Uop::new(k::MOV_IMM, PC + 4);
            (b.rd, b.imm) = (5, 0xFFFF);
            mov_pair(&mut r, &fuse_pair(&a, &b).unwrap());
            assert_eq!((r[4], r[5]), (0x1234, 0xFFFF));
            b.rd = 4;
            mov_pair(&mut r, &fuse_pair(&a, &b).unwrap());
            assert_eq!(r[4], 0xFFFF);
        }
    }
}
