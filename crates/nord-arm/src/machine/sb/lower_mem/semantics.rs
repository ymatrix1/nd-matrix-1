//! OWNER: W1-MEM (tests). The lowered uops against `Cpu::step`: random words of every class on
//! random states, registers and the ordered bus access log compared (`model.rs` executes the
//! uops, because the real executor belongs to W1-XMEM). Every test counts what it exercised and
//! asserts a floor, and one test proves the comparator catches a wrongly packed field.

use super::words::{block, half, literal, rrx, single, wild};
use super::{model, FLAG_P, FLAG_U, FLAG_WB, MUTANTS};
use crate::cpu::Cpu;
use crate::machine::sb::config::SbConfig;
use crate::machine::sb::decode::{decode, lower_or_generic, LowerCtx, Lowered};
use crate::machine::sb::mutants;
use crate::machine::sb::testkit::{fuzz_cases, fuzz_seed, random_cpu, LogBus, Rng};
use crate::machine::sb::trace::BuildOpts;
use crate::machine::sb::uop::{holds, k, Uop};

/// What one lowered case did.
#[derive(Default, Clone, Copy, Debug)]
pub(super) struct Seen {
    pub(super) loads: u32,
    pub(super) stores: u32,
    pub(super) wrote_back: bool,
    pub(super) skipped: bool,
    pub(super) folded: bool,
}

#[derive(Default, Debug)]
struct Tally {
    cases: u32,
    lowered: u32,
    loads: u32,
    stores: u32,
    wrote_back: u32,
    skipped: u32,
    folded: u32,
}

impl Tally {
    fn add(&mut self, seen: Option<Seen>) {
        self.cases += 1;
        if let Some(s) = seen {
            self.lowered += 1;
            self.loads += u32::from(s.loads > 0);
            self.stores += u32::from(s.stores > 0);
            self.wrote_back += u32::from(s.wrote_back);
            self.skipped += u32::from(s.skipped);
            self.folded += u32::from(s.folded);
        }
    }
}

/// A deliberate corruption of the lowered uops (the comparator test).
type Mutation = fn(&mut [Uop]);

pub(super) fn unmutated(_: &mut [Uop]) {}

/// `word` at `pc` on a random state from `rng`: the registry's uops (after `mutate`) under the
/// model against `Cpu::step`. `Ok(None)` when the registry left it to `GENERIC`.
pub(super) fn check(
    word: u32,
    pc: u32,
    rng: &mut Rng,
    fold: bool,
    mutate: Mutation,
) -> Result<Option<Seen>, String> {
    let seed = rng.next();
    let cpu = random_cpu(rng, pc);
    check_on(word, cpu, seed, fold, mutate)
}

