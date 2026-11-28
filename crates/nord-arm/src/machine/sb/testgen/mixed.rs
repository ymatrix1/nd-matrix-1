//! OWNER: W1-VGEN (moved from the scaffold of `testgen.rs`, behaviour unchanged). The scaffold's
//! unstructured generators: the hand-written `tiny_loop` and the flat random-word mix that
//! mirrors the private `word()` of the `fastrun.rs` test. Their streams are part of the contract
//! (work packages seeded against the skeleton): the structured generator of `program.rs` is a new
//! item, not a change of these.

use super::CODE;
use crate::machine::sb::testkit::Rng;

/// A small hand-written program: `r1 = 5; r0 = 0; loop { bl sub; r1 -= 1; str r0,[r8]; bne }`
/// then `b .`, with `sub { r0 += r1 << 1; r2 = [r8]; r0 += r2; bx lr }`. Needs `r8` to point at
/// writable data; leaves `r0 = 258`, `r1 = 0` and spins at word 6.
pub(crate) fn tiny_loop() -> Vec<u32> {
    vec![
        0xE3A0_1005, // 0  mov r1,#5
        0xE3A0_0000, // 1  mov r0,#0
        0xEB00_0003, // 2  bl sub
        0xE251_1001, // 3  subs r1,r1,#1
        0xE588_0000, // 4  str r0,[r8]
        0x1AFF_FFFB, // 5  bne 2
        0xEAFF_FFFE, // 6  b .
        0xE080_0081, // 7  sub: add r0,r0,r1,lsl #1
        0xE598_2000, // 8  ldr r2,[r8]
        0xE080_0002, // 9  add r0,r0,r2
        0xE12F_FF1E, // 10 bx lr
    ]
}

/// One random word from a mix that stays inside a region of `n` words at [`CODE`]: data
/// processing on r0..r7, loads and stores through r8 (data) and r10 (the code itself:
/// self-modifying code), block transfers on r9, short branches, a few generic and garbage words.
/// `at` is the index of the word (branch offsets are relative to it).
pub(crate) fn random_word(rng: &mut Rng, at: usize, n: usize) -> u32 {
    let cond = if rng.chance(4) { rng.below(14) } else { 14 } << 28;
    let rd = if rng.chance(12) { 15 } else { rng.below(8) }; // sometimes writes pc
    let (rn, rm) = (rng.below(8), rng.below(8));
    cond | match rng.below(28) {
        20 => 0x050F_0004 | rd << 12 | rng.below(2) << 20, // str/ldr rd,[pc,#-4]: the next word
        21 => {
            0x01C0_00B0
                | 10 << 16
                | rd << 12
                | rng.below(2) << 20
                | rng.below(2) << 8
                | rng.below(16)
        } // strh/ldrh [r10,#imm]
        22 => 0x038F_C001,                                 // orr r12,pc,#1 (Thumb target)
        23 => 0x012F_FF1C,                                 // bx r12 (enter Thumb state)
        24 => 0x0890_800F | rng.below(2) << 20 | rng.below(2) << 24 | rng.below(2) << 23 | 9 << 16, // ldm/stm with pc
        25..=27 => rng.next() & 0x0FFF_FFFF,
        0..=8 => {
            let (op, s) = (rng.below(16), rng.below(2));
            let s = if (8..=11).contains(&op) { 1 } else { s };
            let operand = match rng.below(3) {
                0 => 1 << 25 | rng.below(0x1000),
                1 => rm | rng.below(0x100) << 4 & !0x10,
                _ => rm | 1 << 4 | rng.below(8) << 8 | rng.below(4) << 5,
            };
            op << 21 | s << 20 | rn << 16 | rd << 12 | operand
        }
        9..=11 => {
            let (l, b, base) = (
                rng.below(2),
                rng.below(2),
                if rng.chance(5) { 10 } else { 8 },
            );
            0x0580_0000 | b << 22 | l << 20 | base << 16 | rd << 12 | (rng.below(0x40) * 4)
        }
        12 => 0x0890_000F | rng.below(2) << 20 | rng.below(2) << 24 | rng.below(2) << 23 | 9 << 16,
        13..=14 => {
            let target = rng.below(n as u32) as i64;
            let off = (target - at as i64 - 2) as u32 & 0x00FF_FFFF;
            0x0A00_0000 | rng.below(2) << 24 | off
        }
        15 => 0x0000_0090 | rd << 16 | rm << 8 | rn, // mul
        16 => 0x012F_FF10 | 14,                      // bx lr
        17 => 0x0100_0090 | rm | rd << 12 | rn << 16, // swp
        _ => rng.next() & 0x0FFF_FFFF,
    }
}

/// `n` random words (see [`random_word`]).
pub(crate) fn random_program(rng: &mut Rng, n: usize) -> Vec<u32> {
    (0..n).map(|i| random_word(rng, i, n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::predecode::{decode_arm, op};
    use crate::machine::sb::testgen::machine_with;
    use crate::machine::Stop;

    #[test]
    fn the_tiny_loop_computes_what_its_comment_says() {
        let mut m = machine_with(&tiny_loop(), 1, false);
        assert_eq!(m.run(100, 0, None), (100, Stop::Budget));
        assert_eq!((m.cpu.r[0], m.cpu.r[1], m.cpu.r[15]), (258, 0, CODE + 24));
        assert_eq!(m.bus.mem.read32(0x9000_0000), 258);
    }

    #[test]
    fn programs_are_deterministic_per_seed_and_cover_every_construct_class() {
        let a = random_program(&mut Rng::new(3), 2000);
        assert_eq!(a, random_program(&mut Rng::new(3), 2000));
        assert_ne!(a, random_program(&mut Rng::new(4), 2000));
        let mut seen = std::collections::BTreeSet::new();
        for w in &a {
            seen.insert(decode_arm(*w).op.min(op::DP_BASE));
        }
        for class in [
            op::B,
            op::BL,
            op::BX,
            op::LDR,
            op::LDRB,
            op::STR,
            op::STRB,
            op::LDM,
            op::HALF,
            op::MUL,
            op::SLOW,
            op::DP_BASE,
        ] {
            assert!(seen.contains(&class), "class {class} never generated");
        }
    }
}
