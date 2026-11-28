//! OWNER: W1-FLOW (tests). Differential tests of trace formation where the code meets its
//! surroundings: several pages and the page cap, hooked and unmapped pages, a store into the
//! running trace, and random programs on every option set. The helpers are those of `diff.rs`.

use super::super::testgen::{random_program, CODE};
use super::super::testkit::{fuzz_cases, fuzz_seed, Rng};
use super::super::uop::{k, ExitKind};
use super::diff::{machine, option_sets, probe, put, same, self_modifying, sweep};
use super::shape::{add, b, bl, mov};
use super::testexec::{drive, Tally};
use super::BuildOpts;
use crate::machine::{Hook, Machine};

#[test]
fn a_trace_over_three_pages_loops_back_to_its_entry_and_the_page_cap_splits_it() {
    let pages = [CODE, CODE + 0x1000, CODE + 0x2000];
    let setup = |m: &mut Machine| {
        put(m, pages[0], &[add(0, 1), b(pages[0] + 4, pages[1])]);
        put(m, pages[1], &[add(0, 2), b(pages[1] + 4, pages[2])]);
        put(m, pages[2], &[add(0, 4), b(pages[2] + 4, pages[0])]);
    };
    for cap in 1..=4usize {
        let mut tally = Tally::default();
        let opts = BuildOpts {
            max_pages: cap,
            ..BuildOpts::default()
        };
        for budget in (1..=20).chain([100, 333]) {
            same("pages", &setup, 3, budget, &opts, &[], &mut tally);
        }
        assert_eq!(tally.laps > 0, cap >= 3, "cap {cap}: {tally:?}");
        assert!(tally.traces > 0, "cap {cap}: {tally:?}");
    }
}

#[test]
fn hooked_and_unmapped_code_ends_the_trace_in_front_of_it_and_runs_as_the_interpreter_does() {
    let (hooked, end) = (CODE + 0x2000, CODE + 0xFF8);
    // a call into a stub, a fall-through into a hooked page
    let stubs = |m: &mut Machine| {
        put(
            m,
            CODE,
            &[
                mov(1, 1),
                bl(CODE + 4, hooked),
                mov(2, 2),
                b(CODE + 12, end),
            ],
        );
        put(m, end, &[mov(6, 6), mov(7, 7)]);
        m.add_hook(hooked, Hook::Return(7));
        m.add_hook(CODE + 0x1000, Hook::Return(3));
    };
    // a jump into a page nothing mapped (the interpreter maps it, the builder never does)
    let unmapped = |m: &mut Machine| {
        put(m, CODE, &[mov(1, 1), b(CODE + 4, CODE + 0x5000)]);
    };
    let mut tally = Tally::default();
    for (setup, seed) in [(&stubs as &dyn Fn(&mut Machine), 8), (&unmapped, 11)] {
        sweep(30, |name, opts, budget| {
            same(name, setup, seed, budget, opts, &[], &mut tally);
        });
    }
    assert!(
        tally.exits[ExitKind::Direct as usize] > 0 && tally.steps > 0,
        "{tally:?}"
    );
    assert!(tally.took(k::EXIT) > 0, "{tally:?}");
}

#[test]
fn a_store_into_a_word_the_trace_still_has_to_execute_leaves_through_the_early_exit() {
    let mut tally = Tally::default();
    sweep(10, |name, opts, budget| {
        same(name, &self_modifying(), 6, budget, opts, &[], &mut tally);
    });
    assert!(tally.exits[ExitKind::Stale as usize] > 0, "{tally:?}");
    let mut m = machine(&self_modifying(), 6);
    let mut t = Tally::default();
    drive(&mut m, 30, &BuildOpts::default(), &[], &mut t);
    assert_eq!(m.cpu.r[0], 2, "the rewritten instruction ran");
    assert!(t.exits[ExitKind::Stale as usize] > 0, "{t:?}");
}

#[test]
fn random_programs_equal_the_interpreter_on_every_option_set() {
    let sets = option_sets();
    let mut tally = Tally::default();
    for case in 0..fuzz_cases(120) {
        let seed = fuzz_seed() + case;
        let mut rng = Rng::new(0x0F10_0000 + seed);
        let n = 48 + rng.below(80);
        let words = random_program(&mut rng, n as usize);
        let flips: Vec<u32> = (0..rng.below(3)).map(|_| CODE + 4 * rng.below(n)).collect();
        let budget = 200 + u64::from(rng.below(600));
        let (name, opts) = &sets[case as usize % sets.len()];
        let label = format!("{name}, program {seed}");
        same(
            &label,
            &|m| put(m, CODE, &words),
            seed,
            budget,
            opts,
            &flips,
            &mut tally,
        );
    }
    assert!(
        tally.traces > 100 && tally.steps > 0 && tally.insns > 1000,
        "{tally:?}"
    );
    for kind in [k::GUARD, k::LOOP_BACK, k::GENERIC_END, k::EXIT] {
        assert!(
            tally.had(kind) > 0,
            "no trace with uop {kind} ran: {tally:?}"
        );
    }
    assert!(
        tally.took(k::GUARD) > 0 && tally.took(k::GENERIC_END) > 0,
        "{tally:?}"
    );
}

#[test]
fn the_probe_of_the_mutation_tests_passes_on_the_unmutated_builder() {
    probe();
}