/// [`check`] on the given state `cpu` (its `r15` is the instruction's address) and bus fill `seed`.
pub(super) fn check_on(
    word: u32,
    cpu: Cpu,
    seed: u32,
    fold: bool,
    mutate: Mutation,
) -> Result<Option<Seen>, String> {
    let pc = cpu.r[15];
    // three equal memories, each holding the instruction itself (a load may read its own word)
    let (mut ba, mut bb, mut oracle) = (LogBus::new(seed), LogBus::new(seed), LogBus::new(seed));
    for bus in [&mut ba, &mut bb, &mut oracle] {
        bus.poke32(pc, word);
    }
    let opts = BuildOpts {
        fold_literals: fold,
        ..BuildOpts::from(&SbConfig::on())
    };
    let mut out = Vec::new();
    let mut peek = |a: u32| Some(oracle.peek32(a));
    let lowered = lower_or_generic(&mut LowerCtx {
        insn: decode(pc, word),
        out: &mut out,
        fold: &mut peek,
        opts: &opts,
    });
    if lowered == Lowered::Generic {
        return Ok(None);
    }
    mutate(&mut out);
    let mut ca = cpu.clone();
    ba.pc = pc;
    ca.step(&mut ba)
        .map_err(|f| format!("{word:#010x}: {f:?}"))?;
    let (mut cb, mut i, mut seen) = (cpu.clone(), 0, Seen::default());
    // A straight-line instruction leaves the pc at the next word, and no memory uop writes `r15`
    // (I2), so the model starts there: a uop that wrote the pc (a write-back of `pc`), or an
    // instruction the interpreter takes as a jump, then shows as a difference in `r15`.
    cb.r[15] = pc.wrapping_add(4);
    let flags = cb.nzcv() as u8;
    while i < out.len() {
        let u = out[i];
        if u.kind == k::PRED {
            if !holds(u.cond, flags) {
                i += u.imm as usize;
                seen.skipped = true;
            }
        } else {
            model::run(&mut cb.r, &mut bb, &u);
        }
        i += 1;
    }
    let what = format!("{word:#010x} at {pc:#x} lowered to {out:?}");
    if ca.r != cb.r {
        return Err(format!("{what}: registers {:x?} != {:x?}", ca.r, cb.r));
    }
    seen.folded = out.iter().any(|u| u.kind == k::MOV_IMM);
    if seen.folded && !seen.skipped {
        // folding removes the one load the interpreter performs
        if !(ba.log.len() == 1 && ba.log[0].0 <= 2 && bb.log.is_empty()) {
            return Err(format!(
                "{what}: folded literal logs {:x?} / {:x?}",
                ba.log, bb.log
            ));
        }
    } else if ba.log != bb.log {
        return Err(format!(
            "{what}: access logs {:x?} != {:x?}",
            ba.log, bb.log
        ));
    }
    seen.loads = ba.log.iter().filter(|e| e.0 <= 2).count() as u32;
    seen.stores = ba.log.iter().filter(|e| e.0 > 2).count() as u32;
    seen.wrote_back = !ba.log.is_empty()
        && out
            .iter()
            .any(|u| k::family(u.kind) == k::Family::Mem && u.rs & FLAG_WB != 0);
    Ok(Some(seen))
}

fn pick_pc(rng: &mut Rng) -> u32 {
    rng.pick(&[0x1000, 0x1004, 0xC000_0FFC, 0xFFFF_FFF8, 0xFFFF_FFFC, 0])
}

/// A valid word of class `class` (0 word/byte, 1 half, 2 block, 3 literal) and its pc.
fn word_of(rng: &mut Rng, class: u32) -> (u32, u32) {
    let (a, b, c) = (rng.chance(2), rng.chance(2), rng.chance(2));
    let sh = 1 + rng.below(3);
    let pc = pick_pc(rng);
    let word = match class {
        0 => single(rng, a, b, c).0,
        1 => half(rng, a, sh, c).0,
        2 => block(rng, a).0,
        _ => literal(rng, pc).0,
    };
    (word, pc)
}

fn cases() -> u32 {
    fuzz_cases(60) as u32 * 10
}

#[test]
fn lowered_memory_uops_equal_the_interpreter_on_random_states() {
    let mut rng = Rng::new(0x71 ^ fuzz_seed());
    let (mut t, mut per_class) = (Tally::default(), [0u32; 3]);
    for i in 0..cases() {
        for class in 0..3 {
            let (word, pc) = word_of(&mut rng, class);
            let seen = check(word, pc, &mut rng, true, unmutated)
                .unwrap_or_else(|e| panic!("case {i}: {e}"));
            per_class[class as usize] += u32::from(seen.is_some_and(|s| s.loads + s.stores > 0));
            t.add(seen);
        }
    }
    // every valid form lowers, and the run is not vacuous: loads, stores, write-back and
    // failed conditions all happened in every class
    assert_eq!(t.lowered, t.cases, "{t:?}");
    let n = t.cases;
    assert!(t.loads > n / 4 && t.stores > n / 4, "{t:?}");
    assert!(t.wrote_back > n / 6 && t.skipped > n / 20, "{t:?}");
    assert!(per_class.iter().all(|c| *c > cases() / 2), "{per_class:?}");
}

