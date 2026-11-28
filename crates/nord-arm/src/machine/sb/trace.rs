//! OWNER: W1-FLOW. Trace formation: `BuildOpts`, `build` and `build_flipped` (design note 4.2,
//! WP W1-FLOW).
//!
//! A trace starts at `start` and follows one path through the code, instruction by instruction:
//!
//! * a block ender (`Insn::is_ender`) goes to `lower_flow` (branches followed or guarded, inlined
//!   returns, `LOOP_BACK`, `GENERIC_END`, see `lower_flow.rs`);
//! * everything else goes through `decode::lower_or_generic` (the static registry: predicate wrap,
//!   `GENERIC` fallback), and the builder then allocates the early exit that the memory and
//!   `GENERIC` uops need (I6): every store uop and every `GENERIC` of the instruction gets one
//!   `Stale` exit with the static `done` through the instruction, `last_pc` = its address and
//!   `target` = the next word. All uops of one instruction share it.
//!
//! The trace ends, always with a terminal uop (`EXIT`, `LOOP_BACK`, `EXIT_REG*` or `GENERIC_END`),
//! when `lower_flow` ends it, when the next pc is the entry (`LOOP_BACK`, also reached by falling
//! through), or with a `Direct` `EXIT` to the next pc when it cannot continue there: unmapped or
//! `HOOK_X` page (I9: the builder reads mapped pages only and never maps one), a pc already in the
//! trace (no inner loops, no duplicated code), the page cap, the uop cap, the instruction cap or
//! the exit-table cap. `done`, `last_pc` and every target are static (I3); `n_guest` is the
//! largest `done` of any exit, that is the longest path including one full loop iteration.
//!
//! Caps (`BuildOpts`): `max_uops` bounds the uops of the guest instructions plus the one terminal
//! uop (never above `MAX_UOPS`); the first instruction is always taken, so a trace has at least
//! one instruction whatever the cap and, for a cap of 1, ends up with two uops. An instruction
//! that would cross the cap is rolled back whole. `max_guards` is enforced by `lower_flow`,
//! `max_pages` here (code pages and literal pages together, never above `MAX_PAGES`).
//!
//! `words` holds `(address, word)` of every instruction the trace was built from and of every
//! folded literal, each address once, in the order they were read (I1: the trace depends on these
//! words only). The `fold` callback of the lowerers reads a literal only from a mapped page
//! without `HOOK_R`/`HOOK_W`/`HOOK_X`, at a word-aligned address, within the page cap, and only
//! when `opts.fold_literals` is set.
//!
//! The peephole passes are registered here statically, in this order (each is a no-op in the
//! skeleton and owned by W2-PEEP): `fuse::run`, `flagopt::run`, `pred::run`.
//!
//! [`build_flipped`] is the entry for the driver's "flipped re-layout" (design note 4.1): the
//! prediction of the conditional branches at the listed addresses (the `last_pc` of the hot guard
//! exit) is inverted relative to the static rule.
//!
//! ASSUMED: [`MAX_INSNS`], twice the uop cap: it only bounds `done` and the build time against a
//! lowerer that emits no uop for an instruction; confirmed when W3-VERIF sees no trace near it.

use super::decode::{decode, lower_or_generic, Insn, LowerCtx};
use super::flagopt;
use super::fuse;
use super::lower_flow::{lower_flow, mutant, Flow, FlowCtx};
use super::pred;
use super::uop::{k, Exit, ExitKind, Sb, Uop, MAX_EXITS, MAX_PAGES, MAX_UOPS};
use crate::cpu::predecode::op;
use crate::mem::Mem;
use parts::{early_exit, fetchable, fold_word, follow_ok};

#[cfg(test)]
pub(super) mod diff;
#[cfg(test)]
mod diff_env;
mod opts;
pub(super) mod parts;
#[cfg(test)]
pub(super) mod shape;
#[cfg(test)]
pub(super) mod shape_caps;
#[cfg(test)]
pub(super) mod testexec;

pub use opts::BuildOpts;

/// Most guest instructions of one trace (ASSUMED, see the module docs).
const MAX_INSNS: usize = 2 * MAX_UOPS;

/// Build the trace starting at `start`, or `None` when none can be built there (misaligned pc,
/// unmapped or `HOOK_X` page).
pub fn build(mem: &mut Mem, start: u32, opts: &BuildOpts) -> Option<Sb> {
    build_flipped(mem, start, opts, &[])
}

