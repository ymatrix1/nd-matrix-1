//! Differential tests of the events around flow instructions: `IDLE`, an interrupt service ended
//! by `RTI`, the deadline re-check after a system-register load, the block set switch of a
//! `MODE1.PEYEN` write, and random programs whose branch targets all lie inside the program.
//!
//! OWNER: W2-FLOW. The same rig as `diff_chain.rs` (translator, real handlers, entry guard, next
//! to `Core::step`). An interrupt is taken by the interpreter (the deadline is 0); `irq.stall`
//! is a cumulative cycle count the `Window` row of the entry guard refuses blocks for, so the tests
//! that want the engine to run the service forget it on both cores. Each test asserts what ran
//! (P10).
use super::enc::{self, Asm, NOP, TRUE};
use super::harness::Tally;
use super::rig::Rig;
use super::{DO_COUNT, IDLE, JUMP, JUMP_I, RET, SYS_LOAD};
use crate::core::interrupts::{IRPTEN, VECTOR_PARCELS};
use crate::core::support::{load, CODE};
use crate::core::uop::verify::gen::{fill_state, Rng};
use crate::core::Core;

#[test]
fn idle_puts_the_core_to_sleep_in_both_and_the_run_goes_on_by_single_steps() {
    let mut a = Asm::default();
    a.nops(2);
    a.put(&enc::idle(false));
    a.nops(4);
    let mut rig = Rig::new(&a.0, |_| {});
    rig.run(8).unwrap();
    assert_eq!(rig.tally.calls(IDLE), 1);
    assert!(rig.a.is_idle() && rig.b.is_idle());
    assert!(
        rig.tally.stepped > 0,
        "an idle core is never let into a block"
    );
}

/// Interrupt vector table base of the scenarios and the vector that is raised. ASSUMED: any
/// vector and base inside the test memory do (`arm_interrupt` writes the vector word itself).
const IVT: u32 = 0x4000;
const VECTOR: u8 = 11;

/// A vector `VECTOR` that jumps to `isr`, interrupts enabled with `imask` and the interrupt
/// latched.
fn arm_interrupt(c: &mut Core, isr: u32, imask: u32) {
    c.set_ivt_base(IVT);
    let w = 0x063e_0000_0000u64 | u64::from(isr);
    let at = IVT + VECTOR_PARCELS * u32::from(VECTOR);
    load(c, at, &[w as u16, (w >> 16) as u16, (w >> 32) as u16]);
    c.mode1 |= IRPTEN;
    c.imask = imask;
    c.raise_interrupt(VECTOR);
}

#[test]
fn an_interrupt_service_ends_with_an_rti_run_by_the_engine() {
    let isr = CODE + 20;
    let mut a = Asm::default();
    a.nops(8);
    a.pad_to(isr);
    a.nops(2);
    a.put(&enc::ret_wide(TRUE, false, true));
    a.nops(4);
    let mut rig = Rig::new(&a.0, |c| arm_interrupt(c, isr, 1 << VECTOR));
    // The interpreter takes the interrupt (the deadline is 0) and runs the vector's jump.
    for _ in 0..4 {
        if rig.b.pc == isr {
            break;
        }
        rig.advance().unwrap();
    }
    assert_eq!(rig.b.pc, isr, "the vector jumped to the service routine");
    assert_eq!(rig.b.interrupts_taken(), 1);
    // `irq.stall` is a cumulative cycle count that stays non-zero after the first interrupt and
    // the `Window` row of the entry guard refuses every block while it is (reported to the
    // integrator, `guard.rs` is not this package's). Both cores forget it, so the engine is let in.
    rig.a.irq.stall = 0;
    rig.b.irq.stall = 0;
    for _ in 0..6 {
        if rig.tally.calls(RET) > 0 {
            break;
        }
        rig.advance().unwrap();
    }
    assert_eq!(rig.tally.calls(RET), 1, "the RTI ran as a handler");
    assert!(rig.b.irq.serviced.is_empty() && rig.b.return_addresses().is_empty());
    assert_eq!(
        rig.b.pc, CODE,
        "back at the instruction the interrupt preempted"
    );
    assert!(rig.tally.blocks >= 1, "{:?}", rig.tally);
}

#[test]
fn a_system_register_load_that_unmasks_an_interrupt_ends_the_block_and_the_next_step_takes_it() {
    let isr = CODE + 24;
    let mut a = Asm::default();
    a.nops(2);
    a.put(&enc::load_imm(0x7B, 1 << VECTOR)); // IMASK
    a.nops(4);
    a.pad_to(isr);
    a.put(&enc::ret_wide(TRUE, false, true));
    a.nops(4);
    let mut rig = Rig::new(&a.0, |c| arm_interrupt(c, isr, 0));
    rig.run(16).unwrap();
    assert_eq!(rig.tally.calls(SYS_LOAD), 1, "{:?}", rig.tally);
    assert_eq!(
        rig.b.interrupts_taken(),
        1,
        "the unmasked interrupt was taken"
    );
    assert!(
        rig.b.irq.serviced.is_empty(),
        "and its service ended with the RTI"
    );
}

