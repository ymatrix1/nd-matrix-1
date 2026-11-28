//! Shared types of the uop engine: the 16-byte [`Uop`], the handler ABI, the exit protocol, the
//! block format and the reserved [`UopKind`] ranges.
//!
//! OWNER: W0 (skeleton). Later packages never edit this file; a contract change goes back to W0
//! as an amendment because every family and both tiers depend on it. Spec:
//! `docs/specs/sharc-uop.md`, design: `docs/research/sharc-compiler-design.md` section 10.3.
use crate::core::Core;
use std::ops::Range;

/// "No index": block id, uop index, native slot or pc that is not set.
pub const NONE: u32 = u32::MAX;

/// log2 of the code-page size in short words (one invalidation bit covers 256 short words).
/// ASSUMED: a trade of invalidation granularity against bitmap size (512 bytes for the whole
/// short-word window); confirm with overlay traces of a program change.
pub const PAGE_SHIFT: u32 = 8;

/// One micro-operation: a handler index, the steps it accounts for and two operands.
///
/// `pc` is the short-word address of the first instruction the uop covers; every cold exit
/// materializes the core state at this pc. VERIFIED: size and alignment by `uop_is_16_bytes`.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Uop {
    /// Handler index, [`UopKind`]`.0`.
    pub h: u16,
    /// Steps this uop accounts for: 1 for a normal instruction, 0 for the second uop of one
    /// instruction, 2 or 3 for a fused group.
    pub steps: u8,
    /// `UF_*` flags.
    pub flags: u8,
    /// Operand a: register index, immediate, packed compute word or constant index.
    pub a: u32,
    /// Operand b, as `a`.
    pub b: u32,
    /// pc (short words) of the first covered instruction.
    pub pc: u32,
}
const _: () = assert!(core::mem::size_of::<Uop>() == 16);

impl Uop {
    /// A uop of `kind` covering `steps` steps at `pc`.
    pub const fn new(kind: UopKind, steps: u8, pc: u32, a: u32, b: u32) -> Self {
        Uop {
            h: kind.0,
            steps,
            flags: 0,
            a,
            b,
            pc,
        }
    }
}

/// Call `finish_step` after the uop (loop end or delay-slot window close).
pub const UF_FIN: u8 = 1;
/// Call `latch_cb_overflow` after the uop (circular-buffer wrap of I7 or I15).
pub const UF_CB: u8 = 2;
/// Block terminator.
pub const UF_LAST: u8 = 4;
/// The uop runs on PEy as well (SIMD mode baked into the block).
pub const UF_SIMD: u8 = 8;

/// A handler index. Ranges are partitioned per family so that packages add constants in their
/// own files without touching each other: core `0x000..`, float `0x100..`, fixed `0x200..`,
/// moves `0x300..`, flow `0x400..`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UopKind(pub u16);

impl UopKind {
    /// Does nothing, accounts for its steps.
    pub const NOP: UopKind = UopKind(0x000);
    /// One instruction through the interpreter. `u.pc` is the instruction, `a` its ISA flag
    /// (0 or 1) and `b` the pc after it; the handler looks the instruction up exactly as
    /// `Core::step` does (`decode_cache.get`, then `fetch_and_cache`) and runs `exec_cached`.
    /// The design text says "a = index into a constant pool of a `CachedInsn`"; the pool cannot
    /// travel through `translate_range`, whose result has no pool, so the lookup is used (it is
    /// the cost the model of section 9.1 prices at 100 cycles).
    pub const INTERP: UopKind = UopKind(0x001);
    /// Terminator that runs `finish_step`.
    pub const FIN: UopKind = UopKind(0x002);
    /// Always `Exit::Slow`: the default of every unregistered slot.
    pub const BAIL: UopKind = UopKind(0x003);
    /// Size of the handler table.
    pub const MAX: u16 = 0x500;

    /// First index of the core family.
    pub const CORE_BASE: u16 = 0x000;
    /// First index of the float family (`exec/h_float.rs`, `lower/float.rs`).
    pub const FLOAT_BASE: u16 = 0x100;
    /// First index of the fixed-point family (`exec/h_fixed.rs`, `lower/fixed.rs`).
    pub const FIXED_BASE: u16 = 0x200;
    /// First index of the move family (`exec/h_move.rs`, `lower/moves.rs`).
    pub const MOVE_BASE: u16 = 0x300;
    /// First index of the flow family (`exec/h_flow.rs`, `lower/flow.rs`).
    pub const FLOW_BASE: u16 = 0x400;

    /// The family whose range holds this kind, if it is inside the table.
    pub const fn family(self) -> Option<KindFamily> {
        match self.0 {
            0x000..=0x0FF => Some(KindFamily::Core),
            0x100..=0x1FF => Some(KindFamily::Float),
            0x200..=0x2FF => Some(KindFamily::Fixed),
            0x300..=0x3FF => Some(KindFamily::Move),
            0x400..=0x4FF => Some(KindFamily::Flow),
            _ => None,
        }
    }
}

