//! OWNER: W1-VGEN. The pieces the structured programs (`blocks.rs`, `program.rs`) are assembled
//! from: one straight-line word that keeps the register contract, the six callee shapes and the
//! tail.
//!
//! Register contract of a generated program (so blocks compose and every loop ends):
//!
//! | Registers | Use |
//! |---|---|
//! | `r0..r7` | scratch, random at start, the only registers straight code writes |
//! | `r8`, `r13` | data page and stack; transfers go through them and their write-back moves them by at most 32 bytes |
//! | `r9` | the next page (`POOL`); the tail stores into it |
//! | `r10` | the code base: `str rX, [r10, #4n]` patches word `n` of the program |
//! | `r11`, `r12` | loop counters (outer, inner); `r12` is also the Thumb entry temporary |
//! | `r14` | link register |

use super::asm::{self as a, AL};
use super::program::{FAR_AT, FUNC_AT, POOL, SPIN_AT, TAIL_AT};
use crate::machine::sb::testkit::Rng;

/// Condition codes that are not "always".
pub(super) const CONDS: [u32; 13] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

/// Control byte values that keep SVC mode (`msr cpsr_c, #imm`): interrupt masks only.
const KEEP_MODE: [u32; 4] = [0x13, 0x93, 0xD3, 0x53];

/// Replace the condition field of `w`.
pub(super) fn with_cond(w: u32, cond: u32) -> u32 {
    w & 0x0FFF_FFFF | cond << 28
}

/// Data processing word of any of the three operand forms; compares get `S` and `rd = 0`.
fn data_processing(rng: &mut Rng) -> u32 {
    let (rd, rn, rm, rs) = (rng.below(8), rng.below(8), rng.below(8), rng.below(8));
    let op = rng.below(16);
    let compare = (8..=11).contains(&op);
    let (s, rd) = if compare { (true, 0) } else { (rng.flag(), rd) };
    match rng.below(10) {
        0..=4 => {
            let rot = if rng.chance(4) { rng.below(16) } else { 0 };
            a::dp_imm(AL, op, s, rn, rd, rot, rng.biased() & 0xFF)
        }
        5..=7 => {
            let amount = if rng.flag() {
                rng.pick(&[0, 1, 31])
            } else {
                rng.below(32)
            };
            a::dp_shift_imm(op, s, rn, rd, rm, rng.below(4), amount)
        }
        _ => a::dp_shift_reg(op, s, rn, rd, rm, rng.below(4), rs),
    }
}

/// `mul`, `mla` and the four long multiplies, or `clz`, a saturating add or a DSP multiply.
fn multiply(rng: &mut Rng) -> u32 {
    let (rd, rn, rm, rs) = (rng.below(8), rng.below(8), rng.below(8), rng.below(8));
    if rng.chance(3) {
        return match rng.below(3) {
            0 => 0xE16F_0F10 | rd << 12 | rm,
            1 => 0xE100_0050 | rng.below(4) << 21 | rn << 16 | rd << 12 | rm,
            _ => {
                0xE100_0080
                    | rng.below(4) << 21
                    | rng.below(4) << 5
                    | rd << 16
                    | rn << 12
                    | rs << 8
                    | rm
            }
        };
    }
    let (long, acc, signed, s) = (rng.flag(), rng.flag(), rng.flag(), rng.flag());
    let head = if long {
        0x0080_0090 | (signed as u32) << 22
    } else {
        0x90
    };
    AL << 28 | head | (acc as u32) << 21 | (s as u32) << 20 | rd << 16 | rn << 12 | rs << 8 | rm
}

/// Word, byte, halfword and doubleword transfers and `swp` through `r8` (data) or `r13` (stack).
fn transfer(rng: &mut Rng) -> u32 {
    let (rd, rm) = (rng.below(8), rng.below(8));
    let base = if rng.chance(4) { 13 } else { 8 };
    match rng.below(10) {
        0..=5 => {
            let (load, byte) = (rng.flag(), rng.chance(3));
            let off = if base == 13 {
                rng.below(8) as i32 * 4
            } else {
                rng.below(0x100) as i32 - 0x80
            };
            if rng.chance(4) {
                // post-indexed: always writes the base back, so only a small step either way
                let step = (rng.below(5) as i32 - 2) * 4;
                a::ldst_imm(load, byte, false, false, base, rd, step)
            } else {
                a::ldst_imm(load, byte, true, rng.chance(6) && base == 13, base, rd, off)
            }
        }
        6..=7 => {
            // strh, ldrh, ldrsb, ldrsh with an immediate offset
            let (load, sh) = if rng.flag() {
                (false, 1)
            } else {
                (true, rng.range(1, 4))
            };
            let imm = rng.below(0x40);
            0xE140_0090
                | (rng.flag() as u32) << 23
                | (load as u32) << 20
                | base << 16
                | rd << 12
                | (imm & 0xF0) << 4
                | sh << 5
                | (imm & 0xF)
        }
        8 => {
            // ldrd or strd on an even register pair
            let sh = rng.range(2, 4);
            0xE140_0090
                | 1 << 23
                | base << 16
                | (rng.below(4) * 2) << 12
                | (rng.below(2) * 8)
                | sh << 5
        }
        _ => 0xE100_0090 | (rng.flag() as u32) << 22 | base << 16 | rd << 12 | rm, // swp, swpb
    }
}

