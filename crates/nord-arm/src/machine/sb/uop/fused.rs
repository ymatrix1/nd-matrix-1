//! OWNER: S0 (frozen). Layout and definition of the fused kinds (`k::CMP_GUARD_I`,
//! `k::CMP_GUARD_R`, `k::TST_GUARD_I`, `k::MOV_PAIR`, `k::LDR_PAIR`).
//!
//! A fused uop covers two adjacent guest instructions (`n == 2`, `pc` of the first, the second
//! at `pc + 4`) and is *defined* as the sequence of its unfused expansion: executed in order,
//! with `bus.pc` set to each access' own pc (I5), the second uop reading the state the first one
//! left. [`fuse_pair`] is the only place that builds a fused uop (W2-PEEP decides when to call
//! it: adjacency, `PRED` boundaries, flag liveness) and [`expand`] is the only place that
//! decodes one; executors and native lowerers either call `expand` or are tested against it.
//!
//! | Kind | Operands of the fused uop | Expansion |
//! |---|---|---|
//! | `CMP_GUARD_I` | `rn`, `imm`, `cond`, `rs` = exit index | `CMP_IMM rn,#imm` ; `GUARD cond -> rs` |
//! | `CMP_GUARD_R` | `rn`, `rm`, `cond`, `rs` | `CMP_REG rn,rm` ; `GUARD cond -> rs` |
//! | `TST_GUARD_I` | `rn`, `imm`, `sh` (shifter-carry bit), `cond`, `rs` | `TST_IMM rn,#imm` ; `GUARD cond -> rs` |
//! | `MOV_PAIR` | `rd`, `rn` = second dest, `imm` = `a \| b << 16` (both below 2^16) | `MOV_IMM rd,#a` ; `MOV_IMM rn,#b` |
//! | `LDR_PAIR` | `rd`, `rm` = second dest, `rn` = shared base, `rs` flags (no write-back), `imm` = `a \| b << 16` (both below 2^12) | `LDR_IMM rd,[rn,#a]` ; `LDR_IMM rm,[rn,#b]` |
//!
//! The guard of a `CMP_GUARD_*` leaves through exit `rs`; its `done`/`last_pc` are the static
//! ones of that exit (I3), which already count the branch instruction.

use super::{k, Uop};

const GUARDED: [(u8, u8); 3] = [
    (k::CMP_IMM, k::CMP_GUARD_I),
    (k::CMP_REG, k::CMP_GUARD_R),
    (k::TST_IMM, k::TST_GUARD_I),
];

/// The fused uop equal to `a ; b`, or `None` when the pair has no fused form (not adjacent,
/// operands outside the packing, flags the packing cannot carry). Pure and total.
pub fn fuse_pair(a: &Uop, b: &Uop) -> Option<Uop> {
    if a.n != 1 || b.n != 1 || b.pc != a.pc.wrapping_add(4) {
        return None;
    }
    let mut f = Uop::new(0, a.pc);
    f.n = 2;
    if b.kind == k::GUARD {
        let &(_, kind) = GUARDED.iter().find(|(src, _)| *src == a.kind)?;
        let shape_ok = match kind {
            k::CMP_GUARD_I => a.rm == 0 && a.sh == 0,
            k::CMP_GUARD_R => a.imm == 0 && a.sh == 0,
            _ => a.rm == 0,
        };
        let plain = a.rd == 0 && a.rs == 0 && a.cond == 0;
        let bare_guard = b.rd == 0 && b.rn == 0 && b.rm == 0 && b.sh == 0 && b.imm == 0;
        if !plain || !shape_ok || !bare_guard {
            return None;
        }
        f.kind = kind;
        (f.rn, f.rm, f.imm, f.sh) = (a.rn, a.rm, a.imm, a.sh);
        (f.cond, f.rs) = (b.cond, b.rs);
        return Some(f);
    }
    let bare = |u: &Uop| u.cond == 0 && u.sh == 0;
    match (a.kind, b.kind) {
        (k::MOV_IMM, k::MOV_IMM)
            if bare(a)
                && bare(b)
                && a.imm <= 0xFFFF
                && b.imm <= 0xFFFF
                && no_src(a)
                && no_src(b) =>
        {
            f.kind = k::MOV_PAIR;
            (f.rd, f.rn, f.imm) = (a.rd, b.rd, a.imm | b.imm << 16);
            Some(f)
        }
        (k::LDR_IMM, k::LDR_IMM)
            if bare(a)
                && bare(b)
                && a.rn == b.rn
                && a.rs == b.rs
                && a.rs & 0b100 == 0
                && a.rm == 0
                && b.rm == 0
                && a.imm <= 0xFFF
                && b.imm <= 0xFFF =>
        {
            f.kind = k::LDR_PAIR;
            (f.rd, f.rm, f.rn, f.rs) = (a.rd, b.rd, a.rn, a.rs);
            f.imm = a.imm | b.imm << 16;
            Some(f)
        }
        _ => None,
    }
}

/// `MOV_IMM` reads no register: `rn`, `rm`, `rs` stay zero.
fn no_src(u: &Uop) -> bool {
    u.rn == 0 && u.rm == 0 && u.rs == 0
}

