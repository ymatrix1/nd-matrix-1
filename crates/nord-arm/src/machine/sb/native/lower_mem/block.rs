//! OWNER: W2-NMEM. Native `ldm` / `stm` (S bit and empty list stay `GENERIC`, `r15` is in no list
//! or base: I2). The rules are those of `exec_mem::block`: one address computation (`low`, the
//! lowest address, and the written-back base), the registers in ascending order, word accesses at
//! `(low + 4 * i) & !3`.
//!
//! The fast path checks the page entry of `low & !3` once and that the last word of the transfer
//! is still in that page (`end_span = 4 * n`), so the `n` accesses are plain loads or stores at
//! `host + 4 * i`; a transfer that crosses a page, or any page the interpreter must handle, leaves
//! for the interpreter at the pc of the instruction with nothing written.
//!
//! `LDM` writes the base back after the loads and not at all when the base is in the list; `STM`
//! writes it back after the store of the lowest register, so a base that is not the lowest
//! register of the list is stored updated (the register variable is read after the write-back).

use super::access::{data_flags, host_addr};
use super::mutant;
use crate::machine::sb::exec_mem::{F_P, F_U, F_WB};
use crate::machine::sb::native::emit::{NCtx, Unsupported};
use crate::machine::sb::uop::{k, Uop};
use cranelift_codegen::ir::{types, InstBuilder};

/// `ldm` / `stm`.
pub(super) fn lower_block(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
    let (rn, list) = (u.rn, u.imm);
    if rn >= 15 || list == 0 || list >> 15 != 0 {
        return Err(Unsupported);
    }
    let load = u.kind == k::LDM;
    let n = list.count_ones();
    let bytes = 4 * i64::from(n);
    let (up, pre) = (u.rs & F_U != 0, u.rs & F_P != 0);
    let wb = u.rs & F_WB != 0;
    let base = cx.reg(rn);
    let low = match (up, pre) {
        (true, false) => base,
        (true, true) => cx.b.ins().iadd_imm(base, 4),
        (false, false) => cx.b.ins().iadd_imm(base, 4 - bytes),
        (false, true) => {
            cx.b.ins()
                .iadd_imm(base, if mutant(867) { -bytes - 4 } else { -bytes })
        }
    };
    let wb_val = cx.b.ins().iadd_imm(base, if up { bytes } else { -bytes });
    let aligned = cx.b.ins().band_imm(low, -4);
    let span = (!mutant(864)).then_some(4 * n);
    let host = host_addr(cx, aligned, !load, span, u.pc);
    let fl = data_flags();
    let regs: Vec<u8> = (0..15u8).filter(|r| list >> *r & 1 != 0).collect();
    if load {
        let order: Vec<(usize, u8)> = if mutant(877) {
            regs.iter().rev().copied().enumerate().collect()
        } else {
            regs.iter().copied().enumerate().collect()
        };
        for (j, r) in order {
            let v = cx.b.ins().load(types::I32, fl, host, 4 * j as i32);
            cx.set_reg(r, v);
        }
        if wb && (list >> rn & 1 == 0 || mutant(866)) && !mutant(857) {
            cx.set_reg(rn, wb_val);
        }
    } else {
        if mutant(865) && wb {
            cx.set_reg(rn, wb_val);
        }
        for (j, r) in regs.iter().copied().enumerate() {
            let v = cx.reg(r);
            cx.b.ins().store(fl, v, host, 4 * j as i32);
            if wb && j == 0 && !mutant(857) {
                cx.set_reg(rn, wb_val);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod unit {
    use super::*;
    use crate::machine::sb::native::emit::testing::build_and_verify;
    use crate::machine::sb::native::lower_mem::tests::one;

    #[test]
    fn shapes_outside_the_contract_are_declined_with_nothing_emitted() {
        let sb = one(Uop::new(k::LDM, 0x1000));
        build_and_verify(&sb, |cx| {
            let before = cx.b.func.dfg.num_insts();
            for (rn, list) in [(15u8, 0b10u32), (1, 0), (1, 0x8000), (1, 0x1_0000)] {
                let mut u = Uop::new(k::LDM, 0x1000);
                (u.rn, u.imm) = (rn, list);
                assert_eq!(
                    lower_block(cx, &u),
                    Err(Unsupported),
                    "rn {rn} list {list:#x}"
                );
            }
            assert_eq!(cx.b.func.dfg.num_insts(), before);
            let mut ok = Uop::new(k::STM, 0x1000);
            (ok.rn, ok.imm, ok.sh) = (1, 0b1100, 2);
            assert_eq!(lower_block(cx, &ok), Ok(()));
            assert!(cx.b.func.dfg.num_insts() > before);
            cx.exit_generic(0x1000);
        });
    }
}