/// Block transfers on `r8` or `r13` with every P/U/W combination (the list holds `r0..r7`).
fn block_transfer(rng: &mut Rng) -> u32 {
    let base = if rng.flag() { 13 } else { 8 };
    let list = (rng.next() & 0xFF) | 1 << rng.below(8);
    a::block(
        AL,
        rng.flag(),
        rng.flag(),
        rng.flag(),
        base == 13 && rng.chance(3),
        base,
        list,
    )
}

/// Status register moves that leave the mode alone.
fn status(rng: &mut Rng) -> u32 {
    match rng.below(5) {
        0 => a::mrs(rng.below(8), false),
        1 => a::mrs(rng.below(8), rng.chance(4)),
        2 => a::msr_imm(false, 8, rng.below(16), rng.below(256)),
        3 => a::msr_reg(false, 8, rng.below(8)),
        _ => a::msr_imm(false, 1, 0, rng.pick(&KEEP_MODE)),
    }
}

/// One straight-line word: no branch, no `pc`, writes only `r0..r7` (and `r8`/`r13` through
/// block-transfer write-back), addresses only the data page and the stack.
pub(super) fn straight_word(rng: &mut Rng) -> u32 {
    match rng.below(100) {
        0..=54 => data_processing(rng),
        55..=64 => multiply(rng),
        65..=84 => transfer(rng),
        85..=94 => block_transfer(rng),
        _ => status(rng),
    }
}

/// Words of callee `kind` (`0..6`) placed at `at`; `callee` is the address it calls when it nests.
/// All return through `lr`: `bx lr`, `mov pc,lr`, a framed `ldmia sp!,{..,pc}`, a nested call, a
/// conditional return, and a counted loop on `r12`. At most 16 words.
pub(super) fn function(rng: &mut Rng, kind: usize, at: u32, callee: u32) -> Vec<u32> {
    let k = rng.range(2, 7) as usize;
    let mut f: Vec<u32> = Vec::new();
    let body =
        |rng: &mut Rng, f: &mut Vec<u32>, k: usize| f.extend((0..k).map(|_| straight_word(rng)));
    match kind {
        0 => {
            body(rng, &mut f, k);
            f.push(a::bx(AL, 14));
        }
        1 => {
            body(rng, &mut f, k);
            f.push(0xE1A0_F00E); // mov pc,lr
        }
        2 => {
            f.push(a::push(0x4070)); // stmdb sp!,{r4-r6,lr}
            body(rng, &mut f, k);
            f.push(a::pop(0x8070)); // ldmia sp!,{r4-r6,pc}
        }
        3 => {
            f.push(a::push(0x4010)); // stmdb sp!,{r4,lr}
            f.push(a::branch(AL, true, at + 4, callee));
            body(rng, &mut f, k);
            f.push(a::pop(0x8010));
        }
        4 => {
            f.push(a::cmp_imm(0, rng.below(16)));
            f.push(with_cond(a::add_imm(false, 0, 0, 1), 12)); // addgt r0,r0,#1
            f.push(with_cond(a::mov_imm(0, 9), 13)); // movle r0,#9
            f.push(a::bx(1, 14)); // bxne lr
            body(rng, &mut f, 2);
            f.push(a::bx(AL, 14));
        }
        _ => {
            f.push(a::mov_imm(12, rng.range(2, 5)));
            body(rng, &mut f, k);
            f.push(a::sub_imm(true, 12, 12, 1));
            f.push(a::branch(1, false, at + 4 * f.len() as u32, at + 4));
            f.push(a::bx(AL, 14));
        }
    }
    f
}

