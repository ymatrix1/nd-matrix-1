//! Test-only rig: two cores built alike, the translated blocks of one of them run through the real
//! handler table next to `Core::step` on the other.
//!
//! OWNER: W2-FLOW. `a` is the oracle and only ever runs `Core::step`; `b` runs the blocks
//! translated by `flow_only` through the stand-in dispatch loop of `harness.rs`. After every block
//! (or every single step) the cores are compared with `exact_diff`, which sees the pc, the step
//! count, both stacks, the pending branch, the interrupt controller and the timers.
use super::harness::{flow_only, run_uops, Tally};
use crate::core::support::CODE;
use crate::core::uop::exec::event::room;
use crate::core::uop::hot::UopHot;
use crate::core::uop::lower::guard;
use crate::core::uop::lower::translate::translate_with;
use crate::core::uop::types::{Block, Exit, Handler, Uop, UopKind, NONE};
use crate::core::uop::verify::compare::exact_diff;
use crate::core::Core;

/// The oracle core, the engine core and the translated code of one program.
pub(in crate::core) struct Rig {
    pub a: Core,
    pub b: Core,
    pub blocks: Vec<Block>,
    pub uops: Vec<Uop>,
    table: Box<[Handler]>,
    pub tally: Tally,
}

impl Rig {
    /// Two cores with `program` at `CODE`, `setup` applied to both, and the program translated
    /// for both PEy modes with [`flow_only`].
    pub fn new(program: &[u16], setup: impl Fn(&mut Core)) -> Rig {
        let cores = [(); 2].map(|()| {
            let mut c = crate::core::support::core_with(program);
            c.irq.due = u64::MAX;
            setup(&mut c);
            c
        });
        let [a, mut b] = cores;
        b.uop_run.stop_pc = NONE;
        let cfg = crate::core::uop::config::UopConfig::default();
        let hi = CODE + program.len() as u32;
        let (blocks, uops) = translate_with(&mut b, &cfg, CODE, hi, flow_only);
        Rig {
            a,
            b,
            blocks,
            uops,
            table: UopHot::handler_table(),
            tally: Tally::default(),
        }
    }

    /// The block of mode `simd` that starts at `pc` and its uops.
    pub fn block_at(&self, pc: u32, simd: bool) -> Option<(&Block, &[Uop])> {
        let b = self
            .blocks
            .iter()
            .find(|b| b.pc == pc && b.simd == simd && b.len > 0)?;
        Some((b, &self.uops[b.first as usize..(b.first + b.len) as usize]))
    }

    /// The block of mode `simd` that has an instruction at `pc` and the index of its first uop
    /// there: what the pc map answers, so a block is entered at any uop boundary.
    pub fn entry_at(&self, pc: u32, simd: bool) -> Option<(&Block, usize)> {
        self.blocks
            .iter()
            .filter(|b| b.simd == simd && b.len > 0)
            .find_map(|b| {
                let uops = &self.uops[b.first as usize..(b.first + b.len) as usize];
                uops.iter()
                    .position(|u| u.pc == pc && u.steps > 0)
                    .map(|i| (b, i))
            })
    }

    /// Replaces the handler of `kind` in the table of this rig (a mutant, for negative controls).
    pub fn replace_handler(&mut self, kind: UopKind, h: Handler) {
        self.table[usize::from(kind.0)] = h;
    }

