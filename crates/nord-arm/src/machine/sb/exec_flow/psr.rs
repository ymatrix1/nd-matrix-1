//! OWNER: W2-XCORE. The `MRS` and `MSR_C` arms: read the CPSR or SPSR into a register, and write
//! the control field of the CPSR.
//!
//! Semantics are those of `Cpu::arm_misc` / `Cpu::msr` (`cpu/arm.rs`):
//!
//! * `MRS` (`rs` bit 0 clear): `r[rd] = cpsr`, with the N, Z, C and V bits taken from the flag
//!   nibble `f` (I4: inside the executor the nibble is authoritative, `cpu.n/z/c/v` may be
//!   stale), every other bit from the `Cpu`; with `rs` bit 0 set: `r[rd] = spsr`.
//! * `MSR_C` writes the bits of [`CTRL_MASK`] of the CPSR from `imm` (`rs` bit 0 set: the
//!   immediate form, `imm` is already rotated) or from `r[rn]` (register form): the I and F bits
//!   and the five mode bits. The T bit is never written (`Cpu::msr` restores it), the flags and
//!   `q` are untouched, so the nibble is not an input or an output. A write that changes the mode
//!   counts one `mode_changes` (the registers are not banked, the count is how a test sees it).
//!
//! A conditional `msr`/`mrs` is wrapped in a `PRED` by the builder; the arms are unconditional.
//! `rd` and `rn` are `0..=14` (I2).
//!
//! VERIFIED: equal to the interpreter on 16 flag nibbles x random registers, CPSR and SPSR reads,
//! register and immediate writes, mode changes and no changes, by `the_mrs_and_msr_arms_equal_the_
//! interpreter_on_random_states` (a few thousand cases, zero differences).

use super::mutant;
use crate::cpu::Cpu;
use crate::machine::sb::uop::Uop;

/// VERIFIED: the bits an `msr cpsr_c, ..` writes: I (7), F (6) and the mode (4..0). It is the
/// `0x0000_00DF` of `Cpu::msr` for the field mask bit 16 (`c`): bit 5 (T) is not part of it, as
/// the interpreter never writes the T bit from an `msr` either (`keep_t`). Checked by `the_t_bit_
/// is_not_written_and_the_flags_are_untouched`.
pub(super) const CTRL_MASK: u32 = 0x0000_00DF;

/// `r[rd] = cpsr` (flags from `f`) or `r[rd] = spsr`.
#[inline(always)]
pub(super) fn mrs(cpu: &mut Cpu, u: &Uop, f: u8) {
    debug_assert!(u.rd < 15, "I2: MRS never names r15");
    let flags = if mutant(409) { cpu.nzcv() as u8 } else { f };
    let v = if u.rs & 1 != 0 && !mutant(410) {
        cpu.spsr
    } else {
        cpu.cpsr() & 0x0FFF_FFFF | u32::from(flags & 15) << 28
    };
    cpu.r[u.rd as usize] = v;
}

/// `cpsr_c = value & CTRL_MASK` with `value` the immediate or `r[rn]`.
#[inline(always)]
pub(super) fn msr_c(cpu: &mut Cpu, u: &Uop) {
    debug_assert!(u.rn < 15, "I2: MSR never names r15");
    let value = if u.rs & 1 != 0 && !mutant(413) {
        u.imm
    } else {
        cpu.r[u.rn as usize]
    };
    let mask = match (mutant(414), mutant(411)) {
        (true, _) => CTRL_MASK | 0xF000_0000,
        (_, true) => CTRL_MASK | 0x20,
        _ => CTRL_MASK,
    };
    let old_mode = cpu.mode;
    let keep_t = cpu.thumb;
    let mixed = (cpu.cpsr() & !mask) | (value & mask);
    cpu.set_cpsr(mixed);
    if !mutant(411) {
        cpu.thumb = keep_t;
    }
    if cpu.mode != old_mode && !mutant(412) {
        cpu.mode_changes += 1;
    }
}

#[cfg(test)]
pub(in crate::machine::sb) mod tests {
    use super::*;
    use crate::machine::sb::testkit::{
        oracle_trace, random_cpu_flags, CpuSnap, DiffRun, Need, Rng,
    };
    use crate::machine::sb::trace::testexec::suite;
    use crate::machine::sb::uop::{k, pack_flags};

    const PC: u32 = 0xC000_0000;

    /// The uop a lowerer is specified to emit for an `mrs`/`msr cpsr_c` word.
    fn uop_of(word: u32) -> Uop {
        let mut u = Uop::new(0, PC);
        if word & 0x0FBF_0FFF == 0x010F_0000 {
            // mrs rd, cpsr|spsr
            u.kind = k::MRS;
            u.rd = (word >> 12 & 15) as u8;
            u.rs = (word >> 22 & 1) as u8;
        } else if word & 0x0FFF_FFF0 == 0x0121_F000 {
            u.kind = k::MSR_C;
            u.rn = (word & 15) as u8;
        } else {
            assert_eq!(word & 0x0FFF_F000, 0x0321_F000, "{word:#010x}");
            u.kind = k::MSR_C;
            u.rs = 1;
            u.imm = crate::cpu::alu::arm_imm(word & 0xFF, word >> 8 & 15);
        }
        u
    }

