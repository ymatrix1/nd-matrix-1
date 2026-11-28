//! OWNER: W1-VGEN (scaffold written by S0; W1-VGEN extends it and keeps every signature).
//! Generators and the "op through lowering versus interpreter" helpers every work package calls
//! from its own tests.
//!
//! Spec (design note 8, WP W1-VGEN): `cfg(test)` support only.
//!
//! * [`random_program`]: the L1 generator. Mirrors the private `word()` of the `fastrun.rs` test
//!   (those test items cannot be reused without editing a file the skeleton may not touch);
//!   W1-VGEN extends it with `bl`/`bx lr` pairs, literal loads, stores into the own, next and
//!   literal page, hooks between slices, a Thumb entry and undefined words.
//! * [`lowered_vs_step`] / [`uops_vs_step`]: one instruction (or a hand-built trace for it)
//!   through the lowering and the T2 executor against `Cpu::step`, with registers, flags, fault,
//!   the ordered bus access log and `bus.pc` compared. This is the helper of the lowering and
//!   executor work packages (W1-DP, W1-MEM, W1-XALU, W1-XMEM, W2-XCORE) and of L0 in `difftest`.
//! * [`drive_traces`]: the tier stack without a cache, as a copy of the outer loop of
//!   `Machine::run` that builds a trace per pc and runs it (the skeleton proof that the
//!   always-correct baseline equals the interpreter).
//!
//! `#![allow(dead_code)]`: consumed by the tests of the work packages named above.

#![allow(dead_code)]

use super::config::SbConfig;
use super::decode::{decode, lower_or_generic, LowerCtx};
use super::exec::run_sb;
use super::lower_flow::{lower_flow, Flow, FlowCtx};
use super::testkit::{CpuSnap, LogBus, Rng};
use super::trace::{build, BuildOpts};
use super::uop::{k, Exit, ExitKind, Sb, Uop};
use crate::cpu::Cpu;
use crate::machine::{Machine, Stop};
use crate::mem::HOOK_X;

/// Base address of generated programs.
pub(crate) const CODE: u32 = 0xC000_0000;

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

/// A machine with `words` at [`CODE`], random `r0..r7` from `seed`, `r8`/`r9` pointing at data,
/// `r10` into the code itself, a stack, and `pc = CODE`; `predecode` selects tier 1 or the plain
/// interpreter.
pub(crate) fn machine_with(words: &[u32], seed: u64, predecode: bool) -> Machine {
    let mut rng = Rng::new(seed);
    let mut m = Machine::new();
    m.predecode = predecode;
    for (i, w) in words.iter().enumerate() {
        m.bus.mem.write32(CODE + 4 * i as u32, *w);
    }
    for reg in m.cpu.r.iter_mut().take(8) {
        *reg = rng.next();
    }
    (m.cpu.r[8], m.cpu.r[9], m.cpu.r[10], m.cpu.r[13]) =
        (0x9000_0000, 0x9000_1000, CODE + 0x100, 0x9000_2000);
    m.cpu.r[14] = CODE;
    m.cpu.r[15] = CODE;
    m
}

/// Lower the one instruction `word` at `pc` into a complete trace, the way the trace builder
/// does for a one-instruction trace: control flow through `lower_flow`, everything else through
/// the static registry with the early exits allocated. `peek` serves literal folding.
pub(crate) fn lower_single(word: u32, pc: u32, peek: &dyn Fn(u32) -> u32) -> Sb {
    let insn = decode(pc, word);
    let opts = BuildOpts::from(&SbConfig::on());
    let (mut uops, mut exits): (Vec<Uop>, Vec<Exit>) = (Vec::new(), Vec::new());
    let next = if insn.is_ender() {
        let (mut depth, mut rets) = (0u8, Vec::new());
        let can_follow = |_: u32| false;
        match lower_flow(&mut FlowCtx {
            insn,
            uops: &mut uops,
            exits: &mut exits,
            done: 0,
            taken_hint: None,
            can_follow: &can_follow,
            depth: &mut depth,
            rets: &mut rets,
            entry_pc: pc,
            opts: &opts,
        }) {
            Flow::End => None,
            Flow::Follow(next) => Some(next),
        }
    } else {
        let mut fold = |a: u32| Some(peek(a));
        lower_or_generic(&mut LowerCtx {
            insn,
            out: &mut uops,
            fold: &mut fold,
            opts: &opts,
        });
        for u in uops.iter_mut() {
            if k::is_store(u.kind) || u.kind == k::GENERIC {
                exits.push(Exit::new(ExitKind::Stale, 1, pc, pc.wrapping_add(4)));
                u.set_exit_idx(exits.len() as u8 - 1);
            }
        }
        Some(pc.wrapping_add(4))
    };
    if let Some(next) = next {
        exits.push(Exit::new(ExitKind::Direct, 1, pc, next));
        uops.push(Uop::exit(exits.len() as u8 - 1, pc));
    }
    Sb::from_parts(pc, 1, uops, exits, vec![(pc, word)]).expect("a one-instruction trace fits")
}