    /// Advances both cores by one block (entered at any uop boundary) when the entry guard and
    /// the deadline let the engine in, else by one interpreter step, and compares them.
    /// `Ok(false)` when the interpreter faulted (identically on both): the run is over.
    pub fn advance(&mut self) -> Result<bool, String> {
        let (pc, simd) = (self.b.pc, self.b.simd());
        let room = room(&self.b, u64::MAX);
        let entry = self.entry_at(pc, simd).filter(|(blk, i)| {
            let left = blk.steps - u32::from(blk.cum[*i]);
            guard::check(&self.b, blk, room).is_none() && room >= left
        });
        let Some((blk, from)) = entry else {
            self.tally.stepped += 1;
            return self.step_both();
        };
        let (end_pc, first) = (blk.end_pc, (blk.first as usize, blk.len as usize));
        let uops = self.uops[first.0 + from..first.0 + first.1].to_vec();
        let ran = run_uops(&mut self.b, &self.table, &uops, end_pc, &mut self.tally);
        self.tally.blocks += 1;
        for k in 0..ran.done {
            self.a
                .step()
                .map_err(|f| format!("oracle faulted in step {k} of a block: {f:?}"))?;
        }
        let alive = match ran.exit {
            Exit::Ok | Exit::Jump | Exit::Smc => true,
            // the uop did not run: the interpreter finishes the instruction on both
            Exit::Fault | Exit::Guard | Exit::Slow => self.step_both()?,
        };
        self.compare(&format!("block at {pc:#x} ended {ran:?}"))?;
        Ok(alive)
    }

    /// One `Core::step` on both cores, results compared.
    fn step_both(&mut self) -> Result<bool, String> {
        let (ra, rb) = (self.a.step(), self.b.step());
        if format!("{ra:?}") != format!("{rb:?}") {
            return Err(format!("step result {ra:?} vs {rb:?}"));
        }
        self.compare("single step")?;
        Ok(ra.is_ok())
    }

    /// Fails when the cores differ anywhere `exact_diff` looks.
    pub fn compare(&self, what: &str) -> Result<(), String> {
        match exact_diff(&self.a, &self.b) {
            Some(d) => Err(format!("{what}: {d}")),
            None => Ok(()),
        }
    }

    /// Advances until `max_iters` blocks or steps were taken or the run is over.
    pub fn run(&mut self, max_iters: u32) -> Result<(), String> {
        for _ in 0..max_iters {
            if !self.advance()? {
                break;
            }
        }
        Ok(())
    }

