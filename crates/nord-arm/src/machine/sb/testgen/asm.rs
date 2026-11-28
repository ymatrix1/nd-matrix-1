//! OWNER: W1-VGEN. A tiny ARM and Thumb assembler for the generators: the encodings the
//! structured programs and the per-family word generators build on. Every encoder is checked
//! against the interpreter in the tests below (what it assembles does what its name says).
//!
//! Addresses are byte addresses; branch and literal offsets are computed from the address `at`
//! of the instruction being encoded, so a generator never counts words by hand.

/// Condition field of "always".
pub(crate) const AL: u32 = 14;
/// `udf`-style word: the media-space hole the interpreter reports as `Fault::Undefined`.
pub(crate) const UNDEFINED: u32 = 0xE7F0_00F0;
/// `swi #0`.
pub(crate) const SWI: u32 = 0xEF00_0000;
/// `mcr p15, 0, r0, c7, c0, 4`: wait for interrupt (`Fault::Wfi`).
pub(crate) const WFI: u32 = 0xEE07_0F90;
/// `mov r0, r0`.
pub(crate) const NOP: u32 = 0xE1A0_0000;

/// Data processing, rotated immediate: `op{s}{cond} rd, rn, #imm8 ror (2 * rot)`.
pub(crate) fn dp_imm(cond: u32, op: u32, s: bool, rn: u32, rd: u32, rot: u32, imm8: u32) -> u32 {
    cond << 28 | 1 << 25 | op << 21 | (s as u32) << 20 | rn << 16 | rd << 12 | rot << 8 | imm8
}

/// Data processing, register shifted by an immediate (`ty` 0 lsl, 1 lsr, 2 asr, 3 ror/rrx);
/// always executed (use `parts::with_cond` for a conditional word).
pub(crate) fn dp_shift_imm(
    op: u32,
    s: bool,
    rn: u32,
    rd: u32,
    rm: u32,
    ty: u32,
    amount: u32,
) -> u32 {
    AL << 28 | op << 21 | (s as u32) << 20 | rn << 16 | rd << 12 | amount << 7 | ty << 5 | rm
}

/// Data processing, register shifted by a register.
pub(crate) fn dp_shift_reg(op: u32, s: bool, rn: u32, rd: u32, rm: u32, ty: u32, rs: u32) -> u32 {
    AL << 28 | op << 21 | (s as u32) << 20 | rn << 16 | rd << 12 | rs << 8 | ty << 5 | 1 << 4 | rm
}

/// `mov rd, #imm8`.
pub(crate) fn mov_imm(rd: u32, imm8: u32) -> u32 {
    dp_imm(AL, 13, false, 0, rd, 0, imm8)
}

/// `add rd, rn, #imm8` (`s`: set the flags).
pub(crate) fn add_imm(s: bool, rd: u32, rn: u32, imm8: u32) -> u32 {
    dp_imm(AL, 4, s, rn, rd, 0, imm8)
}

/// `sub rd, rn, #imm8` (`s`: set the flags).
pub(crate) fn sub_imm(s: bool, rd: u32, rn: u32, imm8: u32) -> u32 {
    dp_imm(AL, 2, s, rn, rd, 0, imm8)
}

/// `cmp rn, #imm8`.
pub(crate) fn cmp_imm(rn: u32, imm8: u32) -> u32 {
    dp_imm(AL, 10, true, rn, 0, 0, imm8)
}

/// `b`/`bl` at `at` to `target`: the offset is `(target - (at + 8)) / 4` in 24 bits.
pub(crate) fn branch(cond: u32, link: bool, at: u32, target: u32) -> u32 {
    let off = (target.wrapping_sub(at.wrapping_add(8)) as i32 >> 2) as u32 & 0x00FF_FFFF;
    cond << 28 | 0x0A00_0000 | (link as u32) << 24 | off
}

/// `bx rm`.
pub(crate) fn bx(cond: u32, rm: u32) -> u32 {
    cond << 28 | 0x012F_FF10 | rm
}