#[test]
fn a_system_register_load_that_enables_pey_switches_the_block_set_of_the_next_entry() {
    let mut a = Asm::default();
    a.nops(2);
    a.put(&enc::load_imm(0x72, 1 << 21)); // MODE1.PEYEN
    a.nops(4);
    let mut rig = Rig::new(&a.0, |_| {});
    rig.run(12).unwrap();
    assert!(rig.b.simd() && rig.a.simd());
    assert_eq!(rig.tally.calls(SYS_LOAD), 1);
    assert!(
        rig.tally.blocks >= 2,
        "the PEy-on blocks took over: {:?}",
        rig.tally
    );
}

/// A random program whose branch targets, loop ends and indirect registers all point at the start
/// of one of its own instructions. Returns the parcels and the start pc of every slot.
fn random_program(rng: &mut Rng) -> (Vec<u16>, Vec<u32>) {
    const SLOTS: usize = 16;
    // kind of each slot: 0 NOP, 1 jump, 2 call, 3 jump (m,i), 4 rts, 5 do, 6 do until, 7 sys load
    let kinds: Vec<u32> = (0..SLOTS)
        .map(|_| rng.pick(&[0, 0, 0, 0, 1, 1, 2, 3, 4, 5, 5, 6, 7]))
        .collect();
    let len = |k: u32, delayed: bool| match k {
        1 | 2 | 5 | 6 => 3,
        3 => 2,
        7 => 2,
        _ => 1,
    } + if delayed && matches!(k, 1..=4) { 2 } else { 0 };
    let delayed: Vec<bool> = kinds.iter().map(|_| rng.below(3) == 0).collect();
    let mut starts = Vec::new();
    let mut pc = CODE;
    for (k, d) in kinds.iter().zip(&delayed) {
        starts.push(pc);
        pc += len(*k, *d);
    }
    let target = |rng: &mut Rng| starts[rng.below(SLOTS as u32) as usize];
    let mut p = Vec::new();
    for (i, (k, d)) in kinds.iter().zip(&delayed).enumerate() {
        let cond = if rng.below(2) == 0 {
            TRUE
        } else {
            rng.below(32) as u8
        };
        let ahead = starts[(i + 1 + rng.below(3) as usize).min(SLOTS - 1)];
        let body = match k {
            1 | 2 => enc::jump(cond, target(rng), *k == 2, *d),
            3 => enc::jump_i(
                cond,
                8 + rng.below(8) as u8,
                8 + rng.below(8) as u8,
                false,
                *d,
            ),
            4 => enc::rts(cond, *d, false),
            5 => enc::do_count(
                1 + rng.below(3) as u16,
                ahead.max(starts[i] + 3) - starts[i],
            ),
            6 => enc::do_until(cond, ahead.max(starts[i] + 3) - starts[i]),
            7 => enc::load_imm(rng.pick(&[0x6A, 0x72, 0x73, 0x7B]), rng.below(2) << 21),
            _ => vec![NOP],
        };
        p.extend(&body);
        if *d && matches!(k, 1..=4) {
            p.extend([NOP, NOP]);
        }
    }
    p.extend([NOP; 8]);
    (p, starts)
}

#[test]
fn random_programs_with_every_target_inside_the_program_match_the_interpreter() {
    let mut total = Tally::default();
    for seed in 1..=24u64 {
        let (program, starts) = random_program(&mut Rng::new(seed * 0x9E37));
        for simd in [false, true] {
            let mut rig = Rig::new(&program, |c| {
                let mut rng = Rng::new(seed);
                fill_state(c, &mut rng, simd);
                for k in 8..16 {
                    c.m[k] = rng.below(4);
                    c.i[k] = starts[rng.below(starts.len() as u32) as usize] - c.m[k];
                }
                for _ in 0..rng.below(4) {
                    c.push_return_address(starts[rng.below(starts.len() as u32) as usize]);
                }
            });
            rig.run(120)
                .unwrap_or_else(|e| panic!("seed {seed} simd {simd} {program:04x?}: {e}"));
            total.merge(&rig.tally);
        }
    }
    for k in [JUMP, JUMP_I, RET, DO_COUNT, SYS_LOAD] {
        assert!(total.calls(k) > 0, "{k:?} never ran: {total:?}");
    }
    assert!(
        total.blocks > 100 && total.fin > 10 && total.stepped > 0,
        "{total:?}"
    );
}