/// The tail the body falls into: a loop of `r12` that reads a literal of the next page
/// (`POOL`, `pc`-relative), stores over it through `r9` and reads it again; then `b SPIN_AT`.
pub(super) fn tail(rng: &mut Rng) -> Vec<u32> {
    let j = rng.below(8);
    let target = POOL + 4 * j;
    let mut t = vec![a::mov_imm(12, rng.range(2, 5))];
    let top = TAIL_AT + 4;
    let here = |t: &Vec<u32>| TAIL_AT + 4 * t.len() as u32;
    t.push(a::mem_pc(true, 0, here(&t), target));
    t.push(a::dp_shift_imm(4, false, 2, 2, 0, 0, 0)); // add r2,r2,r0
    t.push(a::add_imm(false, 5, 5, rng.range(1, 200)));
    t.push(a::str(5, 9, 4 * j as i32));
    t.push(a::mem_pc(true, 1, here(&t), target));
    t.push(a::sub_imm(true, 12, 12, 1));
    t.push(a::branch(1, false, here(&t), top));
    t.push(a::branch(AL, false, here(&t), SPIN_AT));
    t
}

/// Words of every callee, in `FUNC_AT` order then `FAR_AT` order, with their addresses.
pub(super) fn callees(rng: &mut Rng) -> Vec<(u32, Vec<u32>)> {
    let mut out: Vec<(u32, Vec<u32>)> = Vec::new();
    for (kind, at) in FUNC_AT.iter().enumerate() {
        out.push((*at, function(rng, kind, *at, FUNC_AT[0])));
    }
    for (i, at) in FAR_AT.iter().enumerate() {
        let kind = [0, 2, 3, 4][i];
        out.push((*at, function(rng, kind, *at, FAR_AT[0])));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::testkit::{oracle_trace, random_cpu};

    #[test]
    fn straight_words_keep_the_register_contract_and_never_branch_or_fault() {
        let mut rng = Rng::new(21);
        let mut stores = 0;
        for case in 0..4000 {
            let w = straight_word(&mut rng);
            assert_eq!(w >> 28, AL, "{w:#010x} is unconditional");
            let mut cpu = random_cpu(&mut rng, 0x1000);
            (cpu.r[8], cpu.r[13]) = (0x9000_0000, 0x9000_2000);
            let before = cpu.r;
            let t = oracle_trace(&cpu, &[w], case, 1);
            assert_eq!(t.fault, None, "{w:#010x}");
            assert_eq!(t.cpu.r[15], 0x1004, "{w:#010x} must not branch");
            assert_eq!(t.cpu.mode, 0x13, "{w:#010x} keeps the mode");
            for r in [9, 10, 11, 12, 14] {
                assert_eq!(t.cpu.r[r], before[r], "{w:#010x} wrote r{r}");
            }
            for r in [8, 13] {
                let moved = t.cpu.r[r].wrapping_sub(before[r]) as i32;
                assert!(moved.abs() <= 0x20, "{w:#010x} moved r{r} by {moved}");
            }
            for e in &t.bus.log {
                assert!(
                    (0x8FFF_FF00..0x9000_2100).contains(&e.1),
                    "{w:#010x} accessed {:#x}",
                    e.1
                );
            }
            stores += t.bus.stores();
        }
        assert!(stores > 300, "the mix stores ({stores} in 4000 words)");
    }

    #[test]
    fn the_mix_reaches_every_part_and_is_deterministic() {
        use crate::cpu::predecode::{decode_arm, op};
        let (mut a1, mut a2) = (Rng::new(5), Rng::new(5));
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..3000 {
            let w = straight_word(&mut a1);
            assert_eq!(w, straight_word(&mut a2));
            seen.insert(decode_arm(w).op.min(op::DP_BASE));
        }
        for class in [
            op::SLOW,
            op::LDR,
            op::LDRB,
            op::STR,
            op::STRB,
            op::LDM,
            op::HALF,
            op::MUL,
            op::DP_BASE,
        ] {
            assert!(seen.contains(&class), "class {class} never generated");
        }
    }

    #[test]
    fn callees_fit_their_slots_and_return_through_lr() {
        let mut rng = Rng::new(25);
        let all = callees(&mut rng);
        assert_eq!(all.len(), FUNC_AT.len() + FAR_AT.len());
        for (at, f) in &all {
            assert!(f.len() <= 16, "callee at {at:#x} has {} words", f.len());
        }
        assert!(
            tail(&mut rng).len() * 4 <= 0xFC0 - 0xF00,
            "the tail ends before the spin word"
        );
        // kinds 0..5 return with bx lr, mov pc,lr, ldm pc, ldm pc, bx lr, bx lr
        let ends: Vec<u32> = all
            .iter()
            .take(6)
            .map(|(_, f)| *f.last().unwrap())
            .collect();
        assert_eq!(ends[0], a::bx(AL, 14));
        assert_eq!(ends[1], 0xE1A0_F00E);
        assert_eq!((ends[2], ends[3]), (a::pop(0x8070), a::pop(0x8010)));
    }
}