/// Word or byte transfer `rd, [rn, #offset]` (`pre`: pre-indexed, else post-indexed; `wb`: write
/// back for the pre-indexed form). `offset` is signed and at most 4095 in magnitude.
pub(crate) fn ldst_imm(
    load: bool,
    byte: bool,
    pre: bool,
    wb: bool,
    rn: u32,
    rd: u32,
    offset: i32,
) -> u32 {
    assert!(
        offset.unsigned_abs() <= 0xFFF,
        "offset {offset} does not fit 12 bits"
    );
    AL << 28
        | 0x0400_0000
        | (pre as u32) << 24
        | ((offset >= 0) as u32) << 23
        | (byte as u32) << 22
        | (wb as u32) << 21
        | (load as u32) << 20
        | rn << 16
        | rd << 12
        | offset.unsigned_abs()
}

/// `ldr rd, [rn, #offset]`.
pub(crate) fn ldr(rd: u32, rn: u32, offset: i32) -> u32 {
    ldst_imm(true, false, true, false, rn, rd, offset)
}

/// `str rd, [rn, #offset]`.
pub(crate) fn str(rd: u32, rn: u32, offset: i32) -> u32 {
    ldst_imm(false, false, true, false, rn, rd, offset)
}

/// The pc-relative form at `at` that addresses `target` (`ldr rd, [pc, #x]` when `load`, else
/// `str rd, [pc, #x]`): `pc` reads as `at + 8`, so the offset is `target - at - 8`.
pub(crate) fn mem_pc(load: bool, rd: u32, at: u32, target: u32) -> u32 {
    let off = target.wrapping_sub(at.wrapping_add(8)) as i32;
    ldst_imm(load, false, true, false, 15, rd, off)
}

/// Block transfer. `p`: pre, `u`: up, `w`: write back; `list` is the register bit mask.
pub(crate) fn block(cond: u32, load: bool, p: bool, u: bool, w: bool, rn: u32, list: u32) -> u32 {
    cond << 28
        | 0x0800_0000
        | (p as u32) << 24
        | (u as u32) << 23
        | (w as u32) << 21
        | (load as u32) << 20
        | rn << 16
        | list
}

/// `stmdb sp!, {list}`.
pub(crate) fn push(list: u32) -> u32 {
    block(AL, false, true, false, true, 13, list)
}

/// `ldmia sp!, {list}`.
pub(crate) fn pop(list: u32) -> u32 {
    block(AL, true, false, true, true, 13, list)
}

/// `mrs rd, cpsr` (`spsr`: the saved status register instead).
pub(crate) fn mrs(rd: u32, spsr: bool) -> u32 {
    AL << 28 | 0x010F_0000 | (spsr as u32) << 22 | rd << 12
}

/// `msr cpsr_<mask>, #imm8 ror (2 * rot)` (`spsr`: the saved one). `mask` bit 3 selects the flag
/// byte, bit 0 the control byte.
pub(crate) fn msr_imm(spsr: bool, mask: u32, rot: u32, imm8: u32) -> u32 {
    AL << 28 | 0x0320_F000 | (spsr as u32) << 22 | mask << 16 | rot << 8 | imm8
}

/// `msr cpsr_<mask>, rm`.
pub(crate) fn msr_reg(spsr: bool, mask: u32, rm: u32) -> u32 {
    AL << 28 | 0x0120_F000 | (spsr as u32) << 22 | mask << 16 | rm
}

/// Thumb encodings (16 bit halfwords) of the small subset the generated Thumb blocks use.
pub(crate) mod thumb {
    /// `bx lr`.
    pub const BX_LR: u16 = 0x4770;

    /// Pack halfwords into little-endian words, two per word (an odd last one is padded with 0).
    pub fn pack(halfwords: &[u16]) -> Vec<u32> {
        halfwords
            .chunks(2)
            .map(|c| c[0] as u32 | (c.get(1).copied().unwrap_or(0) as u32) << 16)
            .collect()
    }

    /// `movs rd, #imm8` (`rd` 0..=7).
    pub fn mov_imm(rd: u32, imm8: u32) -> u16 {
        (0x2000 | rd << 8 | imm8) as u16
    }

    /// `cmp rd, #imm8`.
    pub fn cmp_imm(rd: u32, imm8: u32) -> u16 {
        (0x2800 | rd << 8 | imm8) as u16
    }

    /// `adds rd, #imm8`.
    pub fn add_imm(rd: u32, imm8: u32) -> u16 {
        (0x3000 | rd << 8 | imm8) as u16
    }

