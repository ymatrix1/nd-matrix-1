//! OWNER: W2-NMEM. Native lowering of control uops (kinds `144..=154` and the fused kinds
//! `160..=164`).
//!
//! Spec (design note 4.3 and 9, WP W2-NMEM). The arms are those of `exec_flow.rs`:
//!
//! | Kind | Native code |
//! |---|---|
//! | `PRED` | `NCtx::predicated`: a branch on the condition, the next `imm` uops in the body block, a join block; native only when the whole body lowers, else `Unsupported` at the `PRED` (its state is exact before the first instruction of the body) |
//! | `GUARD` | `brif cond, side, cont`; the side block leaves through exit `rs` (`NCtx::exit`: dirty state stored, `insns += done`, `r15` the target, the exit index returned) |
//! | `GUARD_REG_EQ` | the same side exit when `r[rn] != imm` (an inlined return that went elsewhere) |
//! | `LOOP_BACK` | `NCtx::loop_back`: `acc += done`, jump to the loop header while `acc + n_guest <= ctl.limit`, else leave through the exit; with `ctl.limit` at or below the entry `cpu.insns` the first back-edge always leaves, which is the watch exit of idle v2 (T2: `Lap::watch`) |
//! | `EXIT`, `EXIT_REG`, `EXIT_REG_ARM` | `NCtx::exit`: a static target, `bx` interworking, `mov pc,rm` |
//! | `CMP_GUARD_*`, `TST_GUARD_I`, `MOV_PAIR`, `LDR_PAIR` | `fused.rs`: the unfused expansion |
//! | `GENERIC`, `GENERIC_END`, `MRS`, `MSR_C` | always `Unsupported`: the point where native code hands over to the interpreter (a `GENERIC_END` leaves through its `Dynamic` exit, which stays driver-side) |
//!
//! Budget accounting is `NCtx`'s: every exit adds the static `done` of its exit (to the iteration
//! accumulator in a loop), a side exit of a memory uop adds the instructions retired before it.
//! An exit kind native code cannot take (`Dynamic`, an indirect exit through `r15`, a
//! `LoopBack` exit used by anything but `LOOP_BACK`) makes its uop `Unsupported` with nothing
//! emitted. `cond` outside `0..=15` is not a condition and is `Unsupported` too.
//!
//! The mutants `880..=899` belong to this module and its children ([`MUTANTS`]); the
//! memory lowerer's are `850..=879` (`lower_mem::MUTANTS`). `lower_mem/tests.rs` kills them all.

use super::emit::{NCtx, Unsupported};
use crate::machine::sb::uop::{k, ExitKind, Uop};
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::InstBuilder;

mod cond;
mod fused;

#[cfg(test)]
pub(crate) mod suite;
#[cfg(test)]
mod tests;

use super::lower_mem::mutant;

/// `(id, name)` of the mutation sites of the flow lowerer (block `880..=899` of `mutants.rs`).
#[cfg(test)]
pub(crate) const MUTANTS: &[(u32, &str)] = &[
    (
        880,
        "native flow: a guard takes its exit when the condition fails",
    ),
    (
        881,
        "native flow: guard_reg_eq leaves when the register is equal",
    ),
    (
        882,
        "native flow: a predicated body runs when the condition fails",
    ),
    (883, "native flow: HI is C or Z"),
    (884, "native flow: GE is N != V"),
    (
        885,
        "native flow: loop_back leaves for the interpreter instead of looping",
    ),
    (
        887,
        "native flow: the second half of a fused pair retires nothing before it",
    ),
    (888, "native flow: LS is not C and Z"),
    (889, "native flow: GT ignores Z"),
    (890, "native flow: LE is Z and N != V"),
    (891, "native flow: CS is not C"),
    (
        892,
        "native flow: guard_reg_eq compares the low halfword only",
    ),
    (893, "native flow: a predicated body is one uop short"),
];

/// Whether `NCtx::exit(idx)` leaves natively through exit `idx` of the trace: the exit exists and
/// is a static one or an indirect one through a register that is a variable. A `LoopBack` exit
/// belongs to `LOOP_BACK` (leaving through it from a guard is a plain exit, not the back-edge).
fn plain_exit(cx: &NCtx<'_>, idx: usize) -> bool {
    cx.sb().exits.get(idx).is_some_and(|e| match e.kind {
        ExitKind::Direct | ExitKind::Fault | ExitKind::Stale | ExitKind::Watch => true,
        ExitKind::Indirect | ExitKind::IndirectArm => e.reg < 15,
        ExitKind::Dynamic | ExitKind::LoopBack => false,
    })
}

