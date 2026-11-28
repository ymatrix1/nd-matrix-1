//! Test-only program generator of W1-IDLE: ARM code with finite counting phases followed by an
//! idle (or deliberately non-idle) tail, so the v1 and v2 schedules see the same program from
//! different phases.
//!
//! Register convention: `r0` stays 0, `r4` is the loop counter, `r5..r9` are scratch, `r10` is
//! `0x2000` (plain RAM), `r11` is `0x6000_0000` (a device window). A spin loop only writes
//! constants, reloads unchanged memory, stores the value already there (`r0 = 0`) or sets flags,
//! so its state recurs after one period once the first iterations have settled.

use super::super::testkit::Rng;
use crate::machine::Machine;

/// Where the programs are loaded.
pub(super) const CODE: u32 = 0xC000_0000;

pub(super) const AL: u32 = 0xE;
const EQ: u32 = 0x0;
const NE: u32 = 0x1;

/// A machine with `code` at [`CODE`] and `pc` on its first word.
pub(super) fn machine(code: &[u32]) -> Machine {
    let mut m = Machine::new();
    for (i, w) in code.iter().enumerate() {
        m.bus.mem.write32(CODE + 4 * i as u32, *w);
    }
    m.cpu.r[15] = CODE;
    m
}

/// `mov rd, #(imm8 ror 2*rot)`.
pub(super) fn mov_imm(cond: u32, rd: u32, imm8: u32, rot: u32) -> u32 {
    cond << 28 | 0x03A0_0000 | rd << 12 | rot << 8 | imm8
}

/// `b<cond>` at word index `from` to word index `to`.
pub(super) fn branch(cond: u32, from: usize, to: usize) -> u32 {
    let off = to as i32 - (from as i32 + 2);
    cond << 28 | 0x0A00_0000 | (off as u32 & 0x00FF_FFFF)
}

/// `cmp|tst rn, rm` (flags only).
fn cmp_reg(cond: u32, tst: bool, rn: u32, rm: u32) -> u32 {
    cond << 28 | if tst { 0x0110_0000 } else { 0x0150_0000 } | rn << 16 | rm
}

fn cmp_imm(cond: u32, rn: u32, imm8: u32) -> u32 {
    cond << 28 | 0x0350_0000 | rn << 16 | imm8
}

/// `mov rd, rm` with the S bit when `s`.
fn mov_reg(cond: u32, s: bool, rd: u32, rm: u32) -> u32 {
    cond << 28 | 0x01A0_0000 | (s as u32) << 20 | rd << 12 | rm
}

/// `ldr|ldrb|str rd, [rn, #off]` (immediate offset, no write-back).
fn mem(load: bool, byte: bool, rd: u32, rn: u32, off: u32) -> u32 {
    0xE580_0000 | (byte as u32) << 22 | (load as u32) << 20 | rn << 16 | rd << 12 | off
}

/// `mcr p15, 0, r0, c7, c0, 4`: wait for interrupt.
pub(super) const WFI: u32 = 0xEE07_0F90;
/// `b .`
pub(super) const SPIN1: u32 = 0xEAFF_FFFE;

/// How a program ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tail {
    /// A pure loop of this period.
    Spin(u32),
    /// `ldr r5,[r10] ; cmp r5,#0 ; beq top ; b .` (a memory poll that never changes).
    Poll,
    /// `ldr r5,[r11] ; b top`: a device read per iteration, never idle.
    Device,
    /// `wfi ; b .`
    Wfi,
}

/// A generated program.
pub(super) struct Prog {
    pub code: Vec<u32>,
    pub budget: u64,
    pub tail: Tail,
    /// Instructions of the counting phases (the non-idle part).
    pub work: u64,
    /// The budget leaves room for any schedule to reach the idle tail and skip.
    pub must_skip: bool,
}

fn pick_cond(rng: &mut Rng) -> u32 {
    if rng.chance(3) {
        rng.pick(&[EQ, NE, 2, 3, 4, 5])
    } else {
        AL
    }
}

/// One instruction that only reads state or rewrites what it already holds.
fn pure_op(rng: &mut Rng) -> u32 {
    let (a, b, x) = (rng.below(12), rng.below(12), 5 + rng.below(5));
    match rng.below(9) {
        0 => cmp_reg(pick_cond(rng), false, a, b),
        1 => cmp_reg(pick_cond(rng), true, a, b),
        2 => cmp_imm(pick_cond(rng), a, rng.below(256)),
        3 => mov_reg(pick_cond(rng), false, x, x),
        4 => mov_imm(AL, 7 + rng.below(3), rng.below(256), 0),
        5 => mem(true, false, x, 10, 4 * rng.below(16)),
        6 => mem(true, true, x, 10, rng.below(64)),
        7 => mem(false, false, 0, 10, 4 * rng.below(16)),
        _ => mov_reg(AL, true, x, x),
    }
}

/// A loop of exactly `period` instructions ending in the branch to its first word.
pub(super) fn spin_loop(rng: &mut Rng, period: u32) -> Vec<u32> {
    let n = period as usize;
    let mut code: Vec<u32> = (0..n - 1).map(|_| pure_op(rng)).collect();
    code.push(branch(AL, n - 1, 0));
    if n >= 2 && rng.chance(3) {
        // a conditional back branch that is always taken (r0 is 0): cmp r0,#0 ; beq top
        code[n - 2] = cmp_imm(AL, 0, 0);
        code[n - 1] = branch(EQ, n - 1, 0);
    }
    code
}

