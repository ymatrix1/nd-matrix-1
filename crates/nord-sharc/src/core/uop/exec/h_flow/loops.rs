//! `DO_COUNT` and `DO_UNTIL`: the instructions that push a loop frame.
//!
//! OWNER: W2-FLOW. The loop back-edge itself is `finish_step` (the `UF_FIN` of the last body
//! instruction, or the handler of a branch that is that instruction), not this file. `DO ... UNTIL
//! LCE` is `FlowOp::DoUntilCounter` of `exec_flow_op` with the operands of the uop; the arithmetic
//! `DO ... UNTIL cond` calls `Core::exec_do_until` itself on a [`Decoded`] rebuilt from the 48-bit
//! word in the uop, because its walk of the body (`body_len`, `refresh_check`) reads code and
//! flags and is exact only when it is the interpreter's own function. The frame is pushed through
//! `Core::push_loop`, so the loop stack overflow bit and its interrupt are the interpreter's.
//!
//! Allocation: none. `exec_do_until` walks the body twice (count, then index) in `refresh_check`
//! (`arith_loop.rs`), so `DO_UNTIL` allocates nothing, like every other flow handler (`allocs.rs`
//! pins all of them to 0, `DO_UNTIL` over bodies of 1, 5 and 38 instructions included).
use super::end_step;
use super::pack::FROM_REG;
use crate::core::exec::Decoded;
use crate::core::uop::exec::exit::materialize;
use crate::core::uop::types::{Exit, Uop, UopCtx};
use crate::core::{Core, LoopFrame};
use crate::visa::{bits, Insn, Kind};

/// A `DO` is a 48-bit instruction. VERIFIED: `visa/classify.rs` gives `T12a` and `T13a` three
/// parcels, checked by `lower/flow` (`lower` declines any other length) and by the census
/// encodings in `diff.rs`.
pub(in crate::core) const DO_LEN: u32 = 3;

/// `DO (pc, end) UNTIL LCE` with the counter in `b` (immediate, or `FROM_REG | register code`).
/// A count of 0 faults in the interpreter before anything changed.
///
/// A count read from a register is read through `Core::read_ureg`, whose system registers see the
/// interpreter's view of the machine: `FADDR`, `DADDR` and `PC` read `pc`, `EMUCLK`, `EMUCLK2` and
/// `TCOUNT` read the cycle clock (`steps`). Inside a block those are the block entry's values, so
/// the handler materializes first, like `SYS_LOAD` (`misc.rs`) and `INTERP`; an immediate count
/// reads nothing and does not. Checked by `diff_chain.rs` (a non-zero entry step count and a DO
/// after other instructions of its block, for every register that reads `pc` or `steps`).
pub(super) fn do_count(core: &mut Core, ctx: &mut UopCtx, u: &Uop) -> Exit {
    let next = u.pc.wrapping_add(DO_LEN);
    let count = if u.b & FROM_REG != 0 {
        materialize(core, ctx, u.pc);
        core.read_ureg((u.b & 0x7F) as u8)
    } else {
        u.b
    };
    if count == 0 {
        return Exit::Fault;
    }
    core.push_loop(LoopFrame {
        start: next,
        end: u.a,
        remaining: count,
        ..LoopFrame::default()
    });
    end_step(core, ctx, u, Kind::T12a, next, next)
}

/// `DO (pc, end) UNTIL cond`: `a` and `b` are the low 32 and the upper 16 bits of the word.
pub(super) fn do_until(core: &mut Core, ctx: &mut UopCtx, u: &Uop) -> Exit {
    let next = u.pc.wrapping_add(DO_LEN);
    let w = u64::from(u.a) | u64::from(u.b) << 32;
    let cond = bits(w, 37, 33);
    let d = Decoded {
        pc: u.pc,
        insn: Insn {
            kind: Kind::T13a,
            parcels: [(w >> 32) as u16, (w >> 16) as u16, w as u16],
            len: DO_LEN as usize,
        },
        w,
        cond_ok: core.cond(cond),
        cond_y: core.simd() && core.cond_for(true, cond),
    };
    if core.exec_do_until(&d, next).is_err() {
        // `exec_do_until` has no failing path; a future one must not leave a half-built loop
        return Exit::Fault;
    }
    end_step(core, ctx, u, Kind::T13a, next, next)
}

