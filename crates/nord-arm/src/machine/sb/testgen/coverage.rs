//! OWNER: W1-VGEN. The evidence that the structured generator exercises what it claims to (the
//! non-vacuity rule of design note 8): generated programs run long, reach their end, execute
//! every instruction class, enter Thumb, poll and message devices, rewrite their own page (so
//! decoded code goes stale) and react to hooks. Counted on the oracle, never on a tier under test.

use super::program::{machine_from, structured_program, FUNC_AT, SPIN_AT};
use super::CODE;
use crate::cpu::predecode::{decode_arm, op};
use crate::cpu::Fault;
use crate::machine::sb::testkit::{compare_runs, oracle_run, Rng, Run};
use crate::machine::{Hook, Machine, Stop};
use std::collections::BTreeMap;

/// Run `m` one instruction at a time, counting the decode class of every ARM word executed.
fn profile(m: &mut Machine, steps: u64) -> (BTreeMap<u8, u64>, bool, Option<Stop>) {
    let (mut classes, mut thumb, mut stop) = (BTreeMap::new(), false, None);
    for _ in 0..steps {
        let (pc, was_thumb) = (m.cpu.r[15], m.cpu.thumb);
        thumb |= was_thumb;
        if !was_thumb {
            let w = m.bus.mem.read32(pc);
            *classes
                .entry(decode_arm(w).op.min(op::DP_BASE))
                .or_insert(0) += 1;
        }
        match m.run(1, 0, None) {
            (_, Stop::Budget) => {}
            (_, other) => {
                stop = Some(other);
                break;
            }
        }
    }
    (classes, thumb, stop)
}

#[test]
fn generated_programs_run_long_and_cover_every_instruction_class_and_construct() {
    let mut classes: BTreeMap<u8, u64> = BTreeMap::new();
    let (mut thumb, mut spun, mut faulted, mut ran_long) = (0, 0, 0, 0);
    let (mut polled, mut messages, mut device) = (0, 0, 0);
    for seed in 0..80 {
        let p = structured_program(&mut Rng::new(seed), 40 + 2 * seed as usize);
        let mut m = machine_from(&p, false);
        let (c, t, stop) = profile(&mut m, 4000);
        c.iter()
            .for_each(|(k, n)| *classes.entry(*k).or_insert(0) += n);
        thumb += u32::from(t);
        spun += u32::from(m.cpu.r[15] == SPIN_AT);
        faulted += u32::from(matches!(stop, Some(Stop::Fault(_))));
        ran_long += u32::from(m.cpu.insns >= 100);
        polled += u32::from(m.bus.poll_hits > 0);
        messages += u32::from(!m.bus.messages.is_empty());
        device += u32::from(m.bus.device_reads > 0);
    }
    for (name, class) in [
        ("b", op::B),
        ("bl", op::BL),
        ("bx", op::BX),
        ("ldr", op::LDR),
        ("ldrb", op::LDRB),
        ("str", op::STR),
        ("strb", op::STRB),
        ("ldm", op::LDM),
        ("half", op::HALF),
        ("mul", op::MUL),
        ("dp", op::DP_BASE),
        ("slow", op::SLOW),
    ] {
        assert!(
            classes.get(&class).copied().unwrap_or(0) >= 100,
            "{name} executed {:?} times: {classes:?}",
            classes.get(&class)
        );
    }
    assert!(
        ran_long >= 70,
        "programs run long ({ran_long} of 80 retired 100 insns)"
    );
    assert!(
        spun >= 40,
        "most programs reach their end ({spun} of 80), so their loops end"
    );
    assert!(thumb >= 15, "Thumb entered in {thumb} programs");
    assert!(
        polled >= 3,
        "poll loops reached the poll breaker in {polled} programs"
    );
    assert!(messages >= 3, "DSP messages sent by {messages} programs");
    assert!(device >= 25, "device reads in {device} programs");
    assert!(
        (5..=40).contains(&faulted),
        "faults are the exception, but they happen ({faulted} of 80)"
    );
}

#[test]
fn stores_into_the_own_page_make_decoded_code_stale() {
    let (mut invalidated, mut changed) = (0, 0);
    for seed in 0..80 {
        let p = structured_program(&mut Rng::new(seed), 100);
        let (mut t1, mut t0) = (machine_from(&p, true), machine_from(&p, false));
        t1.run(3000, 0, None);
        t0.run(3000, 0, None);
        invalidated += u32::from(t1.code.invalidated > 0);
        changed += u32::from(
            t0.bus.mem.read_bytes(CODE, 0x400)
                != p.segment(CODE)
                    .iter()
                    .flat_map(|w| w.to_le_bytes())
                    .collect::<Vec<u8>>(),
        );
    }
    assert!(
        invalidated >= 5,
        "T1 dropped decoded pages in {invalidated} of 80 programs"
    );
    assert!(
        changed >= 5,
        "programs rewrote their own words in {changed} of 80 runs"
    );
}

#[test]
fn a_hook_set_in_the_program_changes_what_it_does() {
    let p = structured_program(&mut Rng::new(3), 80);
    let (mut plain, mut hooked) = (machine_from(&p, false), machine_from(&p, false));
    hooked.add_hook(FUNC_AT[0], Hook::Return(0x1234));
    hooked.add_hook(CODE + 4, Hook::SkipInsn);
    plain.run(2000, 0, None);
    hooked.run(2000, 0, None);
    assert_ne!(plain.state_hash(), hooked.state_hash());
}

#[test]
fn generated_programs_fault_now_and_then_with_every_kind_of_fault_word() {
    let (mut undefined, mut swi, mut wfi, mut late, mut faulted) = (0, 0, 0, 0, 0);
    let mut compared = 0;
    for seed in 0..200 {
        let p = structured_program(&mut Rng::new(seed), 40 + seed as usize % 100);
        let mut m = machine_from(&p, false);
        let (_, _, stop) = profile(&mut m, 4000);
        let Some(Stop::Fault(f)) = stop else {
            continue;
        };
        faulted += 1;
        late += u32::from(m.cpu.insns >= 20);
        match f {
            Fault::Undefined { .. } => undefined += 1,
            Fault::Swi { .. } => swi += 1,
            Fault::Wfi => wfi += 1,
        }
        // the faulting run is the same on tier 1: the fault is reported with the same state
        let want = oracle_run(&machine_from(&p, false), 4000, 0, None);
        let mut t1 = machine_from(&p, true);
        let res = t1.run(4000, 0, None);
        let got = Run::of(t1, res);
        compare_runs(&want, &got).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        compared += u32::from(matches!(want.why, Stop::Fault(g) if g == f));
    }
    assert!(
        (10..=70).contains(&faulted),
        "{faulted} of 200 programs faulted"
    );
    assert!(
        undefined >= 5 && swi >= 5 && wfi >= 5,
        "undefined {undefined}, swi {swi}, wfi {wfi}"
    );
    assert!(
        late >= 8,
        "{late} faults came after 20 instructions or more"
    );
    assert_eq!(
        compared, faulted,
        "every faulting run was compared on tier 1"
    );
}