/// `if cond != 0 { leave through exit idx }`; emission continues in the fall-through block.
fn exit_if(cx: &mut NCtx<'_>, cond: cranelift_codegen::ir::Value, idx: usize) {
    let (side, cont) = (cx.b.create_block(), cx.b.create_block());
    cx.b.ins().brif(cond, side, &[], cont, &[]);
    cx.b.seal_block(side);
    cx.b.switch_to_block(side);
    cx.exit(idx)
        .expect("plain_exit checked that this exit is native");
    cx.b.seal_block(cont);
    cx.b.switch_to_block(cont);
}

/// Emit the native code of `u` (control uops).
pub fn lower_flow(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
    match u.kind {
        k::PRED => {
            if !cond::is_cond(u.cond) {
                return Err(Unsupported);
            }
            let c = u.cond;
            let len = (u.imm as usize).saturating_sub(usize::from(mutant(893)));
            cx.predicated(len, |cx| {
                let v = cond::cond_value(cx, c).expect("is_cond checked");
                if mutant(882) {
                    cx.b.ins().bxor_imm(v, 1)
                } else {
                    v
                }
            })
        }
        k::GUARD => {
            let idx = u.exit_idx();
            if !cond::is_cond(u.cond) || !plain_exit(cx, idx) {
                return Err(Unsupported);
            }
            let v = cond::cond_value(cx, u.cond).expect("is_cond checked");
            let v = if mutant(880) {
                cx.b.ins().bxor_imm(v, 1)
            } else {
                v
            };
            exit_if(cx, v, idx);
            Ok(())
        }
        k::GUARD_REG_EQ => {
            let idx = u.exit_idx();
            if u.rn >= 15 || !plain_exit(cx, idx) {
                return Err(Unsupported);
            }
            let mut r = cx.reg(u.rn);
            let mut want = u.imm;
            if mutant(892) {
                r = cx.b.ins().band_imm(r, 0xFFFF);
                want &= 0xFFFF;
            }
            let cc = if mutant(881) {
                IntCC::Equal
            } else {
                IntCC::NotEqual
            };
            let differs = cx.b.ins().icmp_imm(cc, r, i64::from(want as i32));
            exit_if(cx, differs, idx);
            Ok(())
        }
        k::LOOP_BACK => {
            if mutant(885) && cx.in_loop() {
                cx.exit_generic(u.pc);
                return Ok(());
            }
            cx.loop_back(u.exit_idx())
        }
        k::EXIT | k::EXIT_REG | k::EXIT_REG_ARM => {
            let idx = u.exit_idx();
            if !plain_exit(cx, idx) {
                return Err(Unsupported);
            }
            cx.exit(idx)
        }
        k::CMP_GUARD_I | k::CMP_GUARD_R | k::TST_GUARD_I | k::MOV_PAIR | k::LDR_PAIR => {
            fused::lower_fused(cx, u)
        }
        _ => Err(Unsupported),
    }
}

#[cfg(test)]
mod unit {
    use super::*;
    use crate::machine::sb::native::emit::testing::build_and_verify;
    use crate::machine::sb::uop::{Exit, Sb};

    #[test]
    fn only_static_and_register_indirect_exits_are_native() {
        let mut exits = Vec::new();
        for kind in [
            ExitKind::Direct,
            ExitKind::Fault,
            ExitKind::Stale,
            ExitKind::Watch,
            ExitKind::Indirect,
            ExitKind::IndirectArm,
            ExitKind::Dynamic,
            ExitKind::LoopBack,
        ] {
            let mut e = Exit::new(kind, 1, 0x1000, 0x2000);
            e.reg = 3;
            exits.push(e);
        }
        let mut via_pc = Exit::new(ExitKind::Indirect, 1, 0x1000, 0x2000);
        via_pc.reg = 15;
        exits.push(via_pc);
        let sb = Sb::from_parts(
            0x1000,
            1,
            vec![Uop::exit(0, 0x1000)],
            exits,
            vec![(0x1000, 0)],
        )
        .unwrap();
        build_and_verify(&sb, |cx| {
            let native: Vec<bool> = (0..10).map(|i| plain_exit(cx, i)).collect();
            assert_eq!(
                native,
                [true, true, true, true, true, true, false, false, false, false]
            );
            cx.exit_generic(0x1000);
        });
    }
}
