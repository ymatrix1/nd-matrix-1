//! Which move instructions become a fast uop, and which kind, operands and flags each gets.
//!
//! OWNER: W2-MOVE. The decision follows the table of `exec/h_move/layout.rs`:
//!
//! | instruction | uop |
//! |---|---|
//! | `DM/PM` load or store, types 3a/3b/3c, 4a/4b, 14a, 15a/15b | a transfer kind: (write, address mode, register class) |
//! | `Un = <imm>`, type 17a/17b | `LOAD_IMM` + class |
//! | `DM/PM(Ii, Mm) = <imm>`, type 16a/16b | `IMM_STORE` |
//! | `MODIFY(Is, Mm)`, type 7a/7b | `MODIFY` |
//! | `MODIFY` / `BITREV(Is, <imm>)`, type 19a | `MODIFY_IMM`, `BITREV_IMM` |
//! | `Ureg = Ureg`, `Rr <-> Ss`, type 5a/5b | `REG_MOVE`, `REG_SWAP` |
//! | dual data move, type 1a/1b | `DUAL` |
//!
//! Taken only when the handler can reproduce the interpreter exactly: unconditional (`IF TRUE`,
//! cond 31), no long-word access, no compute in parallel (the compute field is 0), VISA code,
//! and registers below the system registers (`SYS_UREG`: their reads and writes depend on `pc`,
//! the step count and the interrupt state, and a write can change what a block assumes). A
//! move with a compute stays an `Interp` uop until the compute families publish a protocol to run
//! their handler next to a move uop (reported as a need of W2-MOVE).
//!
//! Flags: `UF_SIMD` on every uop of a PEy block, `UF_CB` exactly on the uops that can wrap the
//! circular buffer of `I7` or `I15` (a post-modify, `MODIFY` or dual move through that index), so
//! the dispatcher latches `CB7I` / `CB15I` after them and the translator ends the block there.
//! The instruction length goes to the top bits of operand `a` (`mem::LEN_SHIFT`): a handler that
//! leaves the block early needs the pc after the instruction.
use crate::core::aot_mem::{Addr, Data, MemXfer};
use crate::core::decode_cache::{CachedInsn, DecodedOp, DualMoveOp, MoveOp};
use crate::core::uop::exec::h_move::layout::{
    class_of, dual_a, load_imm_kind, pair_a, xfer_a, xfer_kind, Side, BITREV_IMM, CLASS_R, DIRECT,
    DUAL, IMM_STORE, MODIFY, MODIFY_IMM, POST_IMM, POST_M, PRE_IMM, PRE_M, REG_MOVE, REG_SWAP,
};
use crate::core::uop::lower::LowerCtx;
use crate::core::uop::types::{Uop, UopKind, UF_CB, UF_SIMD};
use crate::core::Core;
use crate::visa::{bits, Kind};

/// Index registers whose circular-buffer wrap raises an interrupt (`CB7I`, `CB15I`).
const fn wraps_interrupt(ii: u8) -> bool {
    ii == 7 || ii == 15
}

/// Whether the instruction has no compute in parallel: the kinds with a compute field need it to
/// be 0 (`ComputeOp::Nop`), the other kinds have none.
fn compute_free(kind: Kind, w: u64) -> bool {
    !matches!(
        kind,
        Kind::T1a | Kind::T3a | Kind::T4a | Kind::T5a | Kind::T7a
    ) || bits(w, 22, 0) == 0
}

/// The uop of `c`, `None` when the instruction stays with the interpreter.
pub(super) fn lowered(cx: &LowerCtx<'_>, c: &CachedInsn) -> Option<Uop> {
    if c.isa || cx.isa {
        return None;
    }
    let len = c
        .next_pc
        .checked_sub(c.pc)
        .filter(|n| (1..=3).contains(n))?;
    if !compute_free(c.insn.kind, c.w) {
        return None;
    }
    let (kind, a, b, cb) = shape(c, len)?;
    let mut u = Uop::new(kind, 1, c.pc, a, b);
    if cx.simd {
        u.flags |= UF_SIMD;
    }
    if cb {
        u.flags |= UF_CB;
    }
    Some(u)
}