#[test]
fn literals_fold_to_the_interpreter_value_and_otherwise_load_at_run_time() {
    let mut rng = Rng::new(0x72 ^ fuzz_seed());
    let (mut on, mut off, mut unaligned) = (Tally::default(), Tally::default(), 0);
    for i in 0..cases() {
        let pc = pick_pc(&mut rng);
        let (word, addr) = literal(&mut rng, pc);
        let seen =
            check(word, pc, &mut rng, true, unmutated).unwrap_or_else(|e| panic!("{i}: {e}"));
        on.add(seen);
        let seen =
            check(word, pc, &mut rng, false, unmutated).unwrap_or_else(|e| panic!("{i}: {e}"));
        off.add(seen);
        unaligned += u32::from(addr & 3 != 0);
    }
    // the literal may be the instruction's own word, the next one, or the largest offset
    for word in [
        0xE51F_0008, // ldr r0,[pc,#-8]: its own word
        0xE51F_0004, // ldr r0,[pc,#-4]
        0xE59F_0000, // ldr r0,[pc]: pc + 8
        0xE59F_0FFF, // ldr r0,[pc,#4095]: unaligned, rotated
        0xE55F_0008, // ldrb r0,[pc,#-8]
        0xE15F_00B8, // ldrh r0,[pc,#-8]
    ] {
        for pc in [0x1000, 0x1004, 0xC000_0FFC, 0xFFFF_FFF8, 0xFFFF_FFFC, 0] {
            for fold in [true, false] {
                let seen =
                    check(word, pc, &mut rng, fold, unmutated).unwrap_or_else(|e| panic!("{e}"));
                let aligned_word = matches!(word, 0xE51F_0008 | 0xE51F_0004 | 0xE59F_0000);
                assert_eq!(
                    seen.is_some(),
                    fold || aligned_word,
                    "{word:#010x} at {pc:#x}"
                );
            }
        }
    }
    let n = cases();
    assert_eq!(
        (on.lowered, on.folded),
        (n, n),
        "every literal folds when it can"
    );
    assert!(
        on.skipped > n / 20 && unaligned > n / 2,
        "{on:?} {unaligned}"
    );
    // without folding only aligned word loads become LDR_ABS (about a tenth of them here)
    assert!(
        off.lowered > n / 30 && off.lowered < n / 2 && off.folded == 0,
        "{off:?}"
    );
    assert!(off.loads > n / 40, "{off:?}");
}

#[test]
fn any_word_of_the_encoding_space_agrees_with_the_interpreter_whenever_it_lowers() {
    let mut rng = Rng::new(0x73 ^ fuzz_seed());
    let (mut t, mut generic_without_log) = (Tally::default(), 0);
    for i in 0..cases() * 30 {
        let (word, pc) = (wild(&mut rng), pick_pc(&mut rng));
        let seen = check(word, pc, &mut rng, i % 2 == 0, unmutated)
            .unwrap_or_else(|e| panic!("case {i}: {e}"));
        generic_without_log += u32::from(seen.is_none());
        t.add(seen);
    }
    let n = t.cases;
    assert!(t.lowered > n / 3 && generic_without_log > n / 10, "{t:?}");
    assert!(
        t.loads > n / 8 && t.stores > n / 8 && t.wrote_back > n / 12,
        "{t:?}"
    );
    assert!(t.folded > 0, "wild words reach the literal fold too: {t:?}");
}

#[test]
fn the_comparison_catches_a_wrongly_packed_field() {
    fn flip_u(v: &mut [Uop]) {
        v.iter_mut().for_each(|u| u.rs ^= FLAG_U);
    }
    fn flip_p(v: &mut [Uop]) {
        v.iter_mut().for_each(|u| u.rs ^= FLAG_P);
    }
    fn flip_wb(v: &mut [Uop]) {
        v.iter_mut().for_each(|u| u.rs ^= FLAG_WB);
    }
    fn next_rd(v: &mut [Uop]) {
        v.iter_mut()
            .filter(|u| u.kind != k::PRED)
            .for_each(|u| u.rd = (u.rd + 1) % 14);
    }
    fn other_base(v: &mut [Uop]) {
        v.iter_mut()
            .filter(|u| u.kind != k::PRED)
            .for_each(|u| u.rn = (u.rn + 1) % 14);
    }
    fn offset(v: &mut [Uop]) {
        let not_block = |u: &&mut Uop| !matches!(u.kind, k::PRED | k::LDM | k::STM);
        v.iter_mut().filter(not_block).for_each(|u| u.imm ^= 4);
    }
    let muts: [(&str, Mutation); 6] = [
        ("U", flip_u),
        ("P", flip_p),
        ("WB", flip_wb),
        ("rd", next_rd),
        ("rn", other_base),
        ("imm", offset),
    ];
    for (name, mutate) in muts {
        let mut rng = Rng::new(0x74);
        let (mut caught, mut ran) = (0, 0);
        for i in 0..600 {
            let (word, pc) = word_of(&mut rng, i % 3);
            // the same word on the same random state, unmutated and mutated
            let (mut same, mut mutated) = (rng.clone(), rng.clone());
            let good = check(word, pc, &mut same, true, unmutated);
            assert!(matches!(good, Ok(Some(_))), "{good:?}");
            rng = same;
            ran += 1;
            caught += u32::from(check(word, pc, &mut mutated, true, mutate).is_err());
        }
        assert!(
            caught * 5 > ran,
            "mutation {name} caught in only {caught} of {ran} cases"
        );
    }
}

