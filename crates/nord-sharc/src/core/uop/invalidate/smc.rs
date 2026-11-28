//! The self-modifying-code rows of design 7.4, driven through the block run: a store into a
//! translated page, a store into another block, a store into the page of another block, and a
//! write through the short-word alias window.
//!
//! OWNER: W2-DRIVE. The engines here are armed with the `nop_lower` lowering (NOPs fast, the rest
//! `Interp`), because an all-`Interp` block decodes the live words on every run and so heals a
//! stale block by itself: only a fast NOP uop shows that a block outlived the code it was built
//! from. Every test also runs the stale block on purpose (`stale_state`) and checks that its
//! state differs from the interpreter's, so none of them can pass for want of a visible
//! difference.
//!
//! Encodings used (parcels in instruction order, like the program images):
//!
//! | Instruction | Parcels |
//! |---|---|
//! | `DM(0x9_0000 + w) = R0` | `0x1100, 0x0009, w` (VERIFIED: `longword::tests`, `aot_mem::tests` use the form) |
//! | `IF TRUE JUMP a` | `0x063e, a >> 16, a & 0xffff` |
//! | one 32-bit instruction | `0x0180, 0x1ecc` (checked: the interpreter runs it as one step) |
//! | `R3 = R1 + R2` | `0x013e, 0x0000, 0x1312` |
use crate::core::support::{load, CODE};
use crate::core::uop::drive::fixture::{arm_nop_fast, Setup};
use crate::core::uop::exec::run_block;
use crate::core::uop::types::Exit;
use crate::core::uop::verify::compare::exact_diff;
use crate::core::Core;

/// `DM(0x9_0000 + w) = R0`: the normal word `w` of the code window, short words `2w` and `2w + 1`.
fn store_r0(w: u16) -> [u16; 3] {
    [0x1100, 0x0009, w]
}

/// `IF TRUE JUMP to` for a short-word address `to`.
fn jump(to: u32) -> [u16; 3] {
    [0x063e, (to >> 16) as u16, to as u16]
}

/// The instruction that replaces two NOPs, and R0 holding it as a normal word (the parcel at the
/// lower address is the low half, `dm_write(CODE / 2, 0x013e_0001)` is parcel 0 = `0x0001`).
const NEW_INSN: [u16; 2] = [0x0180, 0x1ecc];
const R0: u32 = 0x1ecc_0180;

/// A program of `len` NOPs with `head` at its start.
fn program(len: usize, head: &[u16]) -> Vec<u16> {
    let mut p = vec![0x0001; len];
    p[..head.len()].copy_from_slice(head);
    p
}

/// A core running the setup through the interpreter and one armed with NOP-fast blocks over
/// `ranges`.
fn pair(setup: &Setup, ranges: &[(u32, u32)]) -> (Core, Core) {
    let mut got = setup.core();
    arm_nop_fast(&mut got, ranges);
    (setup.core(), got)
}

/// The state the stale blocks would leave: a driver without the revalidation (no `Smc` exit stops
/// it) runs the blocks at the pc, through the pc map, until the pc leaves translated code.
/// Returns the core and the steps it ran.
fn stale_state(setup: &Setup, ranges: &[(u32, u32)]) -> (Core, u64) {
    let (_, mut c) = pair(setup, ranges);
    let hot = c.uop.take().unwrap();
    while let Some((id, idx)) = hot.pcmap(false).get(c.pc) {
        let (e, _, _) = unsafe { run_block(&mut c, &hot, id, idx, 1000) };
        assert!(matches!(e, Exit::Ok | Exit::Jump | Exit::Smc), "{e:?}");
    }
    c.uop = Some(hot);
    let steps = c.steps;
    (c, steps)
}

/// The control of a test: after the same number of steps the interpreter is somewhere else than
/// the stale blocks, so the comparison with the engine means something.
fn assert_stale_is_visible(setup: &Setup, ranges: &[(u32, u32)]) {
    let (stale, steps) = stale_state(setup, ranges);
    let mut want = setup.core();
    assert_eq!(want.run(steps), None);
    assert!(
        want.state_mismatch(&stale).is_some(),
        "stale blocks differ from the interpreter"
    );
}