    /// Run the uop of `word` on `cpu` through the arms, and the word through the interpreter,
    /// and compare the state (the pc and the instruction count belong to the executor loop).
    /// Returns the interpreter's final state.
    fn check(cpu: &Cpu, word: u32, run: &mut DiffRun) -> Cpu {
        let want = oracle_trace(cpu, &[word], 1, 1);
        run.note(cpu, &want);
        let (mut got, u) = (cpu.clone(), uop_of(word));
        let f = pack_flags(&got);
        match u.kind {
            k::MRS => mrs(&mut got, &u, f),
            _ => msr_c(&mut got, &u),
        }
        let (mut a, mut b) = (CpuSnap::of(&want.cpu, None), CpuSnap::of(&got, None));
        (a.r[15], a.insns) = (0, 0);
        (b.r[15], b.insns) = (0, 0);
        assert_eq!(
            a.diff(&b),
            None,
            "{word:#010x} on {:?}",
            CpuSnap::of(cpu, None)
        );
        want.cpu
    }

    suite! {
        #[test]
        fn the_mrs_and_msr_arms_equal_the_interpreter_on_random_states() {
            let mut rng = Rng::new(0x9512);
            let mut run = DiffRun::default();
            let (mut mode_changes, mut irq_changes, mut spsr_reads, mut cpsr_reads) = (0, 0, 0, 0);
            for i in 0..4000u32 {
                let mut cpu = random_cpu_flags(&mut rng, PC, (i % 16) as u8);
                cpu.q = rng.flag();
                cpu.irq_off = rng.flag();
                cpu.fiq_off = rng.flag();
                cpu.mode = rng.pick(&[0x10, 0x11, 0x12, 0x13, 0x17, 0x1B, 0x1F]);
                let (rd, rm) = (rng.below(15), rng.below(15));
                if rng.flag() {
                    // a value that looks like a status word: mode bits and I/F bits, junk above
                    cpu.r[rm as usize] = rng.next() & 0xF000_0020 | rng.pick(&[0x10, 0x13, 0x1F, 0xD3]);
                }
                let word = match rng.below(4) {
                    0 => 0xE10F_0000 | rd << 12,
                    1 => 0xE14F_0000 | rd << 12,
                    2 => 0xE121_F000 | rm,
                    _ => 0xE321_F000 | rng.below(16) << 8 | rng.below(256),
                };
                let after = check(&cpu, word, &mut run);
                mode_changes += u32::from(after.mode != cpu.mode);
                irq_changes += u32::from(after.irq_off != cpu.irq_off || after.fiq_off != cpu.fiq_off);
                match word >> 21 & 0xF {
                    8 => cpsr_reads += 1,
                    10 => spsr_reads += 1,
                    _ => {}
                }
            }
            run.assert_exercised(
                "mrs/msr",
                &Need {
                    cases: 4000,
                    retired: 4000,
                    reg_writes: 1500,
                    max_idle_pct: 55,
                    ..Need::default()
                },
            );
            assert!(mode_changes > 400 && irq_changes > 400, "{mode_changes} {irq_changes}");
            assert!(cpsr_reads > 800 && spsr_reads > 800, "{cpsr_reads} {spsr_reads}");
        }

        #[test]
        fn the_t_bit_is_not_written_and_the_flags_are_untouched() {
            let mut cpu = Cpu::new();
            cpu.n = true;
            cpu.q = true;
            let mut u = Uop::new(k::MSR_C, PC);
            u.rs = 1;
            u.imm = 0xF000_00FF; // every bit set: flags, q, T, I, F, mode
            msr_c(&mut cpu, &u);
            assert_eq!((cpu.irq_off, cpu.fiq_off, cpu.mode, cpu.thumb), (true, true, 0x1F, false));
            assert!(cpu.n && !cpu.z && !cpu.c && !cpu.v && cpu.q, "flags and q stay");
            assert_eq!(cpu.mode_changes, 1, "SVC -> SYS");
            msr_c(&mut cpu, &u);
            assert_eq!(cpu.mode_changes, 1, "no change, no count");
            assert_eq!(CTRL_MASK, 0xDF);
        }

        #[test]
        fn mrs_takes_the_flags_from_the_nibble_and_the_rest_from_the_cpu() {
            let mut cpu = Cpu::new();
            cpu.q = true;
            cpu.spsr = 0xDEAD_BEEF;
            let mut u = Uop::new(k::MRS, PC);
            u.rd = 3;
            mrs(&mut cpu, &u, 0b1010); // N and C; the cpu's own flags are all clear
            assert_eq!(cpu.r[3], 0xA000_0000 | 1 << 27 | 1 << 7 | 1 << 6 | cpu.mode);
            u.rs = 1;
            mrs(&mut cpu, &u, 0b1010);
            assert_eq!(cpu.r[3], 0xDEAD_BEEF);
        }
    }
}