/// [`build`] with the prediction of the conditional branches at `flips` (instruction addresses)
/// inverted.
pub fn build_flipped(mem: &mut Mem, start: u32, opts: &BuildOpts, flips: &[u32]) -> Option<Sb> {
    if !fetchable(mem, start) {
        return None;
    }
    Builder {
        opts,
        flips,
        start,
        limit: opts.max_uops.clamp(1, MAX_UOPS),
        max_pages: opts.max_pages.clamp(1, MAX_PAGES),
        uops: Vec::new(),
        exits: Vec::new(),
        words: Vec::new(),
        pages: vec![start >> 12],
        visited: Vec::new(),
        depth: 0,
        rets: Vec::new(),
        done: 0,
        last_pc: start,
    }
    .run(mem)
}

/// What the main loop does after one instruction.
enum Step {
    Next(u32),
    End,
    /// The instruction was rolled back (it crossed a cap): end the trace in front of it.
    Overflow,
}

/// Lengths to restore when an instruction is rolled back.
struct Snap {
    uops: usize,
    exits: usize,
    words: usize,
    pages: usize,
    visited: usize,
    depth: u8,
    rets: usize,
    top: Option<u32>,
}

struct Builder<'a> {
    opts: &'a BuildOpts,
    flips: &'a [u32],
    start: u32,
    /// Uop budget of the trace including its terminal, `1..=MAX_UOPS`.
    limit: usize,
    max_pages: usize,
    uops: Vec<Uop>,
    exits: Vec<Exit>,
    words: Vec<(u32, u32)>,
    pages: Vec<u32>,
    visited: Vec<u32>,
    depth: u8,
    rets: Vec<u32>,
    /// Instructions retired so far on the (single) path, and the address of the last one.
    done: u16,
    last_pc: u32,
}

