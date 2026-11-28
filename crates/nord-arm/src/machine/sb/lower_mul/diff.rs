//! Layer L0 for the multiply lowering (WP W2-MUL): random `mul`/`mla`/long-multiply words
//! through the registry and the executor against the interpreter (`Cpu::step`), the field table
//! checked word by word, a run that proves it exercised every kind and every outcome, the
//! mutation kill test, and one trace run through `run_sb`.
//!
//! The model is written from the field table of the module doc, not from `plan`: a word is
//! lowered iff its opcode field (bits 23..21) is 0 or 1 and none of `Rd`, bits 15..12, `Rs`, `Rm`
//! is `r15`; the kind is `MUL + acc * 2 + s`.

use super::{lower_mul, Lowered, MUTANT_SITES};
use crate::cpu::Cpu;
use crate::machine::sb::decode::{decode, lower_or_generic, LowerCtx};
use crate::machine::sb::exec::run_sb;
use crate::machine::sb::exec_alu::exec_dp;
use crate::machine::sb::mutants;
use crate::machine::sb::testkit::{random_cpu, LogBus, Rng};
use crate::machine::sb::trace::BuildOpts;
use crate::machine::sb::uop::{k, pack_flags, Exit, ExitKind, Sb, Uop};

const PC0: u32 = 0x2000;

/// What a run proved it did.
#[derive(Debug, Default)]
struct Tally {
    /// Words lowered to a uop, by `k::MUL..=k::MLAS`.
    per_kind: [u32; 4],
    /// Words that stayed `GENERIC`, by cause: long multiply, pc operand.
    long: u32,
    pc: u32,
    /// Executed cases whose destination changed, and flag states seen after the instruction.
    rd_changed: u32,
    flags_seen: u16,
    /// Cases where the destination register aliases a source.
    aliased: u32,
}

/// The word of the fields `(opcode, s, hi = Rd, lo = bits 15..12, rs, rm)`.
fn word(opcode: u32, s: bool, f: [u32; 4]) -> u32 {
    let [hi, lo, rs, rm] = f;
    0xE000_0090 | opcode << 21 | u32::from(s) << 20 | hi << 16 | lo << 12 | rs << 8 | rm
}

/// Lower `w` with `lower_mul` alone.
fn direct(w: u32) -> (Lowered, Vec<Uop>) {
    let opts = BuildOpts::default();
    let (mut out, mut fold) = (Vec::new(), |_: u32| None);
    let mut cx = LowerCtx {
        insn: decode(PC0, w),
        out: &mut out,
        fold: &mut fold,
        opts: &opts,
    };
    let l = lower_mul(&mut cx);
    (l, out)
}

/// Lower `w` through the registry (`lower_or_generic`).
fn registry(w: u32) -> (Lowered, Vec<Uop>) {
    let opts = BuildOpts::default();
    let (mut out, mut fold) = (Vec::new(), |_: u32| None);
    let mut cx = LowerCtx {
        insn: decode(PC0, w),
        out: &mut out,
        fold: &mut fold,
        opts: &opts,
    };
    let l = lower_or_generic(&mut cx);
    (l, out)
}

/// One random field set: registers aliased a quarter of the time each, `r15` about 1 in 16.
fn fields(rng: &mut Rng) -> [u32; 4] {
    let mut f = [0; 4];
    f.iter_mut().for_each(|v| *v = rng.below(16));
    match rng.below(5) {
        0 => f[3] = f[0], // rm == rd
        1 => f[2] = f[0], // rs == rd
        2 => f[1] = f[0], // acc == rd
        3 => f[2] = f[3], // rs == rm
        _ => {}
    }
    f
}

