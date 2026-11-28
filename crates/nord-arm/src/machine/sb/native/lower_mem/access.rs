//! OWNER: W2-NMEM. The page-table fast path of every native memory access (design note 9,
//! "Memory"): the entry check, the page-end check, the side exit and the host address.
//!
//! [`host_addr`] emits, for the guest address `a` (already aligned the way the access wants it):
//!
//! ```text
//! e   = table[a >> 12]                      ; ctl.table, one u32 per page
//! t   = e & MASK_LOAD | MASK_STORE          ; the index and the blocking hook flags
//! bad = (t - 1) >=u MASK_IDX                ; unmapped (t == 0) or a blocking flag set
//!     | (a & 0xFFF) >u 0x1000 - span        ; only when the span can pass the page end
//! if bad { leave for the interpreter at the pc of the instruction }
//! host = arena - 4096 + ((e & MASK_IDX) << 12) + (a & 0xFFF)
//! ```
//!
//! The exit is taken before any register is written, so the state is exact before the
//! instruction. `end_span` is `Some(bytes)` when the uop touches more than one aligned unit
//! (`LDRD`/`STRD`: 8, `LDM`/`STM`: `4 * n`) and the access must stay in the page: an aligned
//! word, halfword or byte cannot pass the page end, so those pass `None` and no check is emitted.
//!
//! The loads of `ctl.table` and `ctl.arena` are `readonly` (native code never writes them, only
//! the driver does, before the entry): the optimiser may share them between accesses.

use super::mutant;
use crate::machine::sb::native::abi::{CTL_ARENA, CTL_TABLE, MASK_IDX, MASK_LOAD, MASK_STORE};
use crate::machine::sb::native::emit::NCtx;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, Endianness, InstBuilder, MemFlags, Value};

/// Flags of a guest-memory access: the host is little-endian like the guest, the address is
/// valid by the check, and guest data has no alignment guarantee on the host side.
pub(super) fn data_flags() -> MemFlags {
    MemFlags::new()
        .with_notrap()
        .with_endianness(Endianness::Little)
}

/// Emit the entry check and return the host address of the byte at guest address `a` (see the
/// module docs). `store` selects the mask, `pc` is the instruction to run in the interpreter
/// when the check fails.
pub(super) fn host_addr(
    cx: &mut NCtx<'_>,
    a: Value,
    store: bool,
    end_span: Option<u32>,
    pc: u32,
) -> Value {
    let ptr = cx.ptr();
    let ro = MemFlags::trusted().with_readonly();
    let page = cx.b.ins().ushr_imm(a, 12);
    let page = cx.b.ins().uextend(ptr, page);
    let slot = cx.b.ins().ishl_imm(page, 2);
    let table = cx.b.ins().load(ptr, ro, cx.ctl, CTL_TABLE as i32);
    let slot = cx.b.ins().iadd(table, slot);
    let e = cx.b.ins().load(types::I32, ro, slot, 0);
    let mask = match (store, mutant(850), mutant(851)) {
        (false, true, _) => MASK_LOAD & MASK_IDX,
        (true, _, true) => MASK_IDX,
        (false, ..) => MASK_LOAD,
        (true, ..) => MASK_STORE,
    };
    let t = cx.b.ins().band_imm(e, i64::from(mask as i32));
    let t = cx.b.ins().iadd_imm(t, -1);
    let mut bad =
        cx.b.ins()
            .icmp_imm(IntCC::UnsignedGreaterThanOrEqual, t, i64::from(MASK_IDX));
    let off = cx.b.ins().band_imm(a, 0xFFF);
    if let Some(span) = end_span {
        let slack = if mutant(873) { 4 } else { 0 };
        let last = i64::from(0x1000 - span) + slack;
        let over = cx.b.ins().icmp_imm(IntCC::UnsignedGreaterThan, off, last);
        bad = cx.b.ins().bor(bad, over);
    }
    let exit_pc = if mutant(872) { pc.wrapping_add(4) } else { pc };
    cx.side_exit_generic_if(bad, exit_pc);
    let index = cx.b.ins().band_imm(e, i64::from(MASK_IDX as i32));
    let index = cx.b.ins().uextend(ptr, index);
    let index = cx.b.ins().ishl_imm(index, 12);
    let arena = cx.b.ins().load(ptr, ro, cx.ctl, CTL_ARENA as i32);
    let host = cx.b.ins().iadd(arena, index);
    let off = cx.b.ins().uextend(ptr, off);
    let host = cx.b.ins().iadd(host, off);
    cx.b.ins().iadd_imm(host, -4096)
}

#[cfg(test)]
mod unit {
    use super::*;
    use crate::machine::sb::native::emit::testing::build_and_verify;
    use crate::machine::sb::native::lower_mem::tests::one;
    use crate::machine::sb::uop::{k, Uop};

    #[test]
    fn a_span_adds_the_page_end_check_and_a_store_uses_its_own_mask() {
        let sb = one(Uop::new(k::LDR_IMM, 0x1000));
        let count = |store: bool, span: Option<u32>| {
            let mut n = 0;
            build_and_verify(&sb, |cx| {
                let a = cx.reg(1);
                let before = cx.b.func.dfg.num_insts();
                host_addr(cx, a, store, span, 0x1000);
                n = cx.b.func.dfg.num_insts() - before;
                cx.exit_generic(0x1000);
            });
            n
        };
        assert!(
            count(false, Some(8)) > count(false, None),
            "icmp + bor for the span"
        );
        assert_eq!(
            count(false, None),
            count(true, None),
            "same shape, different mask"
        );
        assert_eq!(data_flags(), data_flags());
    }
}