    /// `subs rd, #imm8`.
    pub fn sub_imm(rd: u32, imm8: u32) -> u16 {
        (0x3800 | rd << 8 | imm8) as u16
    }

    /// `lsls rd, rm, #amount` (`lsrs` when `right`).
    pub fn shift(right: bool, rd: u32, rm: u32, amount: u32) -> u16 {
        ((right as u32) << 11 | amount << 6 | rm << 3 | rd) as u16
    }

    /// `adds rd, rn, rm`.
    pub fn add_reg(rd: u32, rn: u32, rm: u32) -> u16 {
        (0x1800 | rm << 6 | rn << 3 | rd) as u16
    }

    /// `str rd, [sp, #4 * words]` (`ldr` when `load`).
    pub fn sp_rel(load: bool, rd: u32, words: u32) -> u16 {
        (0x9000 | (load as u32) << 11 | rd << 8 | words) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::predecode::{decode_arm, op};
    use crate::cpu::Fault;
    use crate::machine::sb::testkit::oracle::oracle_trace_on;
    use crate::machine::sb::testkit::{oracle_trace, random_cpu, LogBus, Rng, TraceRun};

    /// Run `words` from a CPU at 0x1000 with `r1 = 0x2000` and flags clear.
    fn run(words: &[u32], steps: u64) -> TraceRun {
        let mut cpu = random_cpu(&mut Rng::new(1), 0x1000);
        cpu.r[1] = 0x2000;
        cpu.set_cpsr(0x13);
        oracle_trace(&cpu, words, 9, steps)
    }

    #[test]
    fn data_processing_encoders_do_what_they_say() {
        let t = run(
            &[
                mov_imm(2, 0x40),
                add_imm(false, 3, 2, 2),
                sub_imm(true, 4, 2, 0x40),
                cmp_imm(3, 0x42),
                dp_imm(AL, 13, false, 0, 5, 4, 0x3F), // mov r5,#0x3F ror 8
                dp_shift_imm(13, false, 0, 6, 2, 0, 4), // mov r6,r2,lsl #4
                dp_shift_reg(13, false, 0, 7, 2, 1, 3), // mov r7,r2,lsr r3
            ],
            7,
        );
        assert_eq!(t.fault, None);
        // r7 = r2 lsr r3 with r3 = 0x42: an amount of 66 shifts everything out
        assert_eq!(&t.cpu.r[2..8], &[0x40, 0x42, 0, 0x3F00_0000, 0x400, 0]);
        assert_eq!(t.cpu.nzcv(), 0b0110, "cmp r3,#0x42 sets Z and C");
    }

    #[test]
    fn branch_and_bx_reach_their_targets() {
        let t = run(&[branch(AL, true, 0x1000, 0x1040)], 1);
        assert_eq!((t.cpu.r[15], t.cpu.r[14]), (0x1040, 0x1004));
        let back = run(&[NOP, branch(AL, false, 0x1004, 0x1000)], 2);
        assert_eq!(back.cpu.r[15], 0x1000, "backward branches");
        let mut cpu = random_cpu(&mut Rng::new(2), 0x1000);
        cpu.r[7] = 0x3000;
        let t = oracle_trace(&cpu, &[bx(AL, 7)], 1, 1);
        assert_eq!((t.cpu.r[15], t.cpu.thumb), (0x3000, false));
        assert_eq!(decode_arm(branch(AL, true, 0, 8)).op, op::BL);
        assert_eq!(decode_arm(bx(AL, 3)).op, op::BX);
    }

    #[test]
    fn memory_encoders_address_what_they_say() {
        let t = run(
            &[
                mov_imm(0, 77),
                str(0, 1, 8),
                ldr(2, 1, 8),
                mem_pc(true, 3, 0x100C, 0x1000),
                mem_pc(false, 0, 0x1010, 0x1800),
            ],
            5,
        );
        assert_eq!(t.fault, None);
        assert_eq!(t.cpu.r[2], 77);
        assert_eq!(
            t.cpu.r[3],
            t.bus.peek32(0x1000),
            "pc-relative load of the first word"
        );
        assert_eq!(
            t.bus.log.iter().map(|e| (e.0, e.1)).collect::<Vec<_>>(),
            vec![(5, 0x2008), (2, 0x2008), (2, 0x1000), (5, 0x1800)]
        );
        let post = run(&[ldst_imm(true, true, false, false, 1, 4, 3)], 1);
        assert_eq!(
            (post.cpu.r[1], post.bus.log[0].1),
            (0x2003, 0x2000),
            "post-indexed ldrb"
        );
        let neg = run(&[ldst_imm(false, false, true, true, 1, 0, -16)], 1);
        assert_eq!(
            (neg.cpu.r[1], neg.bus.log[0].1),
            (0x1FF0, 0x1FF0),
            "pre-indexed with write-back"
        );
    }

    #[test]
    fn block_and_status_encoders_do_what_they_say() {
        let mut cpu = random_cpu(&mut Rng::new(3), 0x1000);
        (cpu.r[0], cpu.r[1], cpu.r[13]) = (11, 22, 0x4000);
        cpu.set_cpsr(0x13);
        let t = oracle_trace(
            &cpu,
            &[push(0b11), mov_imm(0, 0), mov_imm(1, 0), pop(0b11)],
            3,
            4,
        );
        assert_eq!((t.cpu.r[0], t.cpu.r[1], t.cpu.r[13]), (11, 22, 0x4000));
        assert_eq!(t.bus.log[0], (5, 0x3FF8, 11, 0x1000));
        let t = oracle_trace(&cpu, &[msr_imm(false, 8, 4, 0xA0), mrs(2, false)], 3, 2); // flags = 0xA0 ror 8 = 0xA000_0000
        assert_eq!(t.cpu.nzcv(), 0b1010);
        assert_eq!(t.cpu.r[2] >> 28, 0b1010);
        // spsr round trip from a clear old value (the interpreter's register form of
        // `msr spsr_<mask>` writes the whole register: the old bits are kept out of the check)
        cpu.spsr = 0;
        let t = oracle_trace(&cpu, &[msr_reg(true, 9, 0), mrs(3, true)], 3, 2);
        assert_eq!((t.cpu.spsr, t.cpu.r[3]), (11, 11));
        assert_eq!(decode_arm(msr_imm(false, 1, 0, 0x13)).op, op::SLOW);
        assert_eq!(decode_arm(push(1)).op, op::LDM);
    }

    #[test]
    fn the_fault_words_fault_the_way_their_names_say() {
        for (w, want) in [
            (
                UNDEFINED,
                Fault::Undefined {
                    pc: 0x1000,
                    insn: UNDEFINED,
                },
            ),
            (SWI | 7, Fault::Swi { pc: 0x1000, imm: 7 }),
            (WFI, Fault::Wfi),
        ] {
            assert_eq!(run(&[w], 1).fault, Some(want));
        }
        assert_eq!(run(&[NOP], 1).fault, None);
    }

    #[test]
    fn thumb_encodings_run_on_the_interpreter() {
        let halfwords = [
            thumb::mov_imm(0, 200),
            thumb::add_imm(0, 100),
            thumb::sub_imm(0, 4),
            thumb::mov_imm(1, 3),
            thumb::shift(false, 2, 1, 4),
            thumb::shift(true, 3, 2, 2),
            thumb::add_reg(4, 0, 1),
            thumb::sp_rel(false, 4, 2),
            thumb::sp_rel(true, 5, 2),
            thumb::cmp_imm(5, 0),
            thumb::BX_LR,
        ];
        let mut cpu = random_cpu(&mut Rng::new(4), 0x1000);
        (cpu.thumb, cpu.r[13], cpu.r[14]) = (true, 0x5000, 0x8000);
        let mut bus = LogBus::new(3);
        bus.load_words(0x1000, &thumb::pack(&halfwords));
        let t = oracle_trace_on(&cpu, bus, 11);
        assert_eq!(t.fault, None);
        assert_eq!(&t.cpu.r[..6], &[296, 3, 48, 12, 299, 299]);
        assert_eq!(
            t.bus.log[0],
            (5, 0x5008, 299, 0x1000 + 2 * 7),
            "str [sp,#8]"
        );
        assert_eq!(t.bus.log.len(), 2, "Thumb fetches are not data accesses");
        assert_eq!(
            (t.cpu.r[15], t.cpu.thumb),
            (0x8000, false),
            "bx lr leaves Thumb"
        );
    }
}
