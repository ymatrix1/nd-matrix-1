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
//! Each is tagged VERIFIED with the test that checks it; the native tests (`native_access`)
//! compile a load and a store with these masks and compare them with `Mem` on every page kind.

use crate::cpu::Cpu;
use crate::mem::raw::RawMem;
use core::mem::offset_of;

/// Entry mask of a load: keeps the arena index and `HOOK_R` (bit 31). With `- 1` and an unsigned
/// compare against [`MASK_IDX`] it is true exactly for a mapped page without `HOOK_R`.
///
/// VERIFIED: `HOOK_R | MASK_IDX` by `the_masks_are_the_hook_flags_of_the_page_table`, the
/// accept/reject rule by `the_mask_tests_accept_exactly_a_mapped_page_without_the_blocking_flag`
/// and, run natively against `Mem` on six page kinds, by `native_access` (0 differences).
pub const MASK_LOAD: u32 = 0x8FFF_FFFF;
/// Entry mask of a store: keeps the index, `HOOK_W` (bit 30) and `HOOK_C` (bit 28).
///
/// VERIFIED: same tests as [`MASK_LOAD`]; a `HOOK_X` page passes (a store there is allowed, the
/// stale-code logic only needs `HOOK_C`).
pub const MASK_STORE: u32 = 0x5FFF_FFFF;
/// The arena page index of a table entry.
///
/// VERIFIED: `(1 << 28) - 1`, the bits below the four hook flags of `crate::mem`, by
/// `the_masks_are_the_hook_flags_of_the_page_table`.
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
///
/// VERIFIED: `offset_of!`, compared with the address of a live `Cpu`'s fields by
/// `the_cpu_offsets_match_a_live_cpu` (all 16 registers).
pub const OFF_R: usize = offset_of!(Cpu, r);
/// Offsets of the flag bytes (`bool`, 0 or 1), indexed by the bit of the flag nibble
/// (`n << 3 | z << 2 | c << 1 | v`): `[v, c, z, n]`.
///
/// VERIFIED: by `the_cpu_offsets_match_a_live_cpu`; a flag stored as anything but 0 or 1 would
/// be an invalid `bool`, so emitters store `uextend`ed comparisons only.
pub const OFF_FLAG: [usize; 4] = [
    offset_of!(Cpu, v),
    offset_of!(Cpu, c),
    offset_of!(Cpu, z),
    offset_of!(Cpu, n),
];
/// Offset of `Cpu::insns` (a `u64`).
///
/// VERIFIED: by `the_cpu_offsets_match_a_live_cpu`.
pub const OFF_INSNS: usize = offset_of!(Cpu, insns);
/// Offset of `Cpu::thumb` (a `bool`): the `Indirect` exit of a `bx` writes it.
///
/// VERIFIED: by `the_cpu_offsets_match_a_live_cpu` and, through the exit, by
/// `register_target_exits_equal_the_tier2_executor_on_random_registers`.
pub const OFF_THUMB: usize = offset_of!(Cpu, thumb);

/// Offsets of the `NativeCtl` fields.
///
/// VERIFIED: `offset_of!` of a `repr(C)` struct, pinned to `[0, 8, 16, 24, 28]` on 64-bit hosts
/// by `the_control_block_layout_is_the_documented_one`.
pub const CTL_TABLE: usize = offset_of!(NativeCtl, table);
pub const CTL_ARENA: usize = offset_of!(NativeCtl, arena);
pub const CTL_LIMIT: usize = offset_of!(NativeCtl, limit);
pub const CTL_EXIT: usize = offset_of!(NativeCtl, exit);
pub const CTL_TARGET: usize = offset_of!(NativeCtl, target);

/// Exit index native code returns for "run this instruction through the interpreter": the state
/// is exact before the instruction and `r15` is its address.
///
/// VERIFIED: never a valid exit index (`uop::MAX_EXITS` is 64), checked by
/// `the_generic_exit_value_is_not_an_exit_index`.
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

    #[test]
    fn the_generic_exit_value_is_not_an_exit_index() {
        assert!(EXIT_GENERIC as usize > crate::machine::sb::uop::MAX_EXITS);
        assert_eq!(EXIT_GENERIC, u32::MAX);
    }
}

#[cfg(test)]
mod native_access {
    //! The fast path of design note 9, emitted by hand with the constants of this file and run
    //! against `Mem`: the proof that masks, offsets and the host-address formula
    //! `arena - 4096 + (index << 12) + (addr & 0xFFF)` are what `Mem` does.

    use super::*;
    use crate::cpu::Cpu;
    use crate::machine::sb::native::available;
    use crate::machine::sb::native::emit::testing::compile;
    use crate::machine::sb::native::emit::NCtx;
    use crate::machine::sb::native::rt::Jit;
    use crate::machine::sb::uop::{Exit, ExitKind, Sb, Uop};
    use crate::mem::{Mem, HOOK_C, HOOK_R, HOOK_W, HOOK_X};
    use cranelift_codegen::ir::condcodes::IntCC;
    use cranelift_codegen::ir::{types, InstBuilder, MemFlags};

    /// Where a missed access leaves.
    const MISS_PC: u32 = 0x1234;