    /// Advances until the engine core is at `pc` (and equal to the oracle there). Fails when
    /// `limit` advances were not enough or the interpreter faulted first.
    pub fn run_to(&mut self, pc: u32, limit: u32) -> Result<(), String> {
        for _ in 0..limit {
            if self.b.pc == pc {
                return self.compare("at the end");
            }
            if !self.advance()? {
                return Err("the interpreter faulted".into());
            }
        }
        Err(format!(
            "pc {:#x} after {limit} advances, wanted {pc:#x}",
            self.b.pc
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::super::enc::{self, NOP, TRUE};
    use super::super::{JUMP, RET};
    use super::*;
    use crate::core::uop::types::{UF_FIN, UF_LAST};

    #[test]
    fn a_block_ending_in_a_jump_runs_through_the_real_handler_and_matches_the_interpreter() {
        let p = [&[NOP, NOP][..], &enc::jump(TRUE, CODE + 40, false, false)].concat();
        let mut rig = Rig::new(&p, |_| {});
        assert!(rig.advance().unwrap());
        assert_eq!(
            (rig.tally.blocks, rig.tally.calls(JUMP), rig.tally.nop),
            (1, 1, 2)
        );
        assert_eq!((rig.b.pc, rig.b.steps), (CODE + 40, 3));
        assert_eq!(rig.tally.exit(Exit::Jump), 1);
    }

    #[test]
    fn the_fin_flag_of_a_delay_slot_resolves_the_branch_and_counts_as_a_finish() {
        let p = [&enc::jump(TRUE, CODE + 40, false, true)[..], &[NOP, NOP]].concat();
        let mut rig = Rig::new(&p, |_| {});
        let (_, uops) = rig.block_at(CODE, false).unwrap();
        let flags: Vec<u8> = uops.iter().map(|u| u.flags & (UF_FIN | UF_LAST)).collect();
        assert_eq!(flags, [0, UF_FIN, UF_FIN | UF_LAST]);
        assert!(rig.advance().unwrap());
        assert_eq!(
            (rig.tally.fin, rig.tally.fin_jumps),
            (2, 1),
            "only the second slot moves the pc"
        );
        assert_eq!((rig.b.pc, rig.b.steps), (CODE + 40, 3));
    }

    #[test]
    fn an_engine_that_differs_from_the_interpreter_is_reported() {
        let p = [&[NOP][..], &enc::jump(TRUE, CODE + 40, false, false)].concat();
        let mut rig = Rig::new(&p, |_| {});
        let at = rig.uops.iter().position(|u| u.h == JUMP.0).unwrap();
        rig.uops[at].a += 1;
        let e = rig.advance().unwrap_err();
        assert!(e.contains(": pc: "), "{e}");
    }

    #[test]
    fn a_handler_that_bails_leaves_the_uop_to_the_interpreter_which_faults_on_both() {
        let mut rig = Rig::new(
            &[&[NOP][..], &enc::rts(TRUE, false, false)].concat(),
            |_| {},
        );
        assert!(
            !rig.advance().unwrap(),
            "RTS on an empty stack faults identically"
        );
        assert_eq!((rig.tally.calls(RET), rig.tally.exit(Exit::Fault)), (1, 1));
        assert_eq!(
            rig.b.pc,
            CODE + 1,
            "materialized at the uop that did not run"
        );
    }

    #[test]
    fn a_block_is_skipped_for_one_step_when_the_deadline_leaves_no_room() {
        let p = [&[NOP, NOP][..], &enc::jump(TRUE, CODE + 40, false, false)].concat();
        let mut rig = Rig::new(&p, |c| c.irq.due = 2);
        rig.run(1).unwrap();
        assert_eq!((rig.tally.blocks, rig.tally.stepped), (0, 1));
    }

    #[test]
    fn a_block_is_entered_at_any_uop_boundary_and_only_the_rest_of_it_runs() {
        let p = [
            &[NOP, NOP, NOP][..],
            &enc::jump(TRUE, CODE + 40, false, false),
        ]
        .concat();
        let mut rig = Rig::new(&p, |_| {});
        rig.b.pc = CODE + 2;
        rig.a.pc = CODE + 2;
        let (blk, at) = rig.entry_at(CODE + 2, false).unwrap();
        assert_eq!((blk.pc, at), (CODE, 2));
        assert!(rig.entry_at(CODE + 2, true).is_some() && rig.entry_at(CODE + 40, false).is_none());
        assert!(rig.advance().unwrap());
        assert_eq!(
            (rig.tally.nop, rig.tally.calls(JUMP), rig.b.steps),
            (1, 1, 2),
            "one NOP and the jump"
        );
    }

    #[test]
    fn running_to_a_pc_stops_there_and_reports_a_pc_that_is_not_reached() {
        let p = [&[NOP, NOP][..], &enc::jump(TRUE, CODE + 40, false, false)].concat();
        let mut rig = Rig::new(&p, |_| {});
        rig.run_to(CODE + 40, 5).unwrap();
        assert_eq!(rig.b.steps, 3);
        let mut rig = Rig::new(&p, |_| {});
        let e = rig.run_to(CODE + 50, 1).unwrap_err();
        assert!(e.contains("wanted"), "{e}");
    }

    #[test]
    fn a_replaced_handler_is_the_one_that_runs() {
        let p = [&[NOP][..], &enc::jump(TRUE, CODE + 40, false, false)].concat();
        let mut rig = Rig::new(&p, |_| {});
        unsafe extern "C" fn nowhere(
            ctx: *mut crate::core::uop::types::UopCtx,
            _: *const Uop,
        ) -> u32 {
            // SAFETY: the stand-in loop passes the live context.
            unsafe { (*ctx).next_pc = 0x1234 };
            Exit::Jump as u32
        }
        rig.replace_handler(JUMP, nowhere);
        assert!(rig.advance().is_err(), "the interpreter went to CODE + 40");
        assert_eq!(rig.b.pc, 0x1234);
    }
}
