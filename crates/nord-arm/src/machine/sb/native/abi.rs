//! OWNER: W1-NCORE. The binary interface between the driver and native code: `NativeCtl`, the
//! page-table masks and the field offsets of `Cpu`.
//!
//! Spec (design note 9, WP W1-NCORE): a compiled trace is `extern "C" fn(cpu: *mut Cpu, ctl: *mut
//! NativeCtl) -> u32` returning the exit index. Memory fast path: `e = table[a >> 12]`; a load
//! is allowed when `(e & MASK_LOAD) - 1 <u MASK_IDX` (mapped and no `HOOK_R`), a store when
//! `(e & MASK_STORE) - 1 <u MASK_IDX` (mapped, no `HOOK_W`, no `HOOK_C`). Offsets come from
//! `offset_of!` (stable, usable on `pub(crate)` fields) and tests compare them with a live `Cpu`.
//!
//! Provenance: all constants here are derived from the layout of `crate::mem` and
//! `crate::cpu::Cpu` in this repository, checked by the tests below (no firmware involved).

use crate::cpu::Cpu;
use crate::mem::raw::RawMem;
use core::mem::offset_of;

/// Entry mask of a load: keeps the arena index and `HOOK_R` (bit 31). With `- 1` and an unsigned
/// compare against [`MASK_IDX`] it is true exactly for a mapped page without `HOOK_R`.
pub const MASK_LOAD: u32 = 0x8FFF_FFFF;
/// Entry mask of a store: keeps the index, `HOOK_W` (bit 30) and `HOOK_C` (bit 28).
pub const MASK_STORE: u32 = 0x5FFF_FFFF;
/// The arena page index of a table entry.
pub const MASK_IDX: u32 = 0x0FFF_FFFF;

/// State shared between the driver and one native call (`repr(C)`: native code addresses the
/// fields by offset).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NativeCtl {
    /// Page table of the guest memory (`Mem::raw_parts`), refreshed before every entry.
    pub table: *const u32,
    /// Page arena, refreshed before every entry (it moves when Rust code maps a page).
    pub arena: *mut u8,
    /// Absolute `cpu.insns` the trace may not run past (a loop back-edge checks it).
    pub limit: u64,
    /// Index of the exit taken (written by the epilogue, also the return value).
    pub exit: u32,
    /// The new `r15`.
    pub target: u32,
    pub pad: [u32; 2],
}

impl NativeCtl {
    /// A control block for one entry: fresh memory pointers and the instruction limit.
    pub fn new(mem: RawMem, limit: u64) -> Self {
        Self {
            table: mem.table,
            arena: mem.arena,
            limit,
            exit: 0,
            target: 0,
            pad: [0; 2],
        }
    }
}

/// Offset of `Cpu::r[0]`; register `n` is at `OFF_R + 4 * n`.
pub const OFF_R: usize = offset_of!(Cpu, r);
/// Offsets of the flag bytes (`bool`, 0 or 1), indexed by the bit of the flag nibble
/// (`n << 3 | z << 2 | c << 1 | v`): `[v, c, z, n]`.
pub const OFF_FLAG: [usize; 4] = [
    offset_of!(Cpu, v),
    offset_of!(Cpu, c),
    offset_of!(Cpu, z),
    offset_of!(Cpu, n),
];
/// Offset of `Cpu::insns` (a `u64`).
pub const OFF_INSNS: usize = offset_of!(Cpu, insns);
/// Offset of `Cpu::thumb` (a `bool`): the `Indirect` exit of a `bx` writes it.
pub const OFF_THUMB: usize = offset_of!(Cpu, thumb);

/// Offsets of the `NativeCtl` fields.
pub const CTL_TABLE: usize = offset_of!(NativeCtl, table);
pub const CTL_ARENA: usize = offset_of!(NativeCtl, arena);
pub const CTL_LIMIT: usize = offset_of!(NativeCtl, limit);
pub const CTL_EXIT: usize = offset_of!(NativeCtl, exit);
pub const CTL_TARGET: usize = offset_of!(NativeCtl, target);

/// Exit index native code returns for "run this instruction through the interpreter": the state
/// is exact before the instruction and `r15` is its address.
pub const EXIT_GENERIC: u32 = u32::MAX;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::{HOOK_C, HOOK_R, HOOK_W};

    fn offset_of_field<T, F>(base: &T, field: &F) -> usize {
        field as *const F as usize - base as *const T as usize
    }

    #[test]
    fn the_masks_are_the_hook_flags_of_the_page_table() {
        assert_eq!(MASK_LOAD, HOOK_R | MASK_IDX);
        assert_eq!(MASK_STORE, HOOK_W | HOOK_C | MASK_IDX);
        assert_eq!(MASK_IDX, (1 << 28) - 1);
    }

    #[test]
    fn the_mask_tests_accept_exactly_a_mapped_page_without_the_blocking_flag() {
        let ok = |e: u32, mask: u32| (e & mask).wrapping_sub(1) < MASK_IDX;
        assert!(!ok(0, MASK_LOAD) && !ok(0, MASK_STORE), "unmapped");
        assert!(ok(1, MASK_LOAD) && ok(1, MASK_STORE));
        assert!(ok(MASK_IDX, MASK_LOAD), "the largest index still passes");
        assert!(!ok(1 | HOOK_R, MASK_LOAD) && ok(1 | HOOK_R, MASK_STORE));
        assert!(!ok(1 | HOOK_W, MASK_STORE) && ok(1 | HOOK_W, MASK_LOAD));
        assert!(!ok(1 | HOOK_C, MASK_STORE) && ok(1 | HOOK_C, MASK_LOAD));
        assert!(ok(1 | crate::mem::HOOK_X, MASK_LOAD) && ok(1 | crate::mem::HOOK_X, MASK_STORE));
    }

    #[test]
    fn the_cpu_offsets_match_a_live_cpu() {
        let cpu = Cpu::new();
        assert_eq!(OFF_R, offset_of_field(&cpu, &cpu.r));
        for n in 0..16 {
            assert_eq!(OFF_R + 4 * n, offset_of_field(&cpu, &cpu.r[n]));
        }
        let flags = [&cpu.v, &cpu.c, &cpu.z, &cpu.n];
        for (i, f) in flags.iter().enumerate() {
            assert_eq!(OFF_FLAG[i], offset_of_field(&cpu, *f));
        }
        assert_eq!(OFF_INSNS, offset_of_field(&cpu, &cpu.insns));
        assert_eq!(OFF_THUMB, offset_of_field(&cpu, &cpu.thumb));
        assert_eq!(core::mem::size_of::<bool>(), 1);
    }

    #[test]
    fn the_control_block_layout_is_the_documented_one() {
        let ctl = NativeCtl::new(
            RawMem {
                table: core::ptr::null(),
                arena: core::ptr::null_mut(),
            },
            5,
        );
        assert_eq!(core::mem::size_of::<NativeCtl>(), 8 + 8 + 8 + 4 + 4 + 8);
        assert_eq!(
            [CTL_TABLE, CTL_ARENA, CTL_LIMIT, CTL_EXIT, CTL_TARGET],
            [0, 8, 16, 24, 28]
        );
        assert_eq!(
            (ctl.limit, ctl.exit, ctl.target, ctl.pad),
            (5, 0, 0, [0, 0])
        );
        assert_eq!(offset_of_field(&ctl, &ctl.limit), CTL_LIMIT);
    }
}