/// The five handler families, one range of the table each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KindFamily {
    Core,
    Float,
    Fixed,
    Move,
    Flow,
}

impl KindFamily {
    /// Every family, in table order.
    pub const ALL: [KindFamily; 5] = [
        KindFamily::Core,
        KindFamily::Float,
        KindFamily::Fixed,
        KindFamily::Move,
        KindFamily::Flow,
    ];

    /// Handler-table slots of the family.
    pub const fn range(self) -> Range<usize> {
        let base = match self {
            KindFamily::Core => UopKind::CORE_BASE,
            KindFamily::Float => UopKind::FLOAT_BASE,
            KindFamily::Fixed => UopKind::FIXED_BASE,
            KindFamily::Move => UopKind::MOVE_BASE,
            KindFamily::Flow => UopKind::FLOW_BASE,
        };
        base as usize..base as usize + 0x100
    }
}

/// Why a handler returned to the driver. The numeric values are part of the handler ABI.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// The uop finished; continue with the next one.
    Ok = 0,
    /// A terminator wrote `ctx.next_pc`.
    Jump = 1,
    /// The instruction would fault: materialize at `u.pc` and replay through `Core::step`.
    Fault = 2,
    /// A block-entry guard failed: no uop ran, one `Core::step`, retry.
    Guard = 3,
    /// A store hit a code page: materialize after the store, revalidate.
    Smc = 4,
    /// The event deadline moved or the uop needs the interpreter: materialize at `u.pc`, one
    /// `Core::step`, retry.
    Slow = 5,
}

impl Exit {
    /// Number of exit kinds (the length of `UopStats::exits`).
    pub const COUNT: usize = 6;

    /// The exit for a handler return value.
    pub const fn from_u32(v: u32) -> Option<Exit> {
        match v {
            0 => Some(Exit::Ok),
            1 => Some(Exit::Jump),
            2 => Some(Exit::Fault),
            3 => Some(Exit::Guard),
            4 => Some(Exit::Smc),
            5 => Some(Exit::Slow),
            _ => None,
        }
    }
}

/// The rows of the block-entry guard table (design 7.1), the index into `UopStats::guard_fail`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardFail {
    /// `steps + block.steps` is past `min(limit, irq.due)`.
    Steps = 0,
    /// Interrupt window open, core idle or stalled.
    Window = 1,
    /// A delayed branch is pending.
    Pending = 2,
    /// An active loop frame ends inside the block.
    Loop = 3,
    /// `core.simd()` differs from the mode the block was translated for.
    Simd = 4,
    /// Register trace or site counters active.
    Trace = 5,
    /// A circular-buffer wrap latch is set and the block has no `UF_CB` uop.
    CbWrap = 6,
    /// An arithmetic-exception interrupt is enabled and the block has uops that can raise it.
    Arith = 7,
    /// The block is poisoned or the ISA/VISA mode of its pc changed.
    Valid = 8,
}

impl GuardFail {
    /// Number of guard rows (the length of `UopStats::guard_fail`).
    pub const COUNT: usize = 9;
}

/// Context handed to every handler. Integers and one raw pointer only: handlers reach all state
/// through `core`, derived once from the `&mut Core` at `run_block` entry (aliasing discipline,
/// design section 14 risk 10).
#[repr(C)]
pub struct UopCtx {
    pub core: *mut Core,
    /// `core.steps` at block entry.
    pub base_steps: u64,
    /// Steps accounted so far in the block.
    pub done: u32,
    /// Steps left before `min(limit, irq.due)`.
    pub room: u32,
    /// Target pc written by a terminator.
    pub next_pc: u32,
    /// Index of the uop that raised a non-`Ok` exit.
    pub exit_idx: u32,
}

impl UopCtx {
    /// A fresh context for a block entered at `base_steps` with `room` steps before the deadline.
    pub fn new(core: *mut Core, base_steps: u64, room: u32) -> Self {
        UopCtx {
            core,
            base_steps,
            done: 0,
            room,
            next_pc: NONE,
            exit_idx: NONE,
        }
    }
}

/// A uop handler: runs one uop and returns an [`Exit`] as `u32`. Never panics.
pub type Handler = unsafe extern "C" fn(ctx: *mut UopCtx, u: *const Uop) -> u32;

/// The handler of [`UopKind::BAIL`] and the default of every unregistered table slot: leaves the
/// core untouched and asks the driver to run the instruction through `Core::step`.
///
/// # Safety
/// Dereferences nothing, so any arguments are fine; it is `unsafe` only to match [`Handler`].
pub unsafe extern "C" fn bail_handler(_ctx: *mut UopCtx, _u: *const Uop) -> u32 {
    Exit::Slow as u32
}

/// Strength of the entry guard a block needs (design 7.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GuardClass {
    #[default]
    Plain,
    Loop,
    Delayed,
    Cb,
    Arith,
}

