//! Hand-built event-aware blocks (`AotBlock { ev: true }`) run through `Core::run_aot` against the
//! plain interpreter with an event due at every possible position, with and without a taken
//! interrupt, on a delayed branch and on a counter-loop end. They fail if the entry guard, the
//! `SPLIT` mapping in `aot_enter` or the item event paths in `events.rs` are removed or wrong.
#[cfg(test)]
mod tests {
    use crate::core::aot_hot::AotBlock;
    use crate::core::support::{core_with, CODE};
    use crate::core::{Core, Fault};

    const ADD: [u16; 3] = [0x013e, 0x0000, 0x1312];
    const NOP: u16 = 0x0001;
    const JUMP_DB: [u16; 2] = [0x083e, 0x043f]; // JUMP (M8, I8) (DB)
    const DO_LCE: [u16; 3] = [0x0C00, 0x0480, 0x0003]; // DO ... UNTIL LCE (loop-end test of aot_whole)

    /// ADD, ADD, JUMP (DB), two delay slots.
    fn delayed_program() -> Vec<u16> {
        let mut p = [ADD, ADD].concat();
        p.extend_from_slice(&JUMP_DB);
        p.extend_from_slice(&[NOP, NOP]);
        p.extend_from_slice(&ADD);
        p
    }

    fn run_delayed(core: &mut Core) -> Result<(), Fault> {
        core.aot_step_ev(CODE, CODE + 3)?;
        core.aot_step_ev(CODE + 3, CODE + 6)?;
        core.aot_step_ev(CODE + 6, CODE + 8)?;
        core.aot_step_ev(CODE + 8, CODE + 9)?;
        core.aot_step_ev(CODE + 9, CODE + 10)
    }

    /// DO (3 parcels), ADD: the loop ends at the ADD, the block's last instruction (a loop end inside
    /// a block is refused by the guard, as in v1).
    fn loop_program() -> Vec<u16> {
        [DO_LCE.as_slice(), &ADD, &ADD, &ADD].concat()
    }

    fn run_loop(core: &mut Core) -> Result<(), Fault> {
        core.aot_step_ev(CODE, CODE + 3)?;
        core.aot_step_ev(CODE + 3, CODE + 6)
    }

    /// An event-aware block for the first `steps` instructions of `program`.
    fn block(program: &[u16], steps: u32, run: fn(&mut Core) -> Result<(), Fault>) -> AotBlock {
        let n = match steps {
            5 => 10,
            2 => 6,
            _ => 9,
        };
        let code: Vec<u8> = program[..n].iter().flat_map(|p| p.to_le_bytes()).collect();
        AotBlock {
            start: CODE,
            last: CODE + n as u32 - 1,
            steps,
            native: 0,
            dynamic: true,
            code: Box::leak(code.into_boxed_slice()),
            ev: true,
            run,
        }
    }

    fn core(program: &[u16], interrupt: bool) -> Core {
        let mut c = core_with(program);
        c.irq.due = u64::MAX;
        if interrupt {
            c.mode1 |= 1 << 12; // IRPTEN
            c.imask |= 1 << 11;
        }
        c
    }

    /// Runs the block on one core and the interpreter on another for `steps` steps with the event
    /// due at `steps0 + due_at` (or an interrupt scheduled at once) and compares everything.
    fn same_as_the_interpreter(
        program: &[u16],
        b: AotBlock,
        due_at: Option<u64>,
        interrupt: bool,
    ) -> Core {
        let (mut a, mut i) = (core(program, interrupt), core(program, interrupt));
        a.aot_install(vec![b]);
        for c in [&mut a, &mut i] {
            if let Some(k) = due_at {
                c.irq.due = c.steps + k;
            }
            if interrupt {
                c.schedule_interrupt(0, 11);
            }
        }
        let n = u64::from(b.steps);
        assert_eq!(a.run_aot(n), None);
        assert_eq!(i.run(n), None);
        assert_eq!(
            a.state_mismatch(&i),
            None,
            "due_at {due_at:?} irq {interrupt}"
        );
        a
    }

    #[test]
    fn entry_is_not_refused_for_an_event_inside_an_event_aware_block() {
        let p = delayed_program();
        let mut c = core(&p, false);
        let mut b = block(&p, 5, run_delayed);
        c.irq.due = c.steps + 2;
        assert!(c.aot_try_ready_for_test(&b), "ev: event inside the block");
        b.ev = false;
        assert!(!c.aot_try_ready_for_test(&b), "v1: refused");
    }

    #[test]
    fn a_delayed_branch_matches_the_interpreter_with_an_event_at_every_position() {
        let p = delayed_program();
        for k in 0..=6 {
            // an event at a delay slot may split (the branch lands after it): exact either way
            same_as_the_interpreter(&p, block(&p, 5, run_delayed), Some(k), false);
        }
    }

    #[test]
    fn a_counter_loop_end_matches_the_interpreter_with_an_event_at_every_position() {
        let p = loop_program();
        for k in 0..=3 {
            same_as_the_interpreter(&p, block(&p, 2, run_loop), Some(k), false);
        }
    }

    #[test]
    fn a_taken_interrupt_splits_the_block_and_the_state_is_the_interpreters() {
        for (p, steps, run) in [
            (
                delayed_program(),
                5,
                run_delayed as fn(&mut Core) -> Result<(), Fault>,
            ),
            (loop_program(), 2, run_loop),
        ] {
            let a = same_as_the_interpreter(&p, block(&p, steps, run), None, true);
            let st = a.aot_hot_stats();
            assert!(
                st.taken >= 1 && st.splits >= 1 && st.ev_steps >= 1,
                "{st:?}"
            );
            assert!(st.bails >= 1, "a split is counted as a bail");
        }
    }
}