/// `n` random words (70 % `mul`/`mla`, the rest long multiplies and the undefined opcodes) each
/// checked against the model and, when lowered, against the interpreter. `Err` names the first
/// difference.
fn check_all(seed: u64, n: u32) -> Result<Tally, String> {
    let (mut rng, mut t) = (Rng::new(seed), Tally::default());
    for _ in 0..n {
        let opcode = if rng.below(10) < 7 {
            rng.below(2)
        } else {
            rng.range(2, 8)
        };
        let (s, f) = (rng.flag(), fields(&mut rng));
        let w = word(opcode, s, f);
        let [hi, lo, rs, rm] = f;
        let lowered = opcode <= 1 && !f.contains(&15);
        let ctx = format!("{w:#010x}");
        // `lower_mul` alone and the registry must agree with the model
        let (l, out) = direct(w);
        let want_l = if lowered {
            Lowered::Done
        } else {
            Lowered::Generic
        };
        if (l, out.len()) != (want_l, usize::from(lowered)) {
            return Err(format!(
                "{ctx}: lower_mul gave {l:?} with {} uops",
                out.len()
            ));
        }
        let (rl, rout) = registry(w);
        if !lowered {
            if (rl, &rout[..]) != (Lowered::Generic, &[Uop::generic(w, PC0, 0)][..]) {
                return Err(format!("{ctx}: registry did not fall back to GENERIC"));
            }
            t.long += u32::from(opcode > 1);
            t.pc += u32::from(opcode <= 1);
            continue;
        }
        let acc = opcode == 1;
        let mut want = Uop::new(k::MUL + u8::from(acc) * 2 + u8::from(s), PC0);
        (want.rd, want.rs, want.rm) = (hi as u8, rs as u8, rm as u8);
        want.rn = if acc { lo as u8 } else { 0 };
        if (rl, &rout[..]) != (Lowered::Done, &[want][..]) || out != rout {
            return Err(format!("{ctx}: uop {rout:?}, want {want:?}"));
        }
        t.per_kind[(want.kind - k::MUL) as usize] += 1;
        t.aliased += u32::from(hi == rm || hi == rs || (acc && hi == lo));
        // the interpreter
        let mut cpu = random_cpu(&mut rng, PC0);
        let before = cpu.clone();
        let mut bus = LogBus::new(3);
        bus.poke32(PC0, w);
        cpu.step(&mut bus).map_err(|e| format!("{ctx}: {e:?}"))?;
        // the executor, on the same registers and flags
        let (mut r, mut fl) = (before.r, pack_flags(&before));
        exec_dp(&mut r, &mut fl, &rout[0]);
        if r[..15] != cpu.r[..15] || fl != cpu.nzcv() as u8 {
            return Err(format!(
                "{ctx}: r {:x?} f {fl:04b}, interpreter r {:x?} f {:04b}",
                &r[..15],
                &cpu.r[..15],
                cpu.nzcv()
            ));
        }
        if (cpu.r[15], cpu.insns) != (PC0 + 4, 1) {
            return Err(format!("{ctx}: the interpreter did not retire one insn"));
        }
        t.rd_changed += u32::from(r[hi as usize] != before.r[hi as usize]);
        t.flags_seen |= 1 << fl;
    }
    Ok(t)
}

#[test]
fn lowered_multiplies_equal_the_interpreter_and_the_run_was_not_vacuous() {
    let t = check_all(0x4D55_4C00, 6000).unwrap();
    assert!(t.per_kind.iter().all(|n| *n > 600), "{t:?}");
    assert!(t.long > 300 && t.pc > 100, "declined words seen: {t:?}");
    assert!(t.rd_changed > 2000 && t.aliased > 600, "{t:?}");
    // N set/clear and Z set/clear were both observed with S (C and V ride along as inputs)
    let flags = (0..16u16).filter(|f| t.flags_seen >> f & 1 != 0).count();
    assert!(flags >= 14, "{flags} of 16 flag nibbles seen: {t:?}");
}

