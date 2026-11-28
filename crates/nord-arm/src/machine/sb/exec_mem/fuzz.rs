//! Random suites of the memory arms: generators per family, the feature [`Tally`] that makes a
//! run prove it exercised each rule, and the mutation check (every mutant site of `exec_mem.rs`
//! must make the small suite fail). The harness (encoders, field table, comparator) is `diff.rs`.

use super::diff::{
    diff_case, enc_block, enc_half, enc_single, mode, real, uop_of, Exec, Mode, Off, Ran,
};
use super::*;
use crate::cpu::alu::shift;
use crate::machine::sb::mutants;
use crate::machine::sb::testkit::{fuzz_cases, fuzz_seed, random_cpu, Rng};
use std::collections::BTreeMap;

/// The mutants this file has sites for, for `mutants::MUTANTS` (W3-VERIF registers them; its
/// check that every registered id has a site is satisfied by the `mutant(<id>)` calls of `exec_mem.rs`).
/// ASSUMED: the shared block `300..=399` is split `300..=349` W1-XALU, `350..=399` W1-XMEM; confirm
/// with W3-VERIF when it registers both.
pub const MUTANTS: &[(u32, &str)] = &[
    (350, "xmem: word load drops the unaligned rotate"),
    (351, "xmem: word store keeps the low address bits"),
    (352, "xmem: load writes rd before the base write-back"),
    (353, "xmem: offset direction inverted"),
    (354, "xmem: post-indexed access uses the updated address"),
    (355, "xmem: halfword access keeps address bit 0"),
    (356, "xmem: ldrsb zero-extends"),
    (357, "xmem: ldrsh zero-extends"),
    (358, "xmem: ldrd writes the base back after the pair"),
    (359, "xmem: stm writes back after the last store"),
    (360, "xmem: ldm writes back when the base is in the list"),
    (361, "xmem: decrement-before block starts one word low"),
    (362, "xmem: pc not published to the bus"),
    (363, "xmem: strd stores the high word first"),
    (364, "xmem: register offset ignores the shift type"),
    (365, "xmem: ldr_abs drops the unaligned rotate"),
    (366, "xmem: store write-back dropped"),
];

/// Counts of the features a run hit.
#[derive(Default, Debug)]
pub(super) struct Tally(BTreeMap<&'static str, u32>);

impl Tally {
    fn hit(&mut self, name: &'static str, yes: bool) {
        *self.0.entry(name).or_default() += u32::from(yes);
    }

    /// Fail unless every feature was hit at least `min` times (a vacuous run must not pass).
    pub fn all_at_least(&self, min: u32) {
        assert!(!self.0.is_empty(), "nothing was counted");
        for (name, n) in &self.0 {
            assert!(
                *n >= min,
                "feature `{name}` hit {n} times, wanted {min}: {self:?}"
            );
        }
    }
}

/// The families of words the suites generate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Family {
    Single,
    Half,
    Block,
    /// `LDR_ABS`: the interpreter's `ldr rd,[r15]` with `r15` holding the address.
    Abs,
}

/// A register 0..=14, often a low one so base, destination and offset collide.
fn reg(rng: &mut Rng) -> u32 {
    if rng.chance(2) {
        rng.below(4)
    } else {
        rng.below(15)
    }
}

fn rand_mode(rng: &mut Rng) -> Mode {
    mode(rng.below(2), rng.below(2), rng.below(2))
}

fn gen_word(rng: &mut Rng, fam: Family) -> u32 {
    let (rn, rd, m) = (reg(rng), reg(rng), rand_mode(rng));
    match fam {
        Family::Single => {
            let imm = rng.below(0x1000);
            let off = match rng.below(3) {
                0 => Off::Imm(rng.pick(&[0, 1, 3, 4, 0xFFF, imm])),
                1 => Off::Reg(reg(rng)),
                _ => Off::Shifted {
                    rm: reg(rng),
                    ty: rng.below(4),
                    amount: if rng.chance(4) { 0 } else { rng.below(32) },
                },
            };
            enc_single(rng.below(2), rng.below(2), m, rn, rd, off)
        }
        Family::Half => {
            let sh = 1 + rng.below(3);
            let off = if rng.chance(2) {
                Off::Reg(reg(rng))
            } else {
                Off::Imm(rng.below(256))
            };
            enc_half(rng.below(2), sh, m, rn, rd, off)
        }
        Family::Block => {
            let mut list = rng.next() & 0x7FFF;
            if rng.chance(3) {
                list &= rng.next() & rng.next(); // sparse lists
            }
            match rng.below(3) {
                0 => list |= 1 << rn,
                1 => list &= !(1 << rn),
                _ => {}
            }
            enc_block(
                rng.below(2),
                m,
                rn,
                if list == 0 { 1 << rng.below(15) } else { list },
            )
        }
        Family::Abs => enc_single(1, 0, mode(1, 1, 0), 15, rd, Off::Imm(0)),
    }
}