impl Builder<'_> {
    fn run(mut self, mem: &mut Mem) -> Option<Sb> {
        let mut pc = self.start;
        loop {
            if !self.visited.is_empty() {
                if pc == self.start {
                    self.terminal(k::LOOP_BACK, ExitKind::LoopBack, pc);
                    break;
                }
                if !follow_ok(mem, &self.visited, &self.pages, self.max_pages, pc) || self.full() {
                    self.terminal(k::EXIT, ExitKind::Direct, pc);
                    break;
                }
            }
            match self.step(mem, pc) {
                Step::Next(next) => pc = next,
                Step::End => break,
                Step::Overflow => {
                    self.terminal(k::EXIT, ExitKind::Direct, pc);
                    break;
                }
            }
        }
        self.finish()
    }

    /// No room for another instruction and the terminal uop, or for two more exits.
    fn full(&self) -> bool {
        usize::from(self.done) >= MAX_INSNS
            || self.uops.len() + 1 >= self.limit
            || self.exits.len() + 3 > MAX_EXITS
    }

    /// End the path with `kind` (`EXIT` or `LOOP_BACK`) towards `target`; the instructions it
    /// stands for were counted by the uops that retired them (`n = 0`).
    fn terminal(&mut self, kind: u8, exit: ExitKind, target: u32) {
        let idx = self.exits.len() as u8;
        self.exits
            .push(Exit::new(exit, self.done, self.last_pc, target));
        let mut u = Uop::new(kind, self.last_pc);
        u.n = 0;
        u.rs = idx;
        self.uops.push(u);
    }

    fn snap(&self) -> Snap {
        Snap {
            uops: self.uops.len(),
            exits: self.exits.len(),
            words: self.words.len(),
            pages: self.pages.len(),
            visited: self.visited.len(),
            depth: self.depth,
            rets: self.rets.len(),
            top: self.rets.last().copied(),
        }
    }

    fn restore(&mut self, s: Snap) {
        self.uops.truncate(s.uops);
        self.exits.truncate(s.exits);
        self.words.truncate(s.words);
        self.pages.truncate(s.pages);
        self.visited.truncate(s.visited);
        self.depth = s.depth;
        self.rets.truncate(s.rets);
        if self.rets.len() < s.rets {
            self.rets.extend(s.top); // the instruction popped it
        }
    }

    /// Lower the instruction at `pc` and say where the trace goes on.
    fn step(&mut self, mem: &mut Mem, pc: u32) -> Step {
        let first = self.visited.is_empty();
        let word = mem.read32(pc);
        let insn = decode(pc, word);
        let snap = self.snap();
        if !self.pages.contains(&(pc >> 12)) {
            self.pages.push(pc >> 12);
        }
        self.visited.push(pc);
        self.words
            .push((pc, if mutant(213) { word ^ 1 } else { word }));
        let flow = if insn.is_ender() {
            self.flow(mem, insn)
        } else {
            self.lower(mem, &insn);
            Flow::Follow(pc.wrapping_add(4))
        };
        // a trace that goes on needs room for its terminal uop; the first instruction is
        // always taken
        let room = usize::from(matches!(flow, Flow::Follow(_)));
        if !first && self.uops.len() + room > self.limit {
            self.restore(snap);
            return Step::Overflow;
        }
        self.done += 1;
        self.last_pc = pc;
        match flow {
            Flow::Follow(next) => Step::Next(next),
            Flow::End => Step::End,
        }
    }

    fn flow(&mut self, mem: &Mem, insn: Insn) -> Flow {
        let Self {
            opts,
            flips,
            start,
            max_pages,
            uops,
            exits,
            pages,
            visited,
            depth,
            rets,
            done,
            ..
        } = self;
        let flipped = !mutant(214) && flips.contains(&insn.pc);
        let taken_hint = (flipped && insn.entry.op == op::B && insn.is_cond())
            .then(|| insn.branch_target() > insn.pc);
        let (seen, held, cap) = (&*visited, &*pages, *max_pages);
        let can_follow = |t: u32| follow_ok(mem, seen, held, cap, t);
        lower_flow(&mut FlowCtx {
            insn,
            uops,
            exits,
            done: *done,
            taken_hint,
            can_follow: &can_follow,
            depth,
            rets,
            entry_pc: *start,
            opts,
        })
    }

    /// Lower a non-ender through the registry and allocate its early exit.
    fn lower(&mut self, mem: &mut Mem, insn: &Insn) {
        let from = self.uops.len();
        {
            let Self {
                opts,
                max_pages,
                uops,
                words,
                pages,
                ..
            } = self;
            let mut fold = |a: u32| fold_word(mem, words, pages, *max_pages, opts.fold_literals, a);
            lower_or_generic(&mut LowerCtx {
                insn: *insn,
                out: uops,
                fold: &mut fold,
                opts,
            });
        }
        early_exit(&mut self.uops, &mut self.exits, from, insn, self.done);
    }

    fn finish(mut self) -> Option<Sb> {
        let longest = self.exits.iter().map(|e| e.done).max().unwrap_or(0);
        let n_guest = if mutant(211) {
            longest.saturating_sub(1)
        } else {
            longest
        };
        if self.opts.fuse {
            fuse::run(&mut self.uops, self.opts);
        }
        if self.opts.dead_flags {
            flagopt::run(&mut self.uops, self.opts);
        }
        if self.opts.predicate {
            pred::run(&mut self.uops, self.opts);
        }
        Sb::from_parts(self.start, n_guest, self.uops, self.exits, self.words)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::{HOOK_C, HOOK_X};

    #[test]
    fn nothing_is_built_on_unmapped_hooked_or_misaligned_code_and_nothing_is_mapped() {
        let mut mem = Mem::new();
        let opts = BuildOpts::default();
        assert!(
            build(&mut mem, 0xC000_0000, &opts).is_none(),
            "unmapped (I9)"
        );
        assert_eq!(
            (mem.auto_mapped_count, mem.is_mapped(0xC000_0000)),
            (0, false)
        );
        mem.write32(0xC000_1000, 0xE3A0_0005);
        mem.add_flag(0xC000_1000, 0xC000_1000, HOOK_X);
        assert!(build(&mut mem, 0xC000_1000, &opts).is_none(), "hooked page");
        mem.write32(0xC000_2000, 0xE3A0_0005);
        assert!(build(&mut mem, 0xC000_2002, &opts).is_none(), "misaligned");
    }

    #[test]
    fn a_one_branch_program_builds_a_loop_and_the_builder_leaves_the_memory_alone() {
        let mut mem = Mem::new();
        mem.write32(0xC000_0000, 0xE3A0_0005); // mov r0,#5
        mem.write32(0xC000_0004, 0xEAFF_FFFD); // b 0xC0000000
        let before = (
            mem.state_hash(),
            mem.auto_mapped_count,
            mem.flags(0xC000_0000),
        );
        let sb = build(&mut mem, 0xC000_0000, &BuildOpts::default()).unwrap();
        assert_eq!((sb.n_guest, sb.uops.len()), (2, 2));
        assert_eq!(sb.uops[1].kind, k::LOOP_BACK);
        let last = sb.exits.last().unwrap();
        assert_eq!(
            (last.kind, last.done, last.target),
            (ExitKind::LoopBack, 2, 0xC000_0000)
        );
        assert_eq!(
            &*sb.words,
            &[(0xC000_0000, 0xE3A0_0005), (0xC000_0004, 0xEAFF_FFFD)]
        );
        let after = (
            mem.state_hash(),
            mem.auto_mapped_count,
            mem.flags(0xC000_0000),
        );
        assert_eq!(before, after, "building reads memory and marks nothing");
        assert_eq!(
            mem.flags(0xC000_0000) & HOOK_C,
            0,
            "marking is the driver's job"
        );
    }
}