#[test]
fn every_flag_outcome_of_the_s_forms_is_hit_on_edge_operands() {
    // (rm, rs, acc) edge triples: result zero, negative, positive, wrapping to zero, all flag
    // nibbles as inputs (C and V must survive, N and Z follow the result)
    let edges = [
        0u32,
        1,
        2,
        0xFFFF,
        0x1_0000,
        0x7FFF_FFFF,
        0x8000_0000,
        u32::MAX,
    ];
    let (mut seen_nz, mut cases) = (0u8, 0u32);
    for acc in [false, true] {
        for &a in &edges {
            for &b in &edges {
                for &c in &edges {
                    for nzcv in 0..16u32 {
                        let w = word(u32::from(acc), true, [1, 4, 3, 2]); // r1 = r2 * r3 (+ r4)
                        let (l, out) = registry(w);
                        assert_eq!(l, Lowered::Done);
                        let mut cpu = Cpu::new();
                        cpu.r[2] = a;
                        cpu.r[3] = b;
                        cpu.r[4] = c;
                        cpu.r[15] = PC0;
                        cpu.set_cpsr(nzcv << 28 | 0x13);
                        let before = cpu.clone();
                        let mut bus = LogBus::new(4);
                        bus.poke32(PC0, w);
                        cpu.step(&mut bus).unwrap();
                        let (mut r, mut f) = (before.r, pack_flags(&before));
                        exec_dp(&mut r, &mut f, &out[0]);
                        assert_eq!(
                            (r[..15].to_vec(), f),
                            (cpu.r[..15].to_vec(), cpu.nzcv() as u8)
                        );
                        assert_eq!(f & 3, nzcv as u8 & 3, "C and V are inputs, untouched");
                        seen_nz |= 1 << (f >> 2);
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 2 * 8 * 8 * 8 * 16);
    assert_eq!(
        seen_nz, 0b0111,
        "(N, Z) seen as 00, 01 and 10; 11 cannot happen"
    );
}

#[test]
fn every_mutant_site_is_killed() {
    assert!(check_all(1, 1500).is_ok(), "the unmutated oracle passes");
    let mut ids: Vec<u32> = MUTANT_SITES.iter().map(|m| m.0).collect();
    for &(id, name) in MUTANT_SITES {
        assert!(
            (900..950).contains(&id),
            "{name}: id {id} outside the block"
        );
        assert!(
            mutants::with(id, || check_all(1, 1500)).is_err(),
            "mutant {id} {name} survived"
        );
    }
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), MUTANT_SITES.len(), "ids are unique");
}

#[test]
fn a_lowered_trace_of_multiplies_runs_through_run_sb_like_the_interpreter() {
    let words = [
        0xE001_0092u32, // mul r1,r2,r0
        0xE034_3192,    // mlas r4,r2,r1,r3
        0xE015_0495,    // muls r5,r5,r4
        0xE020_6094,    // mla r0,r4,r0,r6
    ];
    let mut uops = Vec::new();
    for (i, w) in words.iter().enumerate() {
        let opts = BuildOpts::default();
        let mut fold = |_: u32| None;
        let mut cx = LowerCtx {
            insn: decode(PC0 + 4 * i as u32, *w),
            out: &mut uops,
            fold: &mut fold,
            opts: &opts,
        };
        assert_eq!(lower_or_generic(&mut cx), Lowered::Done, "{w:#010x}");
    }
    assert_eq!(
        uops.iter().map(|u| u.kind).collect::<Vec<_>>(),
        [k::MUL, k::MLAS, k::MULS, k::MLA]
    );
    uops.push(Uop::exit(0, PC0 + 16));
    let exits = vec![Exit::new(ExitKind::Direct, 4, PC0 + 12, PC0 + 16)];
    let sb = Sb::from_parts(
        PC0,
        4,
        uops,
        exits,
        words
            .iter()
            .zip(0u32..)
            .map(|(w, i)| (PC0 + 4 * i, *w))
            .collect(),
    )
    .unwrap();
    let mut rng = Rng::new(0x5B);
    let mut hit = 0u32;
    for _ in 0..400 {
        let mut a = random_cpu(&mut rng, PC0);
        let mut b = a.clone();
        let (mut ba, mut bb) = (LogBus::new(5), LogBus::new(5));
        ba.load_words(PC0, &words);
        bb.load_words(PC0, &words);
        for _ in 0..4 {
            a.step(&mut ba).unwrap();
        }
        let mut left = 4u64;
        let out = run_sb(&mut b, &mut bb, &sb, &mut left, false);
        assert_eq!((out.exit, out.target), (0, PC0 + 16));
        assert_eq!((b.r, b.nzcv(), b.insns, left), (a.r, a.nzcv(), a.insns, 0));
        hit += u32::from(b.n || b.z);
    }
    assert!(
        hit > 100,
        "the flag-setting forms set N or Z in {hit} of 400 runs"
    );
}
