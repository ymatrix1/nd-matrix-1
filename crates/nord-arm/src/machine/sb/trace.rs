//! OWNER: W1-FLOW. Trace formation: `BuildOpts` and `build` (skeleton: a one-instruction trace
//! made of one `GENERIC_END`).
//!
//! Spec (design note 4.2, WP W1-FLOW): start at `start`; per instruction call `decode`, then
//! `lower_flow` for block enders (`Insn::is_ender`) with a `FlowCtx`, else
//! `decode::lower_or_generic` (the static registry: predicate wrap, `GENERIC` fallback). After
//! each instruction patch the early exits (see `uop.rs`): every pushed store uop and every
//! `GENERIC` gets a `Stale`/`Fault` `Exit` (static `done`, `last_pc`) whose index goes to
//! `Uop::set_exit_idx`. Collect the literal snapshot through the `fold` callback (it registers
//! the literal's page in the page list and its word in `words`). End at the caps in `opts`
//! (`max_uops` 128, `max_guards` 6, `max_pages` 4, never above `MAX_UOPS`/`MAX_EXITS`/`MAX_PAGES`).
//! Compute `n_guest` (longest path including one loop iteration) and build the `Sb` with
//! `Sb::from_parts`. I9: only mapped, unhooked pages are read; the builder never maps one. I1:
//! the trace depends on code words only.
//!
//! The peephole passes are registered here statically, in this order (each is a no-op in the
//! skeleton and owned by W2-PEEP): `fuse::run`, `flagopt::run`, `pred::run`.

use super::config::SbConfig;
use super::decode::{decode, Insn};
use super::flagopt;
use super::fuse;
use super::lower_flow::{lower_flow, FlowCtx};
use super::pred;
use super::uop::{Exit, Sb, Uop};
use crate::mem::{Mem, HOOK_X};

/// Trace-formation limits and pass switches (a copy of the matching `SbConfig` fields).
#[derive(Clone, Debug)]
pub struct BuildOpts {
    pub max_uops: usize,
    pub max_guards: usize,
    pub max_pages: usize,
    pub inline_depth: u8,
    pub fuse: bool,
    pub predicate: bool,
    pub dead_flags: bool,
    pub fold_literals: bool,
}

impl From<&SbConfig> for BuildOpts {
    fn from(c: &SbConfig) -> Self {
        Self {
            max_uops: c.max_uops,
            max_guards: c.max_guards,
            max_pages: c.max_pages,
            inline_depth: c.inline_depth,
            fuse: c.fuse,
            predicate: c.predicate,
            dead_flags: c.dead_flags,
            fold_literals: c.fold_literals,
        }
    }
}

impl Default for BuildOpts {
    fn default() -> Self {
        Self::from(&SbConfig::default())
    }
}

/// Build the trace starting at `start`, or `None` when none can be built there (misaligned pc,
/// unmapped or `HOOK_X` page). The skeleton returns one instruction as a `GENERIC_END` trace.
pub fn build(mem: &mut Mem, start: u32, opts: &BuildOpts) -> Option<Sb> {
    if start & 3 != 0 || !mem.is_mapped(start) || mem.flags(start) & HOOK_X != 0 {
        return None;
    }
    let insn: Insn = decode(start, mem.read32(start));
    let (mut uops, mut exits): (Vec<Uop>, Vec<Exit>) = (Vec::new(), Vec::new());
    let (mut depth, mut rets) = (0u8, Vec::new());
    let can_follow = |_: u32| false;
    lower_flow(&mut FlowCtx {
        insn,
        uops: &mut uops,
        exits: &mut exits,
        done: 0,
        taken_hint: None,
        can_follow: &can_follow,
        depth: &mut depth,
        rets: &mut rets,
        entry_pc: start,
        opts,
    });
    if opts.fuse {
        fuse::run(&mut uops, opts);
    }
    if opts.dead_flags {
        flagopt::run(&mut uops, opts);
    }
    if opts.predicate {
        pred::run(&mut uops, opts);
    }
    Sb::from_parts(start, 1, uops, exits, vec![(start, insn.word)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::uop::{k, ExitKind};

    #[test]
    fn build_options_copy_the_configuration() {
        let c = SbConfig {
            max_uops: 3,
            max_guards: 2,
            max_pages: 1,
            inline_depth: 0,
            fuse: true,
            predicate: true,
            dead_flags: true,
            fold_literals: true,
            ..SbConfig::default()
        };
        let o = BuildOpts::from(&c);
        assert_eq!(
            (o.max_uops, o.max_guards, o.max_pages, o.inline_depth),
            (3, 2, 1, 0)
        );
        assert!(o.fuse && o.predicate && o.dead_flags && o.fold_literals);
        let d = BuildOpts::default();
        assert!(!d.fuse && d.max_uops == 128);
    }

    #[test]
    fn the_skeleton_builds_one_generic_end_trace_that_snapshots_its_word() {
        let mut mem = Mem::new();
        mem.write32(0xC000_0000, 0xE3A0_0005); // mov r0,#5
        let sb = build(&mut mem, 0xC000_0000, &BuildOpts::default()).unwrap();
        assert_eq!(
            (sb.entry, sb.n_guest, sb.uops.len(), sb.exits.len()),
            (0xC000_0000, 1, 1, 1)
        );
        assert_eq!(sb.uops[0], Uop::generic_end(0xE3A0_0005, 0xC000_0000, 0));
        assert_eq!(
            (sb.exits[0].kind, sb.exits[0].done, sb.exits[0].last_pc),
            (ExitKind::Dynamic, 1, 0xC000_0000)
        );
        assert_eq!(&*sb.words, &[(0xC000_0000, 0xE3A0_0005)]);
        assert!(sb.covers_page(0xC000_0000 >> 12) && sb.live);
        assert_eq!(sb.uops[0].kind, k::GENERIC_END);
    }

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
}
