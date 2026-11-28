//! Block entry: which pc map the driver looks the pc up in (design P4), which cached link it
//! follows, and which guard row it counts.
//!
//! OWNER: W2-DRIVE. These are the tests of `chain.rs::next_block` and of the first-block rule of
//! the guard count, kept apart so `chain.rs` stays small. Every engine here is armed with the
//! all-`Interp` lowering, whose blocks are the same in both PEy modes, so a block of the wrong mode
//! behaves exactly like the right one and only a flag set on it (`poisoned`) or a counter shows
//! which block the driver entered. A wrong map is exact anyway (the `Simd` guard row sends the
//! step to `Core::step`, which is why the equivalence tests with PEy on cannot see it), so the
//! selection of `uop_chain` (`self.simd()`) is pinned here and only here.
use crate::core::support::CODE;
use crate::core::uop::drive::fixture::{arm_interp, nop_core, run_both, Setup};
use crate::core::uop::types::{GuardFail, Link};
use crate::core::{Core, MODE1_PEYEN};

/// Poisons the block that starts at `pc` in the pc map of mode `simd` (the map is left alone, so
/// the driver still finds the block and the guard turns it down).
fn poison_at(c: &mut Core, simd: bool, pc: u32) {
    let hot = c.uop.as_mut().expect("armed");
    let (id, idx) = hot.pcmap(simd).get(pc).expect("mapped");
    assert_eq!(idx, 0, "a block starts at {pc:#x}");
    hot.blocks[id as usize].poisoned = true;
}

/// Ten NOPs armed in both modes, the core in PEy mode `pey`.
fn core_in_mode(pey: bool) -> Core {
    let mut c = nop_core(10);
    arm_interp(&mut c, &[(CODE, CODE + 10)]);
    if pey {
        c.mode1 |= MODE1_PEYEN;
    }
    assert_eq!(c.simd(), pey);
    c
}

#[test]
fn each_pey_mode_enters_the_blocks_of_its_own_map() {
    for pey in [false, true] {
        // the block of the OTHER mode is poisoned: entering it would fail the Valid row
        let mut c = core_in_mode(pey);
        {
            let hot = c.uop.as_ref().unwrap();
            let (off, on) = (hot.pcmap(false).get(CODE), hot.pcmap(true).get(CODE));
            assert_ne!(off.unwrap().0, on.unwrap().0, "one block per mode");
            assert!(hot.blocks[on.unwrap().0 as usize].simd);
        }
        poison_at(&mut c, !pey, CODE);
        assert!(c.uop_try(100), "PEy {pey}: the block of its own mode ran");
        let s = c.uop_stats();
        assert_eq!((c.steps, s.blocks_run, s.guard_fail), (10, 1, [0; 9]));
        // the block of ITS mode is poisoned: the entry finds it, and the guard counts its row
        let mut c = core_in_mode(pey);
        poison_at(&mut c, pey, CODE);
        assert!(!c.uop_try(100), "PEy {pey}");
        let s = c.uop_stats();
        assert_eq!(s.guard_fail[GuardFail::Valid as usize], 1, "PEy {pey}");
        assert_eq!((c.steps, s.blocks_run, s.steps_core_step), (0, 0, 1));
    }
}

#[test]
fn a_core_with_pey_on_runs_blocks_and_equals_the_interpreter() {
    // R3 = R1 + R2 between NOPs: in PEy mode it adds the S registers too, and the engine has to
    // run it through the PEy-on blocks (the PEy-off ones would fail the Simd row and run nothing)
    let add = [0x0001, 0x013e, 0x0000, 0x1312, 0x0001, 0x0001];
    for pey in [false, true] {
        let setup = Setup::program(&add).with(move |c| {
            (c.r[1], c.r[2], c.s[1], c.s[2]) = (3, 4, 5, 6);
            if pey {
                c.mode1 |= MODE1_PEYEN;
            }
        });
        let r = run_both(&setup, 5);
        assert_eq!(r.diff, None, "PEy {pey}: {r:?}");
        assert!(r.exercised(), "PEy {pey}: {r:?}");
        assert_eq!(r.stats.guard_fail[GuardFail::Simd as usize], 0, "PEy {pey}");
        let mut got = setup.armed();
        assert_eq!(got.run_uop(5), None);
        assert_eq!(got.r[3], 7);
        assert_eq!(got.s[3], if pey { 11 } else { 0 }, "PEy {pey}: the S view");
    }
}

#[test]
fn a_cached_link_to_a_block_of_the_other_mode_is_not_followed() {
    // two blocks per mode: [CODE, +5) and [+5, +10)
    let mut c = nop_core(10);
    arm_interp(&mut c, &[(CODE, CODE + 5), (CODE + 5, CODE + 10)]);
    let hot = c.uop.as_mut().unwrap();
    let first = hot.pcmap(false).get(CODE).unwrap().0;
    let (second_off, second_on) = (
        hot.pcmap(false).get(CODE + 5).unwrap().0,
        hot.pcmap(true).get(CODE + 5).unwrap().0,
    );
    assert_ne!(second_off, second_on);
    // a link that is current (this epoch) but points at the block of the other mode
    hot.blocks[first as usize].links[0] = Link {
        id: second_on,
        epoch: hot.epoch,
    };
    assert!(c.uop_try(100));
    assert_eq!(
        c.steps, 10,
        "followed, the PEy-on block would have failed the Simd row after five steps"
    );
    let hot = c.uop.as_ref().unwrap();
    assert_eq!(
        hot.blocks[first as usize].links[0],
        Link {
            id: second_off,
            epoch: hot.epoch
        },
        "the map lookup replaced the link"
    );
    assert_eq!(c.uop_stats().guard_fail, [0; 9]);
}

#[test]
fn a_failing_guard_row_is_counted_for_the_first_block_of_a_call_only() {
    // the row of a poisoned block is Valid
    let mut c = core_in_mode(false);
    poison_at(&mut c, false, CODE);
    assert!(!c.uop_try(100));
    let s = c.uop_stats();
    assert_eq!(s.guard_fail[GuardFail::Valid as usize], 1);
    assert_eq!((s.blocks_run, s.steps_core_step, c.steps), (0, 1, 0));
    // the same row met by the second block of a chain is not counted by that call: it returns
    // true after the steps it ran and the next call enters the block and counts the row
    let mut c = nop_core(10);
    arm_interp(&mut c, &[(CODE, CODE + 5), (CODE + 5, CODE + 10)]);
    poison_at(&mut c, false, CODE + 5);
    assert!(c.uop_try(100));
    assert_eq!((c.steps, c.pc), (5, CODE + 5));
    assert_eq!(c.uop_stats().guard_fail, [0; 9], "not counted by this call");
    assert!(!c.uop_try(100));
    let s = c.uop_stats();
    assert_eq!(s.guard_fail[GuardFail::Valid as usize], 1);
    assert_eq!(
        (s.blocks_run, s.steps_core_step),
        (1, 1),
        "one block ran, one step is left to the caller"
    );
}