/// A counting phase (real progress: the counter, a sum and memory change); returns its length in
/// instructions.
fn count_phase(rng: &mut Rng, code: &mut Vec<u32>) -> u64 {
    let (imm8, k) = (1 + rng.below(255), rng.below(3));
    let iters = u64::from(imm8) << (2 * k);
    code.push(mov_imm(AL, 4, imm8, (16 - k) & 15));
    let top = code.len();
    let noise = rng.below(4);
    for _ in 0..noise {
        code.push(match rng.below(4) {
            0 => 0xE085_5004, // add r5,r5,r4
            1 => 0xE026_6005, // eor r6,r6,r5
            2 => mem(false, false, 5, 10, 4 * rng.below(16)),
            _ => mem(true, false, 7, 10, 4 * rng.below(16)),
        });
    }
    code.push(0xE254_4001); // subs r4,r4,#1
    code.push(branch(NE, code.len(), top));
    1 + iters * (u64::from(noise) + 2)
}

/// Program number `i` of a run: the tail kind and the spin period cycle with `i`, so any 200
/// consecutive programs cover every period 1..=40 and every tail; the rest comes from `rng`.
pub(super) fn random_prog(rng: &mut Rng, i: u64) -> Prog {
    let mut code = vec![0xE3A0_AA02, 0xE3A0_B206]; // mov r10,#0x2000 ; mov r11,#0x6000_0000
    let mut work = 2;
    for _ in 0..rng.below(3) {
        work += count_phase(rng, &mut code);
    }
    let (q, r) = (i / 8, i % 8);
    let tail = match r {
        0..=4 => Tail::Spin(1 + ((5 * q + r) % 40) as u32),
        5 => Tail::Poll,
        6 => Tail::Device,
        _ => Tail::Wfi,
    };
    let top = code.len();
    match tail {
        Tail::Spin(p) => code.extend(spin_loop(rng, p)),
        Tail::Poll => code.extend([
            mem(true, false, 5, 10, 0),
            cmp_imm(AL, 5, 0),
            branch(EQ, top + 2, top),
            SPIN1,
        ]),
        Tail::Device => code.extend([mem(true, false, 5, 11, 0), branch(AL, top + 1, top)]),
        Tail::Wfi => code.extend([WFI, SPIN1]),
    }
    let idle = matches!(tail, Tail::Spin(_) | Tail::Poll);
    let (budget, must_skip) = match rng.below(4) {
        0 => (1 + u64::from(rng.below(work as u32 + 1)), false),
        1 => (work + u64::from(rng.below(2000)), false),
        _ => (3 * work + 30_000 + u64::from(rng.below(20_000)), idle),
    };
    Prog {
        code,
        budget,
        tail,
        work,
        must_skip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn a_spin_loop_has_exactly_its_period_and_branches_to_its_first_word() {
        let mut rng = Rng::new(1);
        for p in 1..=40 {
            let code = spin_loop(&mut rng, p);
            assert_eq!(code.len(), p as usize);
            let last = *code.last().unwrap();
            let off = ((last & 0x00FF_FFFF) << 8) as i32 >> 8;
            assert_eq!(last >> 24 & 0xF, 0xA, "a branch");
            assert_eq!(code.len() as i32 - 1 + 2 + off, 0, "period {p}");
        }
    }

    #[test]
    fn programs_are_deterministic_and_cover_every_period_and_tail() {
        let gen = |seed| {
            let mut rng = Rng::new(seed);
            (0..200)
                .map(|i| random_prog(&mut rng, i))
                .collect::<Vec<_>>()
        };
        let (a, b) = (gen(7), gen(7));
        assert!(a
            .iter()
            .zip(&b)
            .all(|(x, y)| x.code == y.code && x.budget == y.budget));
        let periods: BTreeSet<u32> = a
            .iter()
            .filter_map(|p| {
                if let Tail::Spin(n) = p.tail {
                    Some(n)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(periods, (1..=40).collect());
        for t in [Tail::Poll, Tail::Device, Tail::Wfi] {
            assert!(a.iter().any(|p| p.tail == t), "{t:?}");
        }
        assert!(a.iter().all(|p| p.code.len() < 1000 && p.budget > 0));
        assert!(a.iter().any(|p| p.must_skip) && a.iter().any(|p| !p.must_skip));
    }

    #[test]
    fn a_counting_phase_runs_for_the_instructions_it_reports() {
        let mut rng = Rng::new(3);
        let mut code = vec![0xE3A0_AA02];
        let n = count_phase(&mut rng, &mut code) + 1;
        code.push(SPIN1);
        let mut m = machine(&code);
        let (_, stop) = m.run(n, 0, None);
        assert_eq!(stop, crate::machine::Stop::Budget);
        assert_eq!(
            m.cpu.r[15],
            CODE + 4 * (code.len() as u32 - 1),
            "now on the final `b .`"
        );
        assert_eq!(m.reg(4), 0);
    }
}
