//! The `Fault` exit through the driver: a fault an `Interp` uop stores, and a fast uop that
//! refuses and is replayed by the interpreter (design 4.5, `chain.rs` module header).
//!
//! OWNER: W2-DRIVE. The refusing handler is a test kind in the table of a hand-built block
//! (`fixture::arm_one_block`): a fast family is not needed to see what the driver does with the
//! exit, and the all-`Interp` blocks never produce one that is not already stored.
use crate::core::support::{core_with, CODE};
use crate::core::uop::drive::fixture::{arm_one_block, arm_one_interp_block, nop_or_interp};
use crate::core::uop::types::{Exit, Handler, Uop, UopCtx, UopKind};
use crate::core::Core;

/// The fault-injecting handler of the tests below: a fast uop whose precondition failed. It
/// leaves the core alone and answers `Fault` without storing one.
unsafe extern "C" fn refuse(_: *mut UopCtx, _: *const Uop) -> u32 {
    Exit::Fault as u32
}

const REFUSE: UopKind = UopKind(UopKind::MOVE_BASE + 1);

/// A core running `program` from `CODE` with no event pending.
fn core_of(program: &[u16]) -> Core {
    let mut c = core_with(program);
    c.irq.due = u64::MAX;
    c
}

/// One block over `[CODE, CODE + n)`: NOPs fast, the instruction at `CODE + at` a `REFUSE` uop.
fn arm_refusing(core: &mut Core, n: u32, at: u32) {
    arm_one_block(
        core,
        (CODE, CODE + n),
        |c, b| {
            if c.pc == CODE + at {
                b.push(Uop::new(REFUSE, 1, c.pc, 0, c.next_pc));
                b.set_end(c.next_pc);
            } else {
                nop_or_interp(c, b);
            }
        },
        &[(REFUSE, refuse as Handler)],
    );
}

/// Two NOPs, an encoding the interpreter does not model (three parcels), a NOP.
const UNMODELLED: [u16; 6] = [0x0001, 0x0001, 0xd70a, 0x3c23, 0xd70a, 0x0001];

#[test]
fn an_interp_fault_ends_the_chain_with_the_state_of_the_faulting_instruction() {
    let (mut want, mut got) = (core_of(&UNMODELLED), core_of(&UNMODELLED));
    arm_one_interp_block(&mut got, (CODE, CODE + 6));
    let w = want.run(10);
    assert!(w.is_some(), "the interpreter faults too");
    assert_eq!(got.run_uop(10), w);
    assert_eq!(want.state_mismatch(&got), None);
    assert_eq!((got.pc, got.steps), (CODE + 2, 2));
    let s = got.uop_stats();
    assert_eq!(s.exits[Exit::Fault as usize], 1, "{s:?}");
    assert_eq!(
        (s.steps_uop, s.steps_interp_uop, s.steps_core_step),
        (2, 2, 0),
        "no replay: {s:?}"
    );
    assert_eq!(got.uop_take_fault(), None, "run_uop took it");
}

#[test]
fn a_refusing_fast_uop_is_replayed_by_the_interpreter_which_then_succeeds() {
    // the NOP at CODE + 1 is refused (as if a precondition of its fast form failed): the
    // driver runs it through `Core::step`, which executes it, and the chain goes on
    let program = [0x0001; 6];
    let (mut want, mut got) = (core_of(&program), core_of(&program));
    arm_refusing(&mut got, 4, 1);
    assert_eq!((want.run(4), got.run_uop(4)), (None, None));
    assert_eq!(want.state_mismatch(&got), None);
    assert_eq!((got.pc, got.steps), (CODE + 4, 4));
    let s = got.uop_stats();
    assert_eq!(s.exits[Exit::Fault as usize], 1, "{s:?}");
    assert_eq!(
        s.steps_core_step, 1,
        "the replay is the only Core::step: {s:?}"
    );
    assert_eq!(
        s.steps_uop, 3,
        "NOP, then the two after the replayed one: {s:?}"
    );
    assert_eq!(got.uop_take_fault(), None);
}

#[test]
fn a_refusing_fast_uop_whose_replay_faults_leaves_the_interpreters_fault() {
    let (mut want, mut got) = (core_of(&UNMODELLED), core_of(&UNMODELLED));
    arm_refusing(&mut got, 6, 2);
    let w = want.run(10);
    assert!(w.is_some());
    assert_eq!(
        got.run_uop(10),
        w,
        "the fault is the interpreter's, partial effects included"
    );
    assert_eq!(want.state_mismatch(&got), None);
    assert_eq!((got.pc, got.steps), (CODE + 2, 2));
    assert_eq!(got.uop_stats().exits[Exit::Fault as usize], 1);
}
