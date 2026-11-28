//! Test-only stand-in for the dispatch loop: the lowering the flow tests translate with, the
//! executor that runs a block's uops through a handler table, and the tally of what ran.
//!
//! OWNER: W2-FLOW. W2-DRIVE owns the real loop (`exec/mod.rs`, still a skeleton here), so the flow
//! tests carry their own executor, written from the dispatch protocol of `exec/mod.rs` (P2): a uop
//! that returns `Ok` adds its steps to `done`, `UF_FIN` calls `finish_step(u.pc, kind, next)` with
//! the kind and the next pc of the decode-cache entry and treats a different result as `Jump`,
//! `Jump` and `Smc` count the steps of the uop, `Fault`, `Guard` and `Slow` leave the uop
//! unrun, and the core is materialized (`pc`, `steps`) at every exit. `INTERP` is emulated by its
//! rules (`h_core.rs`): materialize, `exec_cached`, `Jump` when the pc is not the one after it.
//! The rig that compares it with `Core::step` is in `rig.rs`.
use super::KINDS;
use crate::core::decode_cache::{CachedInsn, DecodedOp};
use crate::core::uop::exec::exit::materialize;
use crate::core::uop::lower::builder::BlockBuilder;
use crate::core::uop::lower::decode::decode_at;
use crate::core::uop::lower::flow::FlowFamily;
use crate::core::uop::lower::{Family, LowerCtx};
use crate::core::uop::types::{Exit, Handler, Uop, UopCtx, UopKind, UF_FIN};
use crate::core::Core;

/// The lowering the flow tests translate with: the flow family, a fast `NOP` uop for a NOP (so
/// delay slots and loop ends carry a `UF_FIN` that is not on an `Interp` uop) and `Interp` for
/// every other instruction.
pub(in crate::core) fn flow_only(cx: &LowerCtx<'_>, c: &CachedInsn, out: &mut BlockBuilder) {
    if FlowFamily::lower(cx, c, out) {
        return;
    }
    match c.op {
        DecodedOp::Nop => out.push(Uop::new(UopKind::NOP, 1, c.pc, 0, c.next_pc)),
        _ => out.interp(c),
    }
}

/// What the executor did, to prove a test exercised the handlers (design P10).
#[derive(Clone, Debug, Default)]
pub(in crate::core) struct Tally {
    /// Handler calls per flow kind, in `KINDS` order.
    pub flow: [u32; KINDS.len()],
    pub nop: u32,
    pub interp: u32,
    /// `UF_FIN` uops, and those whose `finish_step` moved the pc.
    pub fin: u32,
    pub fin_jumps: u32,
    /// Blocks run, and single interpreter steps taken instead (the guard said no).
    pub blocks: u32,
    pub stepped: u32,
    /// Uop exits by kind.
    pub exits: [u32; Exit::COUNT],
}

impl Tally {
    /// Calls of the handler of `kind`.
    pub fn calls(&self, kind: UopKind) -> u32 {
        KINDS
            .iter()
            .position(|k| *k == kind)
            .map_or(0, |i| self.flow[i])
    }

    /// Calls of every flow handler.
    pub fn flow_calls(&self) -> u32 {
        self.flow.iter().sum()
    }

    /// Uops that exited with `e`.
    pub fn exit(&self, e: Exit) -> u32 {
        self.exits[e as usize]
    }

    /// Adds the counts of `o`.
    pub fn merge(&mut self, o: &Tally) {
        for (a, b) in self.flow.iter_mut().zip(o.flow) {
            *a += b;
        }
        for (a, b) in self.exits.iter_mut().zip(o.exits) {
            *a += b;
        }
        self.nop += o.nop;
        self.interp += o.interp;
        self.fin += o.fin;
        self.fin_jumps += o.fin_jumps;
        self.blocks += o.blocks;
        self.stepped += o.stepped;
    }
}

/// How a run of uops ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::core) struct Ran {
    pub exit: Exit,
    /// Index of the uop that ended the run (`uops.len()` when it ran off the end).
    pub idx: usize,
    /// Steps accounted.
    pub done: u32,
}

/// `INTERP` by the rules of `h_core.rs`: one instruction through `exec_cached`. When the
/// instruction poked the deadline (`irq.due <= steps`) and uops follow it in the block, the block
/// must stop here (design 4.5 rule 4: the interpreter's next `step` runs the sequencer before
/// anything else); the stand-in reports it as a `Jump` to the pc the core is already at, which
/// is the effect, whatever W2-DRIVE decides to call it.
fn interp(core: &mut Core, ctx: &mut UopCtx, u: &Uop, more: bool) -> Exit {
    materialize(core, ctx, u.pc);
    let c = decode_at(core, u.pc);
    if core.exec_cached(&c).is_err() {
        return Exit::Fault;
    }
    if core.pc == u.b && !(more && core.irq.due <= core.steps) {
        Exit::Ok
    } else {
        ctx.next_pc = core.pc;
        Exit::Jump
    }
}