#[test]
fn smc_own_block_is_dropped_at_the_store_and_the_new_code_runs_like_the_interpreter() {
    // store, three NOPs, two NOPs the store replaces by one instruction, ten NOPs
    let setup = Setup::program(&program(18, &store_r0(3))).with(|c| c.r[0] = R0);
    let ranges = [(CODE, CODE + 18)];
    assert_stale_is_visible(&setup, &ranges);
    let (mut want, mut got) = pair(&setup, &ranges);
    assert_eq!((want.run(12), got.run_uop(12)), (None, None));
    assert_eq!(exact_diff(&want, &got), None);
    assert_eq!(
        got.pc,
        CODE + 6 + 2 + 7,
        "the new instruction took one step: {:#x}",
        got.pc
    );
    let s = got.uop_stats();
    assert_eq!(s.exits[Exit::Smc as usize], 1, "{s:?}");
    assert!(s.smc >= 1);
    assert_eq!(
        s.steps_uop, 1,
        "the store, then no block is left to enter: {s:?}"
    );
    let hot = got.uop.as_ref().unwrap();
    assert!(
        hot.blocks.iter().all(|b| b.len == 0),
        "both PEy modes dropped"
    );
    assert!(!hot.dropped.is_empty());
    assert!(got.uop_needs_maintenance());
    // after the maintenance the new code is in blocks again and the same program still agrees
    assert!(got.uop_maintain() > 0);
    assert!(!got.uop_needs_maintenance());
    (want.pc, got.pc) = (CODE, CODE);
    let before = got.uop_stats().steps_uop;
    assert_eq!((want.run(14), got.run_uop(14)), (None, None));
    assert_eq!(exact_diff(&want, &got), None);
    assert!(got.uop_stats().steps_uop > before, "blocks ran again");
}

#[test]
fn smc_identical_overlay_keeps_the_block_and_the_run_goes_on_through_it() {
    // the program already holds the new instruction: the store writes what is there (P6)
    let mut p = program(18, &store_r0(3));
    p[6..8].copy_from_slice(&NEW_INSN);
    let setup = Setup::program(&p).with(|c| c.r[0] = R0);
    let (mut want, mut got) = pair(&setup, &[(CODE, CODE + 18)]);
    let blocks = got.uop.as_ref().unwrap().blocks.len();
    assert_eq!((want.run(15), got.run_uop(15)), (None, None));
    assert_eq!(exact_diff(&want, &got), None);
    let s = got.uop_stats();
    assert_eq!(
        s.exits[Exit::Smc as usize],
        1,
        "a store into a code page always exits: {s:?}"
    );
    assert_eq!(
        (s.steps_uop, s.steps_core_step),
        (15, 0),
        "the block went on after it: {s:?}"
    );
    let hot = got.uop.as_ref().unwrap();
    assert_eq!(hot.blocks.len(), blocks);
    assert!(hot.blocks.iter().all(|b| b.len != 0) && hot.dropped.is_empty());
    assert!(!got.uop_needs_maintenance(), "nothing to retranslate");
    assert_eq!(s.translations, blocks as u64);
}

#[test]
fn smc_other_block_drops_that_block_and_keeps_the_storing_one() {
    // block A at CODE stores into block B at CODE + 0x100 (the next page), then jumps there
    let mut p = program(0x108, &store_r0(0x81));
    p[3..6].copy_from_slice(&jump(CODE + 0x100));
    let setup = Setup::program(&p).with(|c| c.r[0] = R0);
    let ranges = [(CODE, CODE + 6), (CODE + 0x100, CODE + 0x108)];
    assert_stale_is_visible(&setup, &ranges);
    let (mut want, mut got) = pair(&setup, &ranges);
    assert_eq!(
        got.uop.as_ref().unwrap().blocks.len(),
        4,
        "A and B, one per PEy mode"
    );
    // store, jump, two NOPs, the new instruction, three NOPs
    assert_eq!((want.run(8), got.run_uop(8)), (None, None));
    assert_eq!(exact_diff(&want, &got), None);
    assert_eq!(got.pc, CODE + 0x100 + 2 + 2 + 3);
    let s = got.uop_stats();
    assert_eq!(s.exits[Exit::Smc as usize], 1, "{s:?}");
    assert_eq!(
        s.steps_uop, 2,
        "the store and the jump ran in A, B was gone: {s:?}"
    );
    let hot = got.uop.as_ref().unwrap();
    let pcs: Vec<u32> = hot
        .blocks
        .iter()
        .filter(|b| b.len != 0)
        .map(|b| b.pc)
        .collect();
    assert_eq!(pcs, [CODE, CODE], "only the two blocks of A are alive");
    assert!(
        hot.dropped.iter().all(|&(lo, _)| lo == CODE + 0x100),
        "{:x?}",
        hot.dropped
    );
    assert!(hot.pcmap(false).get(CODE + 0x100).is_none() && hot.pcmap(false).get(CODE).is_some());
}