    /// `r0 = [r1]` (load) or `[r1] = r2` (store) through the page table, else `exit_generic`.
    fn emit_access(cx: &mut NCtx<'_>, store: bool) {
        let (ptr, m) = (cx.ptr(), MemFlags::trusted());
        let addr = cx.reg(1);
        let page = cx.b.ins().ushr_imm(addr, 12);
        let page = cx.b.ins().uextend(ptr, page);
        let off = cx.b.ins().ishl_imm(page, 2);
        let table = cx.b.ins().load(ptr, m, cx.ctl, CTL_TABLE as i32);
        let slot = cx.b.ins().iadd(table, off);
        let e = cx.b.ins().load(types::I32, m, slot, 0);
        let mask = if store { MASK_STORE } else { MASK_LOAD };
        let t = cx.b.ins().band_imm(e, i64::from(mask as i32));
        let t = cx.b.ins().iadd_imm(t, -1);
        let ok =
            cx.b.ins()
                .icmp_imm(IntCC::UnsignedLessThan, t, i64::from(MASK_IDX));
        let (hit, miss) = (cx.b.create_block(), cx.b.create_block());
        cx.b.ins().brif(ok, hit, &[], miss, &[]);
        cx.b.seal_block(miss);
        cx.b.switch_to_block(miss);
        cx.exit_generic(MISS_PC);
        cx.b.seal_block(hit);
        cx.b.switch_to_block(hit);
        let idx = cx.b.ins().band_imm(e, i64::from(MASK_IDX));
        let idx = cx.b.ins().uextend(ptr, idx);
        let base = cx.b.ins().ishl_imm(idx, 12);
        let arena = cx.b.ins().load(ptr, m, cx.ctl, CTL_ARENA as i32);
        let host = cx.b.ins().iadd(arena, base);
        let host = cx.b.ins().iadd_imm(host, -4096);
        let lo = cx.b.ins().band_imm(addr, 0xFFF);
        let lo = cx.b.ins().uextend(ptr, lo);
        let host = cx.b.ins().iadd(host, lo);
        if store {
            let v = cx.reg(2);
            cx.b.ins().store(m, v, host, 0);
        } else {
            let v = cx.b.ins().load(types::I32, m, host, 0);
            cx.set_reg(0, v);
        }
        cx.exit(0).unwrap();
    }

    #[test]
    fn a_native_page_table_access_agrees_with_mem_on_every_page_kind() {
        if !available() {
            assert!(Jit::new().is_none() || !crate::machine::sb::native::rt::probe());
            return;
        }
        let mut jit = Jit::new().unwrap();
        let exit = Exit::new(ExitKind::Direct, 1, 0x1000, 0x1004);
        let uops = vec![Uop::exit(0, 0x1000)];
        let sb = Sb::from_parts(0x1000, 1, uops, vec![exit], vec![]).unwrap();
        let load = compile(&mut jit, &sb, |cx| emit_access(cx, false)).unwrap();
        let store = compile(&mut jit, &sb, |cx| emit_access(cx, true)).unwrap();
        // page kind: (address, flag, mapped, loads natively, stores natively); None = plain,
        // and the unmapped page is never written
        let kinds: [(u32, Option<u32>, bool, bool, bool); 7] = [
            (0x8000_0000, None, true, true, true),
            (0x8000_1000, Some(HOOK_R), true, false, true),
            (0x8000_2000, None, false, false, false),
            (0x8000_3000, Some(HOOK_X), true, true, true),
            (0x8000_4000, Some(HOOK_W), true, true, false),
            (0x8000_5000, Some(HOOK_C), true, true, false),
            (0xFFFF_F000, None, true, true, true),
        ];
        let mut mem = Mem::new();
        for (i, (base, flag, mapped, ..)) in kinds.iter().enumerate() {
            if *mapped {
                mem.write32(base + 0x24, 0xA000_0000 + i as u32);
            }
            match *flag {
                Some(HOOK_C) => mem.mark_code(*base),
                Some(f) => mem.add_flag(*base, *base, f),
                None => {}
            }
        }
        let (mut loads, mut stores, mut misses) = (0, 0, 0);
        for round in 0..2 {
            for (i, (base, flag, mapped, load_ok, store_ok)) in kinds.iter().enumerate() {
                let addr = base + 0x24;
                let mut cpu = Cpu::new();
                cpu.r[1] = addr;
                cpu.r[0] = 0x5555_5555;
                let out = jit.run(&load, &mut cpu, &mut mem, 0);
                if *load_ok {
                    assert_eq!(
                        (out.exit, cpu.r[0]),
                        (0, 0xA000_0000 + i as u32),
                        "load {i} {flag:?}"
                    );
                    loads += 1;
                } else {
                    assert!(out.is_generic() && out.target == MISS_PC, "load {i}");
                    assert_eq!(cpu.r[0], 0x5555_5555, "a missed load changes nothing");
                    misses += 1;
                }
                cpu.r[2] = 0xDEAD_0000 + i as u32;
                let out = jit.run(&store, &mut cpu, &mut mem, 0);
                let seen = if *mapped { mem.read32(addr) } else { 0 };
                if *store_ok {
                    assert_eq!((out.exit, seen), (0, 0xDEAD_0000 + i as u32), "store {i}");
                    stores += 1;
                    mem.write32(addr, 0xA000_0000 + i as u32); // restore for the next round
                } else {
                    assert!(out.is_generic() && out.target == MISS_PC, "store {i}");
                    assert_eq!(seen, if *mapped { 0xA000_0000 + i as u32 } else { 0 });
                    misses += 1;
                }
            }
            // the second round runs after the arena was reallocated by many new pages
            if round == 0 {
                for page in 0..300u32 {
                    mem.write32(0x9000_0000 + page * 0x1000, page);
                }
            }
        }
        assert!(
            mem.stale_code.is_empty(),
            "no native store touched a code page"
        );
        assert_eq!(
            (loads, stores, misses),
            (10, 8, 10),
            "5 loads, 4 stores and 5 misses per round"
        );
    }
}