/// The address a single or half transfer computes before the bus aligns it (the log holds the
/// aligned one).
fn raw_addr(word: u32, before: &[u32; 16]) -> u32 {
    let reg = before[(word & 15) as usize];
    let off = match word >> 25 & 7 {
        2 => word & 0xFFF,
        3 => shift(word >> 5 & 3, word >> 7 & 31, reg, false, true).0,
        _ if word >> 22 & 1 == 1 => word >> 4 & 0xF0 | word & 0xF,
        _ => reg,
    };
    let base = before[(word >> 16 & 15) as usize];
    let moved = if word >> 23 & 1 == 1 {
        base.wrapping_add(off)
    } else {
        base.wrapping_sub(off)
    };
    if word >> 24 & 1 == 1 {
        moved
    } else {
        base
    }
}

/// Facts of one interpreted case, counted so the run can prove it exercised each rule.
fn record(t: &mut Tally, fam: Family, word: u32, ran: &Ran) {
    let (rn, rd) = ((word >> 16 & 15) as usize, (word >> 12 & 15) as usize);
    let (p, up, w, l) = (
        word >> 24 & 1,
        word >> 23 & 1,
        word >> 21 & 1,
        word >> 20 & 1 == 1,
    );
    let wb = p == 0 || w == 1;
    let addr = raw_addr(word, &ran.before);
    let moved = ran.after.r[rn] != ran.before[rn];
    t.hit("register file changed", ran.after.r != ran.before);
    t.hit("access logged", !ran.log.is_empty());
    match fam {
        Family::Single => {
            let (byte, reg) = (word >> 22 & 1 == 1, word >> 25 & 1 == 1);
            let (ty, amount) = (word >> 5 & 3, word >> 7 & 31);
            t.hit("single: load", l);
            t.hit("single: store", !l);
            t.hit("single: byte", byte);
            t.hit("single: unaligned word access", !byte && addr & 3 != 0);
            t.hit("single: register offset", reg);
            t.hit(
                "single: lsr/asr #0 offset",
                reg && (1..=2).contains(&ty) && amount == 0,
            );
            t.hit("single: ror offset", reg && ty == 3 && amount != 0);
            t.hit("single: post-indexed", p == 0);
            t.hit("single: subtract", up == 0);
            t.hit("single: store write-back moved the base", !l && wb && moved);
            t.hit(
                "single: load write-back moved the base",
                l && wb && rn != rd && moved,
            );
            t.hit(
                "single: load with base == dest and write-back",
                l && wb && rn == rd,
            );
            t.hit(
                "single: store with base == dest and write-back",
                !l && wb && rn == rd,
            );
        }
        Family::Half => {
            let sh = word >> 5 & 3;
            t.hit("half: ldrh", l && sh == 1);
            t.hit(
                "half: ldrsb negative",
                l && sh == 2 && ran.after.r[rd] >> 31 == 1,
            );
            t.hit(
                "half: ldrsh negative",
                l && sh == 3 && ran.after.r[rd] >> 31 == 1,
            );
            t.hit("half: strh", !l && sh == 1);
            t.hit("half: ldrd", !l && sh == 2);
            t.hit("half: strd", !l && sh == 3);
            t.hit(
                "half: odd halfword address",
                (sh == 1 || l && sh == 3) && addr & 1 == 1,
            );
            t.hit("half: odd byte address", l && sh == 2 && addr & 1 == 1);
            t.hit("half: register offset", word >> 22 & 1 == 0);
            t.hit(
                "half: ldrd write-back onto the pair",
                !l && sh == 2 && wb && (rn == rd || rn == rd + 1),
            );
            t.hit(
                "half: load with base == dest and write-back",
                l && wb && rn == rd,
            );
            t.hit("half: write-back moved the base", wb && moved);
        }
        Family::Block => {
            let list = word & 0xFFFF;
            let lowest = list.trailing_zeros() as usize;
            t.hit("block: ldm", l);
            t.hit("block: stm", !l);
            t.hit("block: increment after", up == 1 && p == 0);
            t.hit("block: increment before", up == 1 && p == 1);
            t.hit("block: decrement after", up == 0 && p == 0);
            t.hit("block: decrement before", up == 0 && p == 1);
            t.hit("block: write-back", w == 1);
            t.hit(
                "block: ldm with the base in the list and write-back",
                l && w == 1 && list >> rn & 1 == 1,
            );
            t.hit(
                "block: stm with write-back, base in the list, not the lowest",
                !l && w == 1 && list >> rn & 1 == 1 && rn != lowest,
            );
            t.hit(
                "block: stm with write-back and the base the lowest",
                !l && w == 1 && rn == lowest,
            );
            t.hit("block: eight or more registers", list.count_ones() >= 8);
            t.hit("block: unaligned base", ran.before[rn] & 3 != 0);
        }
        Family::Abs => {
            t.hit("abs: unaligned address", ran.before[15] & 3 != 0);
        }
    }
}

