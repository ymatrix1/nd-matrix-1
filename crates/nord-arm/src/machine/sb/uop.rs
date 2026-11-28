//! OWNER: S0 (frozen). The micro-op IR of the ARM0 superblock tiers: `Uop`, the kind ids `k::*`,
//! the exits, the trace record `Sb`, and the flag-nibble helpers.
//!
//! Spec: `docs/research/arm0-jit-design.md` sections 3.1 and 3.2. Every kind id, fused ones
//! included, is fixed here so no later work package edits this file. The executor treats an
//! unknown id as `unreachable!`; [`k::family`] is the static table every dispatcher (executor,
//! native codegen) goes through, and a test enumerates all 256 ids against it.
//!
//! # S0 conventions (not in the design table; pinned here so parallel work packages agree)
//!
//! * `Uop::cond` of a memory uop and of `GENERIC` is the index of its *early exit* in
//!   `Sb::exits`: the exit taken after the uop when a store left `stale_code` non-empty (I6).
//!   `GENERIC_END` and every other control uop name their exit in `rs`. [`Uop::exit_idx`] hides
//!   the difference. The trace builder allocates the early exits (the lowerers cannot: they see
//!   no exit table) and stores the index with [`Uop::set_exit_idx`].
//! * Only an instruction that `ends_block` can fault or jump, and every ender is lowered by
//!   `lower_flow`, so only `GENERIC_END` can fault: it leaves through its own `Dynamic` exit and
//!   the fault stays in `cpu.fault`. The driver tests `cpu.fault` after every run, whatever the
//!   exit kind, and drains `Mem::stale_code` after every run too (an ender executed through
//!   `exec_decoded` may store, and the early exit only guards the non-enders). `ExitKind::Fault`
//!   is reserved: no builder emits it and no executor needs it.
//! * `Uop::n` is bookkeeping for validators and the fuser, never an input of an executor: the
//!   instructions an exit retires are `Exit::done` (I3). Counting `n` over the uops of a path in
//!   program order, a `PRED` body included (a conditional instruction whose condition fails
//!   still retires), gives the `done` of the exit that path leaves through.
//! * A fused uop (`k::CMP_GUARD_*`, `k::TST_GUARD_I`, `k::MOV_PAIR`, `k::LDR_PAIR`) covers two
//!   consecutive guest instructions at `pc` and `pc + 4` (`n == 2`) and is defined as the
//!   sequence of its unfused expansion; `uop::fused::expand` is that definition.
//! * An exit is *applied by whoever runs the trace* (the executor or the native epilogue), before
//!   it returns: `cpu.insns += done`, `*left -= done`, `cpu.r[15] = target`, `bus.pc = last_pc`,
//!   flag nibble written back. [`super::backend::RunOut`] then only names the exit taken and the
//!   new pc, and the driver reads `sb.exits[exit].kind` to decide what happens next. No uop but
//!   `GENERIC*`/`EXIT*` touches `r15` (I2), so the target of a non-dynamic exit is static.
//! * Page numbers in `Sb::pages` are `addr >> 12` (the unit of `Mem::stale_code`).
//! * No kind id exists for `MUL`/`MLA`/long multiplies and the other `SLOW` classes: they stay
//!   `GENERIC` until a design review allocates ids (the gaps 108..=111, 129..=143, 155..=159 are
//!   reserved, not allocated).

use crate::cpu::predecode::COND_MASK;
use crate::cpu::Cpu;

/// Most uops of one trace (`SbConfig::max_uops` default).
pub const MAX_UOPS: usize = 128;
/// Most exits of one trace; an exit index fits a `u8` and the 6 guards of a trace plus its
/// early exits stay below this.
pub const MAX_EXITS: usize = 64;
/// Most 4 KiB pages one trace reads.
pub const MAX_PAGES: usize = 4;
/// Index of a trace in the cache arena.
pub type SbId = u32;
/// No trace (an unlinked exit, an empty inline-cache slot).
pub const NO_SB: SbId = u32::MAX;