#[test]
fn smc_page_of_another_block_keeps_every_block_and_costs_no_translation() {
    // the store lands at CODE + 0x180: the page of block B (CODE + 0x100..0x108), not its code
    let mut p = program(0x108, &store_r0(0xC0));
    p[3..6].copy_from_slice(&jump(CODE + 0x100));
    let setup = Setup::program(&p).with(|c| c.r[0] = R0);
    let ranges = [(CODE, CODE + 6), (CODE + 0x100, CODE + 0x108)];
    let (mut want, mut got) = pair(&setup, &ranges);
    let translations = got.uop_stats().translations;
    // store, jump, eight NOPs
    assert_eq!((want.run(10), got.run_uop(10)), (None, None));
    assert_eq!(exact_diff(&want, &got), None);
    let s = got.uop_stats();
    assert_eq!(
        s.exits[Exit::Smc as usize],
        1,
        "the page is a code page: {s:?}"
    );
    assert_eq!(
        (s.steps_uop, s.steps_core_step),
        (10, 0),
        "B ran as a block: {s:?}"
    );
    assert_eq!(s.translations, translations);
    let hot = got.uop.as_ref().unwrap();
    assert!(hot.blocks.iter().all(|b| b.len != 0) && hot.dropped.is_empty());
    assert!(!got.uop_needs_maintenance());
}

#[test]
fn alias_write_invalidates() {
    // three NOPs become `R3 = R1 + R2` through the short-word alias window, the path of a DMA or
    // of a DSP store of one parcel; the interpreter's twin gets the same code loaded raw
    const ADD: [u16; 3] = [0x013e, 0x0000, 0x1312];
    let setup = Setup::nops(16).with(|c| (c.r[1], c.r[2]) = (5, 6));
    let (mut want, mut got) = pair(&setup, &[(CODE, CODE + 16)]);
    load(&mut want, CODE + 6, &ADD);
    for (k, parcel) in ADD.iter().enumerate() {
        got.dm_write(CODE + 6 + k as u32, u32::from(*parcel))
            .unwrap();
    }
    assert!(
        got.uop_needs_maintenance(),
        "the alias write marked the page"
    );
    assert_eq!((want.run(14), got.run_uop(14)), (None, None));
    assert_eq!(exact_diff(&want, &got), None);
    assert_eq!(got.r[3], 11, "the add ran, not the three NOPs it replaced");
    let s = got.uop_stats();
    assert_eq!(
        (s.steps_uop, s.steps_core_step),
        (0, 14),
        "no block survived the write: {s:?}"
    );
    assert!(got.uop.as_ref().unwrap().blocks.iter().all(|b| b.len == 0));
    // retranslated, the new code runs in blocks and still agrees
    assert!(got.uop_maintain() > 0);
    (want.pc, got.pc) = (CODE, CODE);
    (want.r[3], got.r[3]) = (0, 0);
    assert_eq!((want.run(14), got.run_uop(14)), (None, None));
    assert_eq!(exact_diff(&want, &got), None);
    assert_eq!(got.r[3], 11);
    assert!(
        got.uop_stats().steps_uop > 0,
        "blocks again: {:?}",
        got.uop_stats()
    );
}
