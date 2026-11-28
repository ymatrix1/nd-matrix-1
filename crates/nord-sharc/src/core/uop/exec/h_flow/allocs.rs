//! Test-only: which flow handlers allocate (design 9.3: no allocation in a handler; P14).
//!
//! OWNER: W2-FLOW. Every flow handler but one reuses state the core already owns (the call stack,
//! the loop stack and the pending branch keep their capacity), so a steady-state call allocates
//! nothing. The exception is `DO_UNTIL` (the arithmetic loop): it runs the interpreter's
//! `Core::exec_do_until`, whose `refresh_check` collects the body addresses in a `Vec`. That is
//! inherited from v1 (the interpreter and the `Interp` fallback allocate the same way), lives in
//! `arith_loop.rs`, which is not a W2-FLOW file, and is pinned here so it cannot grow: the handler
//! allocates exactly what `refresh_check` does and nothing else.
use super::pack::{indirect, Branch, FROM_REG};
use super::{DO_COUNT, DO_UNTIL, IDLE, JUMP, JUMP_I, RET, SYS_LOAD};
use crate::core::interrupts::Serviced;
use crate::core::support::{core_with, CODE};
use crate::core::uop::hot::UopHot;
use crate::core::uop::types::{Exit, Handler, Uop, UopCtx, UopKind};
use crate::core::uop::verify::count_alloc::allocs_in;
use crate::core::{Core, LoopFrame};

/// Parcels of the program of every case (16-bit NOPs).
const PROGRAM: usize = 64;

/// A case: its name, the kind, the operands `a` and `b`, and the state it runs in.
type Case = (&'static str, UopKind, u32, u32, fn(&mut Core));

/// The handler of `kind` in `table` on `core` at its pc. The table is built by the caller: building
/// it allocates.
fn call(table: &[Handler], core: &mut Core, kind: UopKind, a: u32, b: u32) -> Exit {
    let u = Uop::new(kind, 1, core.pc, a, b);
    let mut ctx = UopCtx::new(core as *mut Core, core.steps, 8);
    // SAFETY: `ctx.core` points to `core`, which outlives the call and is not used meanwhile.
    let r = unsafe { table[usize::from(kind.0)](&mut ctx, &u) };
    Exit::from_u32(r).expect("an exit code")
}

/// The allocations of the second call of a handler: the first one grows the stacks of a fresh
/// core, which a steady pass has already done. `setup` builds the state of one call.
fn steady_allocs(kind: UopKind, a: u32, b: u32, setup: fn(&mut Core)) -> (Exit, u64) {
    let table = UopHot::handler_table();
    let mut c = core_with(&[0x0001; PROGRAM]);
    setup(&mut c);
    call(&table, &mut c, kind, a, b);
    c.call_stack.clear();
    c.loops.clear();
    c.pending = None;
    c.irq.serviced.clear();
    setup(&mut c);
    allocs_in(|| call(&table, &mut c, kind, a, b))
}

#[test]
fn a_steady_call_of_every_flow_handler_but_the_arithmetic_loop_allocates_nothing() {
    let word = |delayed, call, rti| Branch::new(31, 3, delayed, call, rti).0;
    let ret = |delayed, rti| Branch::new(31, 1, delayed, false, rti).0;
    let with_return: fn(&mut Core) = |c| c.push_return_address(CODE + 5);
    let in_service: fn(&mut Core) = |c| {
        c.push_return_address(CODE + 5);
        c.irq.serviced.push(Serviced {
            vector: 6,
            status_pushed: false,
        });
    };
    let nothing: fn(&mut Core) = |_| {};
    let cases: [Case; 12] = [
        ("jump", JUMP, CODE + 40, word(false, false, false), nothing),
        ("call", JUMP, CODE + 40, word(false, true, false), nothing),
        (
            "delayed",
            JUMP,
            CODE + 40,
            word(true, false, false),
            nothing,
        ),
        (
            "jump (m,i)",
            JUMP_I,
            indirect(8, 8),
            word(false, false, false),
            nothing,
        ),
        ("rts", RET, 0, ret(false, false), with_return),
        ("rts db", RET, 0, ret(true, false), with_return),
        ("rti", RET, 0, ret(false, true), in_service),
        ("do imm", DO_COUNT, CODE + 5, 4, nothing),
        ("do reg", DO_COUNT, CODE + 5, FROM_REG | 3, |c| c.r[3] = 4),
        ("idle", IDLE, 0, word(false, false, false), nothing),
        ("sys load", SYS_LOAD, 0x7B, 5, nothing),
        ("sys load timer", SYS_LOAD, 0x6F, 5, nothing),
    ];
    for (name, kind, a, b, setup) in cases {
        let (exit, n) = steady_allocs(kind, a, b, setup);
        assert_eq!(n, 0, "{name} allocated ({exit:?})");
        assert!(matches!(exit, Exit::Ok | Exit::Jump), "{name}: {exit:?}");
    }
}

#[test]
fn the_arithmetic_loop_handler_allocates_what_the_interpreters_refresh_check_does_and_no_more() {
    // DO (pc, off) UNTIL FOREVER: bodies of 1, 5 and 38 16-bit instructions after the DO
    let mut walks = Vec::new();
    for off in [3u32, 7, 40] {
        let w = 0x0E3E_0000_0000u64 | u64::from(off);
        let (exit, handler) = steady_allocs(DO_UNTIL, w as u32, (w >> 32) as u32, |_| {});
        assert_eq!(exit, Exit::Ok);
        let frame = LoopFrame {
            start: CODE + 3,
            end: CODE + off,
            term: Some(31),
            ..LoopFrame::default()
        };
        let mut twin = core_with(&[0x0001; PROGRAM]);
        twin.push_loop(frame);
        twin.refresh_check(); // the first walk warms the decode cache, as the handler's first call did
        let ((), walk) = allocs_in(|| twin.refresh_check());
        assert_eq!(
            handler, walk,
            "off {off}: the handler allocates {handler} times, the interpreter's body walk {walk}"
        );
        walks.push(walk);
    }
    // the walk is the only allocation, and it is larger for a longer body (`Vec` growth)
    assert!(walks[2] >= walks[1], "{walks:?}");
}