/// The two uops a fused uop stands for, in execution order; `None` for any other kind.
pub fn expand(u: &Uop) -> Option<[Uop; 2]> {
    let second_pc = u.pc.wrapping_add(4);
    match u.kind {
        k::CMP_GUARD_I | k::CMP_GUARD_R | k::TST_GUARD_I => {
            let src = GUARDED.iter().find(|(_, fused)| *fused == u.kind)?.0;
            let mut a = Uop::new(src, u.pc);
            (a.rn, a.rm, a.imm, a.sh) = (u.rn, u.rm, u.imm, u.sh);
            let mut g = Uop::new(k::GUARD, second_pc);
            (g.cond, g.rs) = (u.cond, u.rs);
            Some([a, g])
        }
        k::MOV_PAIR => {
            let mut a = Uop::new(k::MOV_IMM, u.pc);
            let mut b = Uop::new(k::MOV_IMM, second_pc);
            (a.rd, a.imm) = (u.rd, u.imm & 0xFFFF);
            (b.rd, b.imm) = (u.rn, u.imm >> 16);
            Some([a, b])
        }
        k::LDR_PAIR => {
            let mut a = Uop::new(k::LDR_IMM, u.pc);
            let mut b = Uop::new(k::LDR_IMM, second_pc);
            (a.rd, a.rn, a.rs, a.imm) = (u.rd, u.rn, u.rs, u.imm & 0xFFFF);
            (b.rd, b.rn, b.rs, b.imm) = (u.rm, u.rn, u.rs, u.imm >> 16);
            Some([a, b])
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(kind: u8, pc: u32) -> Uop {
        Uop::new(kind, pc)
    }

    fn guard(pc: u32, cond: u8, exit: u8) -> Uop {
        let mut g = at(k::GUARD, pc);
        (g.cond, g.rs) = (cond, exit);
        g
    }

    #[test]
    fn each_compare_and_guard_pair_fuses_and_expands_back_exactly() {
        let mut ci = at(k::CMP_IMM, 0x1000);
        (ci.rn, ci.imm) = (3, 0x2A);
        let mut cr = at(k::CMP_REG, 0x1000);
        (cr.rn, cr.rm) = (3, 4);
        let mut ti = at(k::TST_IMM, 0x1000);
        (ti.rn, ti.imm, ti.sh) = (5, 0xFF, 1);
        for (a, kind) in [
            (ci, k::CMP_GUARD_I),
            (cr, k::CMP_GUARD_R),
            (ti, k::TST_GUARD_I),
        ] {
            let g = guard(0x1004, 1, 7);
            let f = fuse_pair(&a, &g).unwrap();
            assert_eq!((f.kind, f.n, f.pc, f.cond, f.rs), (kind, 2, 0x1000, 1, 7));
            assert_eq!(expand(&f), Some([a, g]), "kind {kind}");
        }
    }

    #[test]
    fn mov_and_ldr_pairs_pack_two_operands_and_expand_back_exactly() {
        let mut a = at(k::MOV_IMM, 0x2000);
        (a.rd, a.imm) = (1, 0xFFFF);
        let mut b = at(k::MOV_IMM, 0x2004);
        (b.rd, b.imm) = (2, 0x1234);
        let f = fuse_pair(&a, &b).unwrap();
        assert_eq!(
            (f.kind, f.rd, f.rn, f.imm),
            (k::MOV_PAIR, 1, 2, 0x1234_FFFF)
        );
        assert_eq!(expand(&f), Some([a, b]));

        let mut l1 = at(k::LDR_IMM, 0x3000);
        (l1.rd, l1.rn, l1.rs, l1.imm) = (4, 13, 0b011, 0xFFC);
        let mut l2 = at(k::LDR_IMM, 0x3004);
        (l2.rd, l2.rn, l2.rs, l2.imm) = (5, 13, 0b011, 0x004);
        let f = fuse_pair(&l1, &l2).unwrap();
        assert_eq!((f.kind, f.rd, f.rm, f.rn, f.n), (k::LDR_PAIR, 4, 5, 13, 2));
        assert_eq!(expand(&f), Some([l1, l2]));
    }

    #[test]
    fn pairs_the_packing_cannot_carry_are_not_fused() {
        let g = guard(0x1004, 1, 0);
        let mut ci = at(k::CMP_IMM, 0x1000);
        assert!(fuse_pair(&ci, &g).is_some());
        assert!(
            fuse_pair(&ci, &guard(0x1008, 1, 0)).is_none(),
            "not adjacent"
        );
        ci.n = 0;
        assert!(fuse_pair(&ci, &g).is_none());
        let mut wide = at(k::MOV_IMM, 0x2000);
        wide.imm = 0x1_0000;
        assert!(fuse_pair(&wide, &at(k::MOV_IMM, 0x2004)).is_none());
        let mut wb = at(k::LDR_IMM, 0x3000);
        wb.rs = 0b111;
        let mut wb2 = wb;
        wb2.pc = 0x3004;
        assert!(fuse_pair(&wb, &wb2).is_none(), "write-back stays unfused");
        let (mut p, mut q) = (at(k::LDR_IMM, 0x3000), at(k::LDR_IMM, 0x3004));
        (p.rn, q.rn) = (1, 2);
        assert!(fuse_pair(&p, &q).is_none(), "different bases");
        assert!(fuse_pair(&at(k::ADD_IMM, 0x1000), &g).is_none());
    }

    #[test]
    fn only_the_fused_kinds_expand() {
        for kind in 0..=255u8 {
            let fused = (k::CMP_GUARD_I..=k::LDR_PAIR).contains(&kind);
            assert_eq!(expand(&at(kind, 0x1000)).is_some(), fused, "kind {kind}");
        }
    }
}
