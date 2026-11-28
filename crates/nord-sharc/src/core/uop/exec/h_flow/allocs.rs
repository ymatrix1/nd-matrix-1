//! Test-only: which flow handlers allocate (design 9.3: no allocation in a handler; P14).
//!
//! OWNER: W2-FLOW. Every flow handler reuses state the core already owns (the call stack, the
//! loop stack and the pending branch keep their capacity), so a steady-state call allocates
//! nothing. That includes `DO_UNTIL` (the arithmetic loop): it runs the interpreter's
//! `Core::exec_do_until`, whose `refresh_check` walks the body twice (count, then index) and no
//! longer collects the addresses in a `Vec` (`arith_loop.rs`, test
//! `the_check_address_equals_the_list_walk_on_random_bodies_and_does_not_allocate`).
use super::pack::{indirect, Branch, FROM_REG};
use super::{DO_COUNT, DO_UNTIL, IDLE, JUMP, JUMP_I, RET, SYS_LOAD};
use crate::core::interrupts::Serviced;
use crate::core::support::{core_with, CODE};
use crate::core::uop::hot::UopHot;
use crate::core::uop::types::{Exit, Handler, Uop, UopCtx, UopKind};
use crate::core::uop::verify::count_alloc::allocs_in;
use crate::core::Core;

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
fn a_steady_call_of_every_flow_handler_including_the_arithmetic_loop_allocates_nothing() {
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
    let until = |off: u32| {
        let w = 0x0E3E_0000_0000u64 | u64::from(off);
        (w as u32, (w >> 32) as u32)
    };
    let (until_1, until_5, until_38) = (until(3), until(7), until(40));
    let cases: [Case; 15] = [
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
        // DO (pc, off) UNTIL FOREVER over bodies of 1, 5 and 38 instructions (the walk of the
        // body once collected them in a Vec: 1, 2 and 5 allocations)
        ("do until 1", DO_UNTIL, until_1.0, until_1.1, nothing),
        ("do until 5", DO_UNTIL, until_5.0, until_5.1, nothing),
        ("do until 38", DO_UNTIL, until_38.0, until_38.1, nothing),
    ];
    for (name, kind, a, b, setup) in cases {
        let (exit, n) = steady_allocs(kind, a, b, setup);
        assert_eq!(n, 0, "{name} allocated ({exit:?})");
        assert!(matches!(exit, Exit::Ok | Exit::Jump), "{name}: {exit:?}");
    }
}