/// `(kind, a, b, can wrap I7/I15)` of the instruction.
fn shape(c: &CachedInsn, len: u32) -> Option<(UopKind, u32, u32, bool)> {
    match c.op {
        DecodedOp::ModifyImm => return modify_imm(c.w, len),
        DecodedOp::DualMove(dm) => return dual(dm, len),
        DecodedOp::Move(MoveOp::RegMove {
            src, dst, cond: 31, ..
        }) => {
            class_of(src)?;
            class_of(dst)?;
            return Some((REG_MOVE, pair_a(src, dst, 8, len), 0, false));
        }
        DecodedOp::Move(MoveOp::RegSwap { r, s, cond: 31, .. }) => {
            return Some((REG_SWAP, pair_a(r, s, 4, len), 0, false));
        }
        DecodedOp::Move(MoveOp::LoadImm { ureg, value }) => {
            let kind = load_imm_kind(class_of(ureg)?);
            return Some((kind, xfer_a(0, 0, ureg, len), value, false));
        }
        _ => {}
    }
    transfer(&MemXfer::from_insn(&c.insn, c.w)?, len)
}

/// The uop of a decoded memory transfer or index modify.
fn transfer(x: &MemXfer, len: u32) -> Option<(UopKind, u32, u32, bool)> {
    let ii = x.ii;
    let (mode, mi, imm) = match x.addr {
        Addr::PostM(m) => (POST_M, m, 0),
        Addr::PostImm(d) => (POST_IMM, 0, d),
        Addr::PreM(m) => (PRE_M, m, 0),
        Addr::PreImm(d) => (PRE_IMM, 0, d),
        Addr::Direct(a) => (DIRECT, 0, a),
        Addr::None => return None,
    };
    let post = mode == POST_M || mode == POST_IMM;
    match x.data {
        Data::Reg { ureg, write } => {
            let class = class_of(ureg)?;
            // type 4 has a 4-bit register field: no such instruction, and no handler either
            if mode == POST_IMM && class != CLASS_R {
                return None;
            }
            let kind = xfer_kind(write, mode, class);
            Some((
                kind,
                xfer_a(ii, mi, ureg, len),
                imm,
                post && wraps_interrupt(ii),
            ))
        }
        Data::Imm(v) => Some((IMM_STORE, xfer_a(ii, mi, 0, len), v, wraps_interrupt(ii))),
        Data::Modify { dst, src } => {
            Some((MODIFY, xfer_a(src, dst, mi, len), 0, wraps_interrupt(src)))
        }
        _ => None,
    }
}

/// Type 19a, fields as `Core::exec_modify_imm` reads them.
fn modify_imm(w: u64, len: u32) -> Option<(UopKind, u32, u32, bool)> {
    let bank = Core::dag_bank(bits(w, 38, 38)) as u8;
    let src = bits(w, 34, 32) as u8 + bank;
    let dst = (bits(w, 37, 35) ^ bits(w, 34, 32)) as u8 + bank;
    let a = xfer_a(src, dst, 0, len);
    let data = bits(w, 31, 0) as u32;
    if bits(w, 39, 39) == 1 {
        Some((BITREV_IMM, a, data, false))
    } else {
        Some((MODIFY_IMM, a, data, wraps_interrupt(src)))
    }
}