/// Runs `uops` of one block on `core` through `table`, from the first uop, and materializes the
/// core at the exit: `pc` is the target of a `Jump`, the uop of a `Fault` or `Slow`, `end_pc`
/// when every uop returned `Ok`.
pub(in crate::core) fn run_uops(
    core: &mut Core,
    table: &[Handler],
    uops: &[Uop],
    end_pc: u32,
    tally: &mut Tally,
) -> Ran {
    let mut ctx = UopCtx::new(core as *mut Core, core.steps, u32::MAX);
    for (idx, u) in uops.iter().enumerate() {
        ctx.core = core as *mut Core;
        let exit = if u.h == UopKind::NOP.0 {
            tally.nop += 1;
            Exit::Ok
        } else if u.h == UopKind::INTERP.0 {
            tally.interp += 1;
            interp(core, &mut ctx, u, idx + 1 < uops.len())
        } else {
            if let Some(i) = KINDS.iter().position(|k| k.0 == u.h) {
                tally.flow[i] += 1;
            }
            // SAFETY: `ctx.core` was derived from `core` just above and nothing else uses the
            // core during the call; `u` is a uop of the block.
            let r = unsafe { table[usize::from(u.h)](&mut ctx, u) };
            Exit::from_u32(r).expect("a handler returns an exit code")
        };
        tally.exits[exit as usize] += 1;
        let mut target = None;
        match exit {
            Exit::Ok | Exit::Jump | Exit::Smc => ctx.done += u32::from(u.steps),
            Exit::Fault | Exit::Guard | Exit::Slow => {
                materialize(core, &ctx, u.pc);
                return Ran {
                    exit,
                    idx,
                    done: ctx.done,
                };
            }
        }
        if exit == Exit::Jump {
            target = Some(ctx.next_pc);
        } else if exit == Exit::Ok && u.flags & UF_FIN != 0 {
            let c = decode_at(core, u.pc);
            tally.fin += 1;
            let pc = core.finish_step(u.pc, c.insn.kind, c.next_pc);
            if pc != c.next_pc {
                tally.fin_jumps += 1;
                target = Some(pc);
            }
        }
        if let Some(pc) = target {
            materialize(core, &ctx, pc);
            return Ran {
                exit: Exit::Jump,
                idx,
                done: ctx.done,
            };
        }
    }
    materialize(core, &ctx, end_pc);
    Ran {
        exit: Exit::Ok,
        idx: uops.len(),
        done: ctx.done,
    }
}

#[cfg(test)]
mod tests {
    use super::super::enc::{self, NOP, TRUE};
    use super::super::rig::Rig;
    use super::super::JUMP;
    use super::*;
    use crate::core::support::CODE;
    use crate::core::uop::hot::UopHot;

    #[test]
    fn flow_only_lowers_flow_and_nop_fast_and_everything_else_as_interp() {
        let p = [
            &[NOP][..],
            &[0x013e, 0x0000, 0x1312],
            &enc::jump(TRUE, CODE + 40, false, false),
        ]
        .concat();
        let rig = Rig::new(&p, |_| {});
        let (_, uops) = rig.block_at(CODE, false).unwrap();
        let k: Vec<u16> = uops.iter().map(|u| u.h).collect();
        assert_eq!(k, [UopKind::NOP.0, UopKind::INTERP.0, JUMP.0]);
    }

    #[test]
    fn tallies_add_up_field_by_field() {
        let p = [&[NOP][..], &enc::jump(TRUE, CODE + 40, false, false)].concat();
        let mut rig = Rig::new(&p, |_| {});
        rig.run(1).unwrap();
        let mut total = Tally::default();
        total.merge(&rig.tally);
        total.merge(&rig.tally);
        assert_eq!(
            (
                total.calls(JUMP),
                total.nop,
                total.blocks,
                total.exit(Exit::Jump),
                total.flow_calls()
            ),
            (2, 2, 2, 2, 2)
        );
    }

    #[test]
    fn the_executor_accounts_steps_by_the_exit_and_stops_at_a_uop_that_did_not_run() {
        let p = [&[NOP, NOP][..], &enc::rts(TRUE, false, false)].concat();
        let mut rig = Rig::new(&p, |_| {});
        let (blk, _) = rig.block_at(CODE, false).unwrap();
        let (end_pc, first, len) = (blk.end_pc, blk.first as usize, blk.len as usize);
        let uops = rig.uops[first..first + len].to_vec();
        let mut tally = Tally::default();
        let table = UopHot::handler_table();
        let ran = run_uops(&mut rig.b, &table, &uops, end_pc, &mut tally);
        assert_eq!(
            (ran.exit, ran.idx, ran.done),
            (Exit::Fault, 2, 2),
            "two NOPs ran, the RTS on an empty stack did not"
        );
        assert_eq!((rig.b.pc, rig.b.steps), (CODE + 2, 2), "materialized there");
    }
}