/// One micro-op, 16 bytes. See the field table of section 3.1 of the design note.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Uop {
    pub kind: u8,
    /// Condition 0..=13 of `PRED` / `GUARD` / fused guards; early-exit index of memory uops and
    /// `GENERIC` (see the module conventions).
    pub cond: u8,
    pub rd: u8,
    pub rn: u8,
    pub rm: u8,
    /// Shift register (DP form 2), memory flags (b0 U, b1 P, b2 WB, b3 REG), or exit index.
    pub rs: u8,
    pub sh: u8,
    /// Guest instructions the uop covers (1, or 2 when fused; 0 for `PRED` and the pure control
    /// uops). Bookkeeping only: executors take counts from `Exit::done`.
    pub n: u8,
    pub imm: u32,
    /// Guest address of the first covered instruction (becomes `bus.pc` before an access, and
    /// is the fault pc).
    pub pc: u32,
}

const _: () = assert!(core::mem::size_of::<Uop>() == 16);

impl Uop {
    /// A uop of `kind` covering one instruction at `pc`, every other field zero.
    pub const fn new(kind: u8, pc: u32) -> Self {
        Self {
            kind,
            cond: 0,
            rd: 0,
            rn: 0,
            rm: 0,
            rs: 0,
            sh: 0,
            n: 1,
            imm: 0,
            pc,
        }
    }

    /// `GENERIC`: run `word` at `pc` through the interpreter's own handler (I8).
    pub const fn generic(word: u32, pc: u32, early_exit: u8) -> Self {
        let mut u = Self::new(k::GENERIC, pc);
        u.imm = word;
        u.cond = early_exit;
        u
    }

    /// `GENERIC_END`: like `GENERIC`, then leave through exit `exit` (a `Dynamic` exit).
    pub const fn generic_end(word: u32, pc: u32, exit: u8) -> Self {
        let mut u = Self::new(k::GENERIC_END, pc);
        u.imm = word;
        u.rs = exit;
        u
    }

    /// `EXIT` through exit `exit`; it covers no instruction of its own.
    pub const fn exit(exit: u8, pc: u32) -> Self {
        let mut u = Self::new(k::EXIT, pc);
        u.n = 0;
        u.rs = exit;
        u
    }

    /// Index into `Sb::exits` this uop leaves through (or may leave through, for memory uops and
    /// `GENERIC`: their early exit).
    pub const fn exit_idx(&self) -> usize {
        match k::family(self.kind) {
            k::Family::Mem => self.cond as usize,
            _ if self.kind == k::GENERIC => self.cond as usize,
            _ => self.rs as usize,
        }
    }

    /// Set the exit index with the convention of [`Uop::exit_idx`].
    pub fn set_exit_idx(&mut self, idx: u8) {
        if matches!(k::family(self.kind), k::Family::Mem) || self.kind == k::GENERIC {
            self.cond = idx;
        } else {
            self.rs = idx;
        }
    }
}

/// Kind ids (`Uop::kind`). Ids are part of the frozen contract; the table is `uop/k.rs`.
pub mod k;

/// Layout and expansion of the fused kinds (`uop/fused.rs`).
pub mod fused;

/// How a trace ends (section 3.2).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitKind {
    /// Constant target.
    Direct,
    /// Target in `r[reg]`: bit 0 selects Thumb (`bx`).
    Indirect,
    /// Target in `r[reg] & !3`, no interworking (`mov pc,rm`).
    IndirectArm,
    /// Target is `cpu.r[15]` after a `GENERIC_END` instruction.
    Dynamic,
    /// Reserved, never emitted: a fault leaves through the `Dynamic` exit of its `GENERIC_END`
    /// (counted) and the driver reads `cpu.fault`.
    Fault,
    /// A store left `stale_code` non-empty: the early exit of memory uops and `GENERIC` (I6).
    Stale,
    /// The budget ended inside a loop.
    LoopBack,
    /// Return to the driver at a dispatch point (idle v2).
    Watch,
}