/// Types 1a / 1b (the compute was checked to be absent).
fn dual(d: DualMoveOp, len: u32) -> Option<(UopKind, u32, u32, bool)> {
    let side = |i, m, reg, write| Side { i, m, reg, write };
    let dm = side(d.dmi, d.dmm, d.dm_reg, d.dm_write);
    let pm = side(d.pmi, d.pmm, d.pm_reg, d.pm_write);
    Some((
        DUAL,
        dual_a(dm, pm, len),
        0,
        wraps_interrupt(d.dmi) || wraps_interrupt(d.pmi),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::uop::config::UopConfig;
    use crate::core::uop::exec::h_move::layout::{
        a_ii, a_mi, a_ureg, dual_sides, CLASS_OTHER, CLASS_R, CLASS_S,
    };
    use crate::core::uop::lower::decode::decode_at;

    /// The uop the move family makes of the first instruction of `program`.
    fn lower_one(program: &[u16], simd: bool) -> Option<Uop> {
        let mut core = core_with(program);
        let pc = core.pc;
        let insn = decode_at(&mut core, pc);
        let cfg = UopConfig::default();
        let cx = LowerCtx {
            cfg: &cfg,
            stop_pc: None,
            simd,
            isa: false,
        };
        lowered(&cx, &insn)
    }

    #[test]
    fn a_dag_load_picks_mode_and_class_and_keeps_the_length_in_a() {
        // R0 = DM(I2, M2): type 3c, post-modify (see moves.rs tests)
        let p = (0b1001u16 << 12) | (2 << 9) | (2 << 6) | (1 << 4);
        let u = lower_one(&[p], false).unwrap();
        assert_eq!(UopKind(u.h), xfer_kind(false, POST_M, CLASS_R));
        assert_eq!((a_ii(u.a), a_mi(u.a), a_ureg(u.a)), (2, 2, 0));
        assert_eq!((u.a >> 30, u.steps, u.flags, u.b), (1, 1, 0, 0));
        // the store form and SIMD
        let u = lower_one(&[p | 0x20], true).unwrap();
        assert_eq!(UopKind(u.h), xfer_kind(true, POST_M, CLASS_R));
        assert_eq!(u.flags, UF_SIMD);
    }

    #[test]
    fn a_register_class_decides_the_kind_and_a_system_register_is_declined() {
        // type 15b `<ureg> = DM(0, I7)` with the register field patched (9e08 0c00: I8 = DM(0, I7))
        let with = |ureg: u16| [0x9e08, ureg << 7];
        let class = |ureg| lower_one(&with(ureg), false).map(|u| UopKind(u.h));
        assert_eq!(class(0x03), Some(xfer_kind(false, PRE_IMM, CLASS_R)));
        assert_eq!(class(0x53), Some(xfer_kind(false, PRE_IMM, CLASS_S)));
        assert_eq!(class(0x18), Some(xfer_kind(false, PRE_IMM, CLASS_OTHER)));
        assert_eq!(class(0x60), None, "system registers stay Interp");
        assert_eq!(class(0x7F), None);
    }

    #[test]
    fn what_the_handlers_cannot_reproduce_stays_with_the_interpreter() {
        assert!(lower_one(&[0x0001], false).is_none(), "NOP");
        assert!(lower_one(&[0x013e, 0x0000, 0x1312], false).is_none(), "ALU");
        // long-word store DM(0x92196) = R0 (LW)
        assert!(lower_one(&[0x1180, 0x0009, 0x2196], false).is_none(), "LW");
        assert!(
            lower_one(&[0x1100, 0x0009, 0x2196], false).is_some(),
            "the plain store"
        );
        // R3 = DM(...) with a compute (type 4a): 6a3f 218d a00f
        assert!(
            lower_one(&[0x6a3f, 0x218d, 0xa00f], false).is_none(),
            "compute"
        );
        // IF NE R1 = DM(I6, 2) (type 4b, cond field patched): not unconditional
        assert!(lower_one(&[0x6c7e, 0x10bf], false).is_some());
        assert!(lower_one(&[0x6c7e & !(0x1F << 1) | (0x10 << 1), 0x10bf], false).is_none());
    }

    #[test]
    fn only_a_post_modify_through_i7_or_i15_can_wrap() {
        // type 4b R1 = DM(I6, 2) post (6c7e 10bf): index bits 43..41 and bank bit 40 of the first
        // parcel, the U bit 38 (0x40) selects the post-modify
        let flag = |i: u16, bank: u16, post: bool| {
            let p0 = (0x6c7e & !(7 << 9) & !(1 << 8)) | i << 9 | bank << 8;
            let p0 = if post { p0 } else { p0 & !0x40 };
            lower_one(&[p0, 0x10bf], false).map(|u| u.flags & UF_CB != 0)
        };
        assert_eq!(flag(7, 0, true), Some(true), "I7 post-modify");
        assert_eq!(flag(7, 1, true), Some(true), "I15 post-modify");
        assert_eq!(flag(6, 0, true), Some(false));
        assert_eq!(flag(6, 1, true), Some(false), "I14");
        assert_eq!(
            flag(7, 0, false),
            Some(false),
            "pre-modify never updates the index"
        );
        assert_eq!(flag(7, 1, false), Some(false));
    }

    #[test]
    fn immediates_and_register_moves_have_their_own_kinds() {
        // R14 = 0x3eb0f34d? type 17a: 0f0e 3eb0 f34d
        let u = lower_one(&[0x0f0e, 0x3eb0, 0xf34d], false).unwrap();
        assert_eq!(UopKind(u.h), load_imm_kind(CLASS_R));
        assert_eq!((a_ureg(u.a), u.a >> 30, u.b), (0x0e, 3, 0x3eb0_f34d));
        // type 17b M1 = 1 (16-bit immediate, sign extended)
        let u = lower_one(&[0x0fa1, 0x0001], false).unwrap();
        assert_eq!((UopKind(u.h), u.b), (load_imm_kind(CLASS_OTHER), 1));
        // a system register is declined: 0x6A
        assert!(lower_one(&[0x0F80 | 0x6A, 0x0001], false).is_none());
        // M6 = R1 (type 5b) and the swap with a compute (type 5a, not taken)
        let u = lower_one(&[0x703e, 0x933f], false).unwrap();
        assert_eq!(UopKind(u.h), REG_MOVE);
        assert_eq!(u.a & 0x3FFF_FFFF, 1 | 0x26 << 8);
        // R9 <-> S8 without a compute is taken, with one (7a3e 0480 1001) it is not
        let u = lower_one(&[0x7a3e, 0x0480, 0x0000], false).unwrap();
        assert_eq!((UopKind(u.h), u.a & 0xFF), (REG_SWAP, 9 | 8 << 4));
        assert!(lower_one(&[0x7a3e, 0x0480, 0x1001], false).is_none());
    }

    #[test]
    fn modifies_and_the_dual_move_carry_their_operands() {
        // type 7b: Id = MODIFY(Is, Mm) (043e 583f)
        let u = lower_one(&[0x043e, 0x583f], false).unwrap();
        assert_eq!(UopKind(u.h), MODIFY);
        assert_eq!(
            (a_ii(u.a), a_mi(u.a), u.a >> 8 & 15),
            (1, 1, 3),
            "src, dst, M3"
        );
        // type 19a: I0 = MODIFY(I0, 0x28) = 1600 0000 0028, and the BITREV form
        let u = lower_one(&[0x1600, 0x0000, 0x0028], false).unwrap();
        assert_eq!((UopKind(u.h), u.b, u.flags), (MODIFY_IMM, 0x28, 0));
        let u = lower_one(&[0x1600 | 0x80, 0x0000, 0x0028], false).unwrap();
        assert_eq!(UopKind(u.h), BITREV_IMM);
        // type 1b: dm(i6,m6)=r4, r0=pm(i15,m9) (3d89 c83f); PM through I15 can wrap
        let u = lower_one(&[0x3d89, 0xc83f], false).unwrap();
        assert_eq!(UopKind(u.h), DUAL);
        assert_eq!(u.flags & UF_CB, UF_CB);
        let (dm, pm) = dual_sides(u.a);
        assert_eq!((dm.i, dm.m, dm.reg, dm.write), (6, 6, 4, true));
        assert_eq!((pm.i, pm.m, pm.reg, pm.write), (15, 9, 0, false));
        // type 1a with a compute stays Interp (2cc0 921b c004)
        assert!(lower_one(&[0x2cc0, 0x921b, 0xc004], false).is_none());
    }
}