/// Run `n` random cases of `fam` against `exec`; the first difference is the `Err`.
pub(super) fn run_family(
    fam: Family,
    n: u32,
    seed: u64,
    exec: Exec,
    t: &mut Tally,
) -> Result<(), String> {
    let mut rng = Rng::new(seed);
    for i in 0..n {
        let pc = 0xC000_0000 | rng.below(0x400) << 2;
        let mut cpu = random_cpu(&mut rng, pc);
        let (word, u) = loop {
            let word = gen_word(&mut rng, fam);
            if fam == Family::Abs {
                cpu.r[15] = rng.biased();
                let mut u = Uop::new(k::LDR_ABS, pc);
                (u.rd, u.imm) = ((word >> 12 & 15) as u8, cpu.r[15]);
                if u.rd != 15 {
                    break (word, u);
                }
            } else if let Some(u) = uop_of(word, pc) {
                break (word, u);
            }
        };
        let ran =
            diff_case(word, &u, &cpu, rng.next(), exec).map_err(|e| format!("case {i}: {e}"))?;
        record(t, fam, word, &ran);
    }
    Ok(())
}

const FAMILIES: [Family; 4] = [Family::Single, Family::Half, Family::Block, Family::Abs];

/// Cases per family at the CI bound: `NS4_FUZZ_CASES` (default 60) times 334 is about 20 k words
/// per family; the local gate sets 3000 for 1 M.
fn cases() -> u32 {
    (fuzz_cases(60) * 334).max(1000) as u32
}

fn fuzz(fam: Family) {
    let n = cases();
    let mut t = Tally::default();
    if let Err(e) = run_family(fam, n, fuzz_seed() + fam as u64, &real, &mut t) {
        panic!("{fam:?}: {e}");
    }
    t.all_at_least(n / 100);
}

#[test]
fn single_transfers_match_ldst_exec() {
    fuzz(Family::Single);
}

#[test]
fn half_and_double_transfers_match_arm_ldst_half() {
    fuzz(Family::Half);
}

#[test]
fn block_transfers_match_arm_ldm() {
    fuzz(Family::Block);
}

#[test]
fn absolute_loads_match_a_pc_base_load() {
    fuzz(Family::Abs);
}

#[test]
fn a_vacuous_run_is_refused() {
    let mut t = Tally::default();
    t.hit("never", false);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| t.all_at_least(1)));
    assert!(
        caught.is_err(),
        "a feature that was never hit must fail the run"
    );
    let empty = std::panic::catch_unwind(|| Tally::default().all_at_least(0));
    assert!(empty.is_err(), "a run that counted nothing must fail");
}

/// Every family at a small fixed size: the suite a mutant must fail.
fn small_suite() -> Result<(), String> {
    FAMILIES.iter().try_for_each(|&fam| {
        let mut t = Tally::default();
        run_family(fam, 3000, 11 + fam as u64, &real, &mut t)
    })
}

#[test]
fn every_mutant_is_killed_by_the_differential_suite() {
    assert_eq!(small_suite(), Ok(()), "the unmutated arms must pass");
    for &(id, name) in MUTANTS {
        let verdict = mutants::with(id, small_suite);
        assert!(verdict.is_err(), "mutant {id} ({name}) survived the suite");
    }
}

#[test]
fn the_mutant_ids_are_unique_in_the_block_of_this_package_and_each_has_a_site() {
    let source = include_str!("../exec_mem.rs");
    let mut ids: Vec<u32> = MUTANTS.iter().map(|m| m.0).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), MUTANTS.len(), "duplicate id");
    for &(id, name) in MUTANTS {
        assert!(
            (350..=399).contains(&id),
            "{name}: {id} is outside 350..=399"
        );
        assert!(
            source.contains(&format!("mutant({id})")),
            "{name}: no site for {id}"
        );
    }
}