impl ExitKind {
    /// Number of kinds (index range of a histogram).
    pub const COUNT: usize = 8;
}

/// One way out of a trace. `done` and `last_pc` are static (I3).
#[derive(Clone, Debug)]
pub struct Exit {
    pub kind: ExitKind,
    /// Register of `Indirect` / `IndirectArm` exits.
    pub reg: u8,
    /// Guest instructions executed from the start of the iteration through the last one retired
    /// on this path.
    pub done: u16,
    /// Constant target of `Direct` exits.
    pub target: u32,
    /// Address of the last instruction retired on this path (becomes `bus.pc`).
    pub last_pc: u32,
    /// Linked successor, patched after the first lookup (`NO_SB` until then).
    pub link: SbId,
    /// Two-way inline cache of `Indirect` exits.
    pub ic_pc: [u32; 2],
    pub ic_id: [SbId; 2],
    pub hits: u32,
}

impl Exit {
    /// An unlinked exit of `kind`.
    pub const fn new(kind: ExitKind, done: u16, last_pc: u32, target: u32) -> Self {
        Self {
            kind,
            reg: 0,
            done,
            target,
            last_pc,
            link: NO_SB,
            ic_pc: [0; 2],
            ic_id: [NO_SB; 2],
            hits: 0,
        }
    }
}

/// A trace superblock: single entry, side exits, at most one loop-back to its head.
#[derive(Clone, Debug)]
pub struct Sb {
    pub entry: u32,
    /// Page numbers (`addr >> 12`) of every guest word the trace was built from.
    pub pages: [u32; MAX_PAGES],
    pub n_pages: u8,
    /// Most guest instructions on any path through the trace, one full loop iteration included.
    pub n_guest: u16,
    pub uops: Box<[Uop]>,
    pub exits: Box<[Exit]>,
    /// `(address, word)` of every guest word the trace depends on (literals included): the basis
    /// of revalidation after a store to one of its pages.
    pub words: Box<[(u32, u32)]>,
    pub live: bool,
    pub enters: u32,
    /// Id in the native backend, `NO_SB` when the trace is not compiled.
    pub native: SbId,
}

impl Sb {
    /// Assemble a trace; `pages` is derived from `words` (and `entry`). `None` when the trace
    /// would read more than [`MAX_PAGES`] pages or has more uops/exits than the caps.
    pub fn from_parts(
        entry: u32,
        n_guest: u16,
        uops: Vec<Uop>,
        exits: Vec<Exit>,
        words: Vec<(u32, u32)>,
    ) -> Option<Sb> {
        if uops.len() > MAX_UOPS || exits.len() > MAX_EXITS {
            return None;
        }
        let mut pages = [0u32; MAX_PAGES];
        let mut n_pages = 0usize;
        for page in std::iter::once(entry >> 12).chain(words.iter().map(|(a, _)| a >> 12)) {
            if !pages[..n_pages].contains(&page) {
                if n_pages == MAX_PAGES {
                    return None;
                }
                pages[n_pages] = page;
                n_pages += 1;
            }
        }
        Some(Sb {
            entry,
            pages,
            n_pages: n_pages as u8,
            n_guest,
            uops: uops.into_boxed_slice(),
            exits: exits.into_boxed_slice(),
            words: words.into_boxed_slice(),
            live: true,
            enters: 0,
            native: NO_SB,
        })
    }

    /// True when the trace read a word of page `page` (a page number).
    pub fn covers_page(&self, page: u32) -> bool {
        self.pages[..self.n_pages as usize].contains(&page)
    }
}

/// Flags of `cpu` as the nibble `n << 3 | z << 2 | c << 1 | v` (the layout of `Cpu::nzcv`).
#[inline(always)]
pub fn pack_flags(cpu: &Cpu) -> u8 {
    cpu.nzcv() as u8
}

