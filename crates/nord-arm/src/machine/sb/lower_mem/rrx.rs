//! OWNER: W1-MEM (tests). The register offsets whose carry input matters or does not.
//!
//! `ROR #0` is RRX: the offset is `C << 31 | rm >> 1`, so the interpreter's address depends on the
//! carry flag, and no memory uop (nor `exec_mem`) has a carry input. `lower_mem` therefore leaves
//! it to `GENERIC`. `LSR #0` and `ASR #0` mean a shift by 32, whose result does not depend on the
//! carry (only its carry-out would), so they stay lowered. Both halves are tested against
//! `Cpu::step`, and each proves it exercised something.

use super::semantics::{check_on, unmutated};
use super::words::{PC, RRX};
use crate::machine::sb::decode::{decode, lower_or_generic, LowerCtx, Lowered};
use crate::machine::sb::testkit::{random_cpu_flags, LogBus, Rng};
use crate::machine::sb::trace::BuildOpts;
use crate::machine::sb::uop::Uop;

/// `ldr/str/ldrb/strb r0,[r1,r2,lsr #0]` and `asr #0` in every addressing mode (pre-indexed,
/// write-back, post-indexed), the shifts that mean 32.
const SHIFT32: [u32; 10] = [
    0xE791_0022, // ldr r0,[r1,r2,lsr #0]
    0xE791_0042, // ldr r0,[r1,r2,asr #0]
    0xE781_0022, // str r0,[r1,r2,lsr #0]
    0xE781_0042, // str r0,[r1,r2,asr #0]
    0xE7D1_0022, // ldrb r0,[r1,r2,lsr #0]
    0xE7C1_0042, // strb r0,[r1,r2,asr #0]
    0xE7B1_0022, // ldr r0,[r1,r2,lsr #0]!
    0xE7A1_0042, // str r0,[r1,r2,asr #0]!
    0xE691_0042, // ldr r0,[r1],r2,asr #0
    0xE681_0022, // str r0,[r1],r2,lsr #0
];

#[test]
fn rrx_register_offsets_lower_to_one_generic_uop_through_the_registry() {
    for (word, why) in RRX {
        let (mut out, mut fold, opts) = (Vec::new(), |_: u32| None, BuildOpts::default());
        let lowered = lower_or_generic(&mut LowerCtx {
            insn: decode(PC, word),
            out: &mut out,
            fold: &mut fold,
            opts: &opts,
        });
        assert_eq!(lowered, Lowered::Generic, "{why}");
        assert_eq!(out, vec![Uop::generic(word, PC, 0)], "{why}");
    }
}

/// `r0..r15` and the ordered `(kind, address, value, pc)` access log of a run.
type Outcome = ([u32; 16], Vec<(u8, u32, u32, u32)>);

/// Registers and ordered access log after the interpreter runs `word` once with carry `c`, on a
/// state that is otherwise fixed: `r1 = 0x2000`, `r2 = 0x10` (so RRX gives `0x8`, or `0x8000_0008`).
fn interpret(word: u32, c: bool) -> Outcome {
    let mut cpu = random_cpu_flags(&mut Rng::new(0x5A), PC, if c { 2 } else { 0 });
    (cpu.r[0], cpu.r[1], cpu.r[2]) = (0x55, 0x2000, 0x10);
    let mut bus = LogBus::new(9);
    bus.poke32(PC, word);
    bus.pc = PC;
    cpu.step(&mut bus).expect("a plain transfer does not fault");
    (cpu.r, bus.log)
}

#[test]
fn the_interpreter_result_of_rrx_depends_on_the_carry_so_it_cannot_lower_without_one() {
    for (word, why) in RRX {
        let (without, with) = (interpret(word, false), interpret(word, true));
        assert_ne!(
            without, with,
            "{why}: registers or access log must differ with the carry"
        );
    }
    // ldr r0,[r1,r2,rrx]! and the post-indexed form: r1 moves by 0x8, or by 0x8000_0008
    for word in [0xE7B1_0062, 0xE691_0062] {
        let (without, with) = (interpret(word, false), interpret(word, true));
        assert_eq!(
            (without.0[1], with.0[1]),
            (0x2008, 0x8000_2008),
            "{word:#010x}"
        );
        assert_ne!(without.0[1], with.0[1]);
    }
    // a pre-indexed access without write-back moves its address instead
    let (without, with) = (interpret(0xE781_0062, false), interpret(0xE781_0062, true));
    assert_eq!((without.1[0].1, with.1[0].1), (0x2008, 0x8000_2008));
}

#[test]
fn lsr_and_asr_by_zero_equal_the_interpreter_for_every_flag_state() {
    let mut rng = Rng::new(0x78);
    let (mut cases, mut lowered, mut loads, mut stores, mut wrote_back) = (0, 0, 0, 0, 0);
    for word in SHIFT32 {
        for nzcv in 0..16 {
            for _ in 0..4 {
                let (seed, cpu) = (rng.next(), random_cpu_flags(&mut rng, PC, nzcv));
                let seen = check_on(word, cpu, seed, true, unmutated)
                    .unwrap_or_else(|e| panic!("nzcv {nzcv}: {e}"));
                cases += 1;
                if let Some(s) = seen {
                    lowered += 1;
                    loads += u32::from(s.loads > 0);
                    stores += u32::from(s.stores > 0);
                    wrote_back += u32::from(s.wrote_back);
                }
            }
        }
    }
    // every case lowered (LSR/ASR #0 are not refused), both directions happened, and so did
    // write-back, under all 16 flag states (so under both values of the carry)
    assert_eq!((cases, lowered), (640, 640));
    assert_eq!((loads, stores), (320, 320));
    assert_eq!(
        wrote_back,
        4 * 16 * 4,
        "four of the ten words write the base back"
    );
}