/// Registers and state classes a block reads and writes (bit sets), used by the exit sweep and
/// by register promotion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Footprint {
    pub reads: u64,
    pub writes: u64,
    pub touches_mem: bool,
    pub touches_flags: bool,
}

/// A cached chain to another block, valid while `epoch` equals the engine epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    pub id: u32,
    pub epoch: u32,
}

impl Link {
    /// No link.
    pub const NONE: Link = Link { id: NONE, epoch: 0 };
}

/// A translated basic block: a range of the uop arena plus what the guards and exits need.
#[derive(Clone, Debug)]
pub struct Block {
    /// pc of the first instruction.
    pub pc: u32,
    /// pc of the last instruction.
    pub last_pc: u32,
    /// pc after the last instruction: where an `Exit::Ok` at the end of the block continues
    /// (fall-through successor). Terminators that jump write `UopCtx::next_pc` instead.
    pub end_pc: u32,
    /// Static step budget of the whole block.
    pub steps: u32,
    /// First uop in the arena and number of uops.
    pub first: u32,
    pub len: u32,
    /// Steps accounted before each uop (for mid-block re-entry and the exit sweep).
    pub cum: Box<[u16]>,
    /// Translated for PEy on or off.
    pub simd: bool,
    /// ISA-mode (48-bit word) code, from `isa_at` at translation time.
    pub isa: bool,
    pub guard: GuardClass,
    pub links: [Link; 2],
    /// Hash (or full copy for short blocks) of the code words the block was built from.
    pub code_hash: u64,
    /// Native table slot, [`NONE`] when there is none (present in every build).
    pub native: u32,
    /// Failed validation or repeated mismatch: run Tier 1 or the interpreter.
    pub poisoned: bool,
    pub footprint: Footprint,
}

impl Block {
    /// An empty block at `pc`.
    pub fn empty(pc: u32) -> Self {
        Block {
            pc,
            last_pc: pc,
            end_pc: pc,
            steps: 0,
            first: 0,
            len: 0,
            cum: Box::default(),
            simd: false,
            isa: false,
            guard: GuardClass::Plain,
            links: [Link::NONE; 2],
            code_hash: 0,
            native: NONE,
            poisoned: false,
            footprint: Footprint::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uop_is_16_bytes() {
        assert_eq!(core::mem::size_of::<Uop>(), 16);
        assert_eq!(core::mem::align_of::<Uop>(), 16);
        let u = Uop::new(UopKind::INTERP, 1, 0x12_0000, 0, 0x12_0001);
        assert_eq!((u.h, u.steps, u.flags, u.pc), (1, 1, 0, 0x12_0000));
    }

    #[test]
    fn families_partition_the_handler_table() {
        let mut next = 0;
        for f in KindFamily::ALL {
            let r = f.range();
            assert_eq!(r.start, next, "{f:?} starts where the previous ends");
            assert_eq!(r.len(), 0x100);
            next = r.end;
            assert_eq!(UopKind(r.start as u16).family(), Some(f));
            assert_eq!(UopKind(r.end as u16 - 1).family(), Some(f));
        }
        assert_eq!(next, UopKind::MAX as usize);
        assert_eq!(UopKind(UopKind::MAX).family(), None);
        for k in [UopKind::NOP, UopKind::INTERP, UopKind::FIN, UopKind::BAIL] {
            assert_eq!(k.family(), Some(KindFamily::Core));
        }
    }

    #[test]
    fn exit_codes_round_trip_and_bail_asks_for_the_interpreter() {
        for v in 0..Exit::COUNT as u32 {
            assert_eq!(Exit::from_u32(v).map(|e| e as u32), Some(v));
        }
        assert_eq!(Exit::from_u32(Exit::COUNT as u32), None);
        let r = unsafe { bail_handler(std::ptr::null_mut(), std::ptr::null()) };
        assert_eq!(Exit::from_u32(r), Some(Exit::Slow));
    }

    #[test]
    fn guard_rows_and_exit_kinds_size_the_stat_arrays() {
        assert_eq!(GuardFail::Valid as usize + 1, GuardFail::COUNT);
        let s = crate::core::UopStats::default();
        assert_eq!(
            (s.exits.len(), s.guard_fail.len()),
            (Exit::COUNT, GuardFail::COUNT)
        );
    }

    #[test]
    fn a_new_block_and_context_carry_no_links_and_no_native_code() {
        let b = Block::empty(0x12_0000);
        assert_eq!(
            (b.native, b.links[0], b.links[1]),
            (NONE, Link::NONE, Link::NONE)
        );
        assert_eq!(
            (b.steps, b.len, b.guard, b.poisoned),
            (0, 0, GuardClass::Plain, false)
        );
        let c = UopCtx::new(std::ptr::null_mut(), 7, 9);
        assert_eq!((c.base_steps, c.done, c.room, c.next_pc), (7, 0, 9, NONE));
    }
}