/// Run `sb` (a trace of the one instruction `word` at `cpu.r[15]`) and `Cpu::step` on equal
/// states; `Err` names the first difference in registers, flags, fault, bus access log or `pc`.
pub(crate) fn uops_vs_step(word: u32, sb: &Sb, cpu: &Cpu, seed: u64) -> Result<(), String> {
    let pc = cpu.r[15];
    let (mut ca, mut ba) = (cpu.clone(), LogBus::new(seed as u32));
    let (mut cb, mut bb) = (cpu.clone(), LogBus::new(seed as u32));
    ba.poke32(pc, word);
    bb.poke32(pc, word);
    ba.pc = pc;
    let ra = ca.step(&mut ba).err();
    let mut left = 4u64;
    run_sb(&mut cb, &mut bb, sb, &mut left, false);
    let rb = cb.fault.take();
    let (sa, sbn) = (CpuSnap::of(&ca, ra), CpuSnap::of(&cb, rb));
    if let Some(d) = sa.diff(&sbn) {
        return Err(format!("word {word:#010x} at {pc:#x}: {d}"));
    }
    if ba.log != bb.log {
        return Err(format!(
            "word {word:#010x}: access logs differ\n step: {:x?}\n sb:   {:x?}",
            ba.log, bb.log
        ));
    }
    if (ba.pc, left) != (bb.pc, 3) {
        return Err(format!(
            "word {word:#010x}: bus.pc {:#x} vs {:#x}, left {left}",
            ba.pc, bb.pc
        ));
    }
    Ok(())
}

/// The lowering of `word` at `cpu.r[15]` against the interpreter ([`lower_single`] then
/// [`uops_vs_step`]). The helper of every lowering work package.
pub(crate) fn lowered_vs_step(word: u32, cpu: &Cpu, seed: u64) -> Result<(), String> {
    let probe = LogBus::new(seed as u32);
    let sb = lower_single(word, cpu.r[15], &|a| probe.peek32(a));
    uops_vs_step(word, &sb, cpu, seed)
}

/// `Machine::run` with the tier stack and no cache: at every pc build a trace (`trace::build`) and
/// run it (`exec::run_sb`); Thumb, hooked, misaligned and unmapped pcs take one step of the real
/// `Machine::run`. Same contract and result as `Machine::run`.
pub(crate) fn drive_traces(
    m: &mut Machine,
    max: u64,
    until: u32,
    stop_pc: Option<u32>,
) -> (u64, Stop) {
    let start = m.cpu.insns;
    let limit = start + max;
    let stop = stop_pc.unwrap_or(u32::MAX);
    let opts = BuildOpts::from(&SbConfig::on());
    let reason = loop {
        if m.cpu.insns >= limit {
            break Stop::Budget;
        }
        let pc = m.cpu.r[15];
        if pc == until {
            break Stop::Until;
        }
        if pc == stop {
            break Stop::StopPc;
        }
        let sb = if m.cpu.thumb || m.bus.mem.flags(pc) & HOOK_X != 0 {
            None
        } else {
            build(&mut m.bus.mem, pc, &opts)
        };
        match sb {
            Some(sb) => {
                let mut left = limit - m.cpu.insns;
                run_sb(&mut m.cpu, &mut m.bus, &sb, &mut left, false);
                if let Some(f) = m.cpu.fault.take() {
                    break Stop::Fault(f);
                }
            }
            None => match m.run(1, until, stop_pc) {
                (_, Stop::Budget) => {}
                (_, other) => break other,
            },
        }
    };
    (m.cpu.insns - start, reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::predecode::{decode_arm, op};
    use crate::machine::sb::testkit::random_cpu;

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

    #[test]
    fn a_machine_starts_at_the_code_with_data_pointers_and_is_reproducible() {
        let words = random_program(&mut Rng::new(9), 64);
        let (a, b) = (machine_with(&words, 5, true), machine_with(&words, 5, true));
        assert_eq!(
            (a.cpu.r[15], a.cpu.r[8], a.bus.mem.clone().read32(CODE + 8)),
            (CODE, 0x9000_0000, words[2])
        );
        assert_eq!(a.cpu.r, b.cpu.r);
        assert_ne!(a.cpu.r[0], machine_with(&words, 6, true).cpu.r[0]);
    }

    #[test]
    fn the_skeleton_lowering_equals_the_interpreter_on_random_words_and_states() {
        let mut rng = Rng::new(0xA11CE);
        for i in 0..3000 {
            let word = random_word(&mut rng, 40, 400);
            let word = if i % 3 == 0 {
                word & 0x0FFF_FFFF | 0xE000_0000
            } else {
                word
            };
            let cpu = random_cpu(&mut rng, 0x1000);
            lowered_vs_step(word, &cpu, i).unwrap();
        }
    }

    #[test]
    fn the_comparator_catches_a_trace_that_does_something_else() {
        let mut rng = Rng::new(5);
        let cpu = random_cpu(&mut rng, 0x1000);
        let wrong = lower_single(0xE3A0_0006, 0x1000, &|_| 0); // mov r0,#6 ...
        let err = uops_vs_step(0xE3A0_0005, &wrong, &cpu, 1).unwrap_err(); // ... compared to mov r0,#5
        assert!(err.contains("r0"), "{err}");
        let nothing = Sb::from_parts(
            0x1000,
            1,
            vec![Uop::exit(0, 0x1000)],
            vec![Exit::new(ExitKind::Direct, 1, 0x1000, 0x1004)],
            vec![(0x1000, 0xE3A0_0005)],
        )
        .unwrap();
        assert!(uops_vs_step(0xE3A0_0005, &nothing, &cpu, 1).is_err());
    }

    #[test]
    fn a_conditional_and_a_branch_lower_to_something_that_runs() {
        let mut rng = Rng::new(6);
        let cpu = random_cpu(&mut rng, 0x1000);
        for word in [
            0x1280_0001,
            0xEAFF_FFFE,
            0xE12F_FF10,
            0xE591_F000,
            0xE582_1000,
        ] {
            lowered_vs_step(word, &cpu, 3).unwrap();
        }
    }
}