#[test]
fn the_comparison_sees_a_uop_that_writes_the_pc() {
    // `ldr r0,[pc,#4]` folds to `MOV_IMM r0 = v`. r0 already holds v, so the mutant that retargets
    // the uop to r15 leaves r0..r14 equal and only the comparison of r15 can tell it apart.
    fn rd_pc(v: &mut [Uop]) {
        v.iter_mut().for_each(|u| u.rd = 15);
    }
    let (pc, word, seed) = (0x1000, 0xE59F_0004, 0x77);
    let v = LogBus::new(seed).peek32(pc + 12);
    assert_ne!(v, pc + 4, "the literal must not equal the next pc");
    let mut cpu = random_cpu(&mut Rng::new(0x77), pc);
    cpu.r[0] = v;
    let good = check_on(word, cpu.clone(), seed, true, unmutated);
    assert!(matches!(good, Ok(Some(s)) if s.folded), "{good:?}");
    let err = check_on(word, cpu, seed, true, rd_pc).expect_err("a write to the pc must show");
    assert!(err.contains("registers"), "{err}");
}

/// The semantic checks of this package as one oracle: the first divergence is the `Err`.
fn suite(rounds: u32) -> Result<(), String> {
    let mut rng = Rng::new(0x75);
    for round in 0..rounds {
        for class in 0..4 {
            let (word, pc) = word_of(&mut rng, class);
            for fold in [true, false].into_iter().take(1 + usize::from(class == 3)) {
                check(word, pc, &mut rng, fold, unmutated).map_err(|e| format!("{round}: {e}"))?;
            }
        }
        let (word, pc) = (wild(&mut rng), pick_pc(&mut rng));
        check(word, pc, &mut rng, round % 2 == 0, unmutated)
            .map_err(|e| format!("{round}: {e}"))?;
        // RRX needs the carry flag no memory uop has: it never lowers (mutant 124 lowers it, and
        // the model then refuses it)
        let (word, pc) = (rrx(&mut rng), pick_pc(&mut rng));
        let seen =
            check(word, pc, &mut rng, true, unmutated).map_err(|e| format!("{round}: {e}"))?;
        if seen.is_some() {
            return Err(format!("{round}: the RRX word {word:#010x} was lowered"));
        }
    }
    Ok(())
}

#[test]
fn every_mutant_of_this_package_is_killed_by_its_semantic_suite() {
    assert_eq!(suite(300), Ok(()), "the suite passes without a mutant");
    let mut ids: Vec<u32> = MUTANTS.iter().map(|m| m.0).collect();
    ids.dedup();
    assert!(ids.len() == MUTANTS.len() && ids.iter().all(|id| (100..=199).contains(id)));
    let survivors: Vec<&str> = MUTANTS
        .iter()
        .filter(|(id, _)| {
            // a mutant is killed by a divergence (`Err`) or by a failed assertion (a panic)
            match std::panic::catch_unwind(|| mutants::with(*id, || suite(300))) {
                Ok(outcome) => outcome.is_ok(),
                Err(_) => false,
            }
        })
        .map(|(_, name)| *name)
        .collect();
    assert!(survivors.is_empty(), "surviving mutants: {survivors:?}");
}