entry!(
    /// Handler of [`super::DO_COUNT`].
    h_do_count => do_count
);
entry!(
    /// Handler of [`super::DO_UNTIL`].
    h_do_until => do_until
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::{core_with, CODE};
    use crate::core::uop::exec::h_flow::{DO_COUNT, DO_UNTIL};
    use crate::core::uop::types::UopKind;

    fn run(core: &mut Core, kind: UopKind, a: u32, b: u32) -> (Exit, UopCtx) {
        let table = crate::core::uop::hot::UopHot::handler_table();
        let u = Uop::new(kind, 1, core.pc, a, b);
        let mut ctx = UopCtx::new(core as *mut Core, core.steps, 8);
        // SAFETY: `ctx.core` points to `core`, which outlives the call and is not used meanwhile.
        let r = unsafe { table[usize::from(kind.0)](&mut ctx, &u) };
        (Exit::from_u32(r).expect("an exit code"), ctx)
    }

    #[test]
    fn a_counted_loop_is_pushed_with_its_start_end_and_count() {
        let mut c = core_with(&[0x0001; 8]);
        let (exit, _) = run(&mut c, DO_COUNT, CODE + 5, 4);
        let f = c.loops.last().copied().expect("a frame");
        assert_eq!(exit, Exit::Ok);
        assert_eq!((f.start, f.end, f.remaining), (CODE + 3, CODE + 5, 4));
    }

    #[test]
    fn the_count_may_come_from_a_register_and_zero_faults_without_a_frame() {
        let mut c = core_with(&[0x0001; 8]);
        c.r[3] = 7;
        let (exit, _) = run(&mut c, DO_COUNT, CODE + 5, FROM_REG | 3);
        assert_eq!(
            (exit, c.loops.last().map(|f| f.remaining)),
            (Exit::Ok, Some(7))
        );
        c.loops.clear();
        c.r[3] = 0;
        let (exit, _) = run(&mut c, DO_COUNT, CODE + 5, FROM_REG | 3);
        assert_eq!((exit, c.loops.len()), (Exit::Fault, 0));
        let (exit, _) = run(&mut c, DO_COUNT, CODE + 5, 0);
        assert_eq!((exit, c.loops.len()), (Exit::Fault, 0));
    }

    #[test]
    fn a_count_register_that_reads_the_pc_or_the_cycle_clock_sees_the_materialized_core() {
        // entry step count 10, 3 steps done before the DO at CODE + 9: PC reads CODE + 9 and
        // EMUCLK reads 13, not the block entry's pc and 10
        for (ureg, want) in [(0x63u32, CODE + 9), (0x69, 13)] {
            let mut c = core_with(&[0x0001; 8]);
            c.steps = 10;
            let mut ctx = UopCtx::new(&mut c as *mut Core, 10, 8);
            ctx.done = 3;
            let u = Uop::new(DO_COUNT, 1, CODE + 9, CODE + 14, FROM_REG | ureg);
            let table = crate::core::uop::hot::UopHot::handler_table();
            // SAFETY: `ctx.core` points to `c`, which outlives the call and is not used meanwhile.
            let r = unsafe { table[usize::from(DO_COUNT.0)](&mut ctx, &u) };
            assert_eq!(r, Exit::Ok as u32, "ureg {ureg:#x}");
            assert_eq!(
                c.loops.last().map(|f| f.remaining),
                Some(want),
                "ureg {ureg:#x}"
            );
        }
    }

    #[test]
    fn the_do_instruction_never_takes_the_back_edge_of_the_loop_it_pushed() {
        // a frame whose end is the DO's own pc: `finish_step` skips it for kind T12a, so the
        // count stays and the exit is the fall-through
        let mut c = core_with(&[0x0001; 8]);
        let pc = c.pc;
        let (exit, ctx) = run(&mut c, DO_COUNT, pc, 3);
        assert_eq!(
            (exit, ctx.next_pc, c.loops.last().map(|f| f.remaining)),
            (Exit::Ok, u32::MAX, Some(3))
        );
    }

    #[test]
    fn an_arithmetic_loop_is_pushed_through_the_interpreters_own_function() {
        // DO (pc, 7) UNTIL FOREVER: parcels 0e3e 0000 0007
        let w = 0x0E3E_0000_0007u64;
        let mut c = core_with(&[0x0001; 12]);
        let (exit, _) = run(&mut c, DO_UNTIL, w as u32, (w >> 32) as u32);
        let f = c.loops.last().copied().expect("a frame");
        assert_eq!(exit, Exit::Ok);
        assert_eq!((f.start, f.end, f.term), (CODE + 3, CODE + 7, Some(31)));
    }
}