/// Write the nibble back to `cpu.n/z/c/v` (and nothing else).
#[inline(always)]
pub fn unpack_flags(cpu: &mut Cpu, f: u8) {
    cpu.n = f & 8 != 0;
    cpu.z = f & 4 != 0;
    cpu.c = f & 2 != 0;
    cpu.v = f & 1 != 0;
}

/// Whether condition `cond` holds for flag nibble `f` (14 and 15 always hold).
#[inline(always)]
pub fn holds(cond: u8, f: u8) -> bool {
    COND_MASK[(cond & 15) as usize] >> (f & 15) & 1 != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uop_is_16_bytes() {
        assert_eq!(core::mem::size_of::<Uop>(), 16);
    }

    #[test]
    fn holds_equals_the_interpreter_condition_for_every_flag_nibble() {
        for cond in 0..14u8 {
            for f in 0..16u8 {
                let mut c = Cpu::new();
                unpack_flags(&mut c, f);
                assert_eq!(
                    holds(cond, f),
                    c.cond(cond as u32),
                    "cond {cond} flags {f:04b}"
                );
            }
        }
        assert!((0..16u8).all(|f| holds(14, f) && holds(15, f)));
    }

    #[test]
    fn pack_and_unpack_round_trip_and_touch_only_the_four_flags() {
        for f in 0..16u8 {
            let mut c = Cpu::new();
            c.q = true;
            c.mode = 0x1F;
            unpack_flags(&mut c, f);
            assert_eq!(pack_flags(&c), f);
            assert!(c.q && c.mode == 0x1F);
        }
    }

    #[test]
    fn exit_index_follows_the_family_convention() {
        let mut m = Uop::new(k::STR_IMM, 0x1000);
        m.set_exit_idx(5);
        assert_eq!((m.exit_idx(), m.cond, m.rs), (5, 5, 0));
        let mut g = Uop::generic(0xE1A0_0000, 0x1000, 3);
        assert_eq!(g.exit_idx(), 3);
        g.set_exit_idx(4);
        assert_eq!(g.exit_idx(), 4);
        let e = Uop::generic_end(0xEAFF_FFFE, 0x1000, 2);
        assert_eq!((e.exit_idx(), e.rs, e.cond), (2, 2, 0));
        let x = Uop::exit(1, 0x1000);
        assert_eq!((x.exit_idx(), x.n), (1, 0));
    }

    #[test]
    fn from_parts_derives_the_page_list_and_enforces_the_caps() {
        let exit = Exit::new(ExitKind::Dynamic, 1, 0x1000, 0);
        let sb = Sb::from_parts(
            0x1000,
            1,
            vec![Uop::generic_end(0, 0x1000, 0)],
            vec![exit.clone()],
            vec![(0x1000, 0), (0x1004, 0), (0x5000, 0)],
        )
        .unwrap();
        assert_eq!((sb.n_pages, sb.pages[..2].to_vec()), (2, vec![1, 5]));
        assert!(sb.covers_page(5) && !sb.covers_page(2) && sb.live);
        let five_pages: Vec<(u32, u32)> = (0..5).map(|p| (p << 12, 0)).collect();
        assert!(Sb::from_parts(0, 1, vec![], vec![], five_pages).is_none());
        let too_many = vec![Uop::default(); MAX_UOPS + 1];
        assert!(Sb::from_parts(0, 1, too_many, vec![], vec![]).is_none());
    }

    #[test]
    fn an_unlinked_exit_has_no_successor() {
        let e = Exit::new(ExitKind::Direct, 3, 0x1008, 0x2000);
        assert_eq!((e.link, e.ic_id, e.hits), (NO_SB, [NO_SB; 2], 0));
        assert_eq!(ExitKind::Watch as usize + 1, ExitKind::COUNT);
    }
}
