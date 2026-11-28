//! OWNER: W2-PEEP. What one uop does to the flag nibble `f = n << 3 | z << 2 | c << 1 | v`
//! (I4), as three bit sets, for the dead-flag pass and the predicate grouping.
//!
//! | Set | Meaning |
//! |---|---|
//! | `reads` | flags whose old value can change what the uop (or the state at a point where the trace can end) observes |
//! | `writes` | flags the uop may change (an S operation whose shifter carry is the old C still lists C) |
//! | `kills` | flags the uop overwrites with a value that does not depend on their old value |
//!
//! `kills` is a subset of `writes`; a flag that is only kept (the V of a logic operation) is in
//! neither. Every uop that can end the trace (guards, exits, `LOOP_BACK`, stores through their
//! early exit I6, fused guards), every `GENERIC*`, `MRS` and `MSR_C`, and every id outside the
//! tables read all four flags: state must be exact there (I4), so a definition before one of
//! them is live whatever follows.
//!
//! The model is deliberately a superset of what the executor does (`exec_alu.rs`): a carry that
//! may come from the old C is a read, and C is killed only by a form-0 immediate with a real
//! rotation, so no shifter encoding can make the pass drop a flag somebody observes.
//! VERIFIED against the executor arms themselves by
//! `the_model_covers_what_the_executor_does_to_the_flags` (every generic and specialised id,
//! six shifter encodings, 16 nibbles, 9 x 9 edge operand pairs: zero differences).

use super::mutant;
use crate::machine::sb::pred::cond_reads;
use crate::machine::sb::uop::{k, Uop};

pub const N: u8 = 8;
pub const Z: u8 = 4;
pub const C: u8 = 2;
pub const V: u8 = 1;
pub const ALL: u8 = 15;

/// Flag effect of one uop (see the module docs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effect {
    pub reads: u8,
    pub writes: u8,
    pub kills: u8,
}

const NONE: Effect = Effect {
    reads: 0,
    writes: 0,
    kills: 0,
};
/// A point where the flags must be exact.
const BARRIER: Effect = Effect {
    reads: ALL,
    writes: 0,
    kills: 0,
};

/// Logic opcodes of the generic ids: `and eor tst teq orr mov bic mvn`.
const fn is_logic(op: u32) -> bool {
    matches!(op, 0 | 1 | 8 | 9 | 12 | 13 | 14 | 15)
}

/// The effect of a generic data-processing id `0..=95` (`k::dp(form, op, s)`).
fn generic(u: &Uop) -> Effect {
    let form = (u.kind >> 5) as u32;
    let (op, s) = ((u.kind >> 1 & 15) as u32, u.kind & 1 != 0);
    let mut e = NONE;
    if matches!(op, 5..=7) && !mutant(504) {
        e.reads |= C; // adc sbc rsc
    }
    if form == 1 && u.sh >> 5 & 3 == 3 && !mutant(510) {
        e.reads |= C; // ror #0 is rrx: the old C is shifted in
    }
    if !s {
        return e;
    }
    // a form-0 immediate with a real rotation: the shifter carry is bit 31 of the immediate
    let mut own_carry = form == 0 && u.sh & 1 == 0;
    if mutant(507) {
        own_carry |= form != 0;
    }
    if mutant(508) {
        own_carry = form == 0;
    }
    if is_logic(op) {
        e.writes = N | Z | C;
        e.kills = N | Z | if own_carry { C } else { 0 };
        if !own_carry {
            e.reads |= C; // the carry out of the shifter may be the old C
        }
    } else {
        e.writes = ALL;
        e.kills = ALL;
    }
    e
}

/// The effect of `u` (see the module docs).
pub fn effect(u: &Uop) -> Effect {
    match u.kind {
        0..=95 => generic(u),
        k::CMP_IMM | k::CMP_REG => Effect {
            reads: 0,
            writes: ALL,
            kills: ALL,
        },
        k::TST_IMM => {
            let own_carry = u.sh & 1 == 0;
            Effect {
                reads: if own_carry { 0 } else { C },
                writes: N | Z | C,
                kills: N | Z | if own_carry { C } else { 0 },
            }
        }
        // `tst rn, rm` (lsl #0): the carry is the old C and stays, V stays
        k::TST_REG => Effect {
            reads: 0,
            writes: N | Z,
            kills: N | Z,
        },
        96..=107 | k::MOV_PAIR | k::LDR_PAIR => NONE,
        k::PRED => Effect {
            reads: cond_reads(u.cond),
            writes: 0,
            kills: 0,
        },
        // loads: no flag, no early exit
        kind if k::family(kind) == k::Family::Mem && !k::is_store(kind) => NONE,
        kind if k::is_store(kind) && mutant(500) => NONE,
        k::GUARD | k::GUARD_REG_EQ if mutant(501) => NONE,
        k::GENERIC | k::GENERIC_END if mutant(502) => NONE,
        k::LOOP_BACK | k::EXIT | k::EXIT_REG | k::EXIT_REG_ARM if mutant(506) => NONE,
        k::MRS if mutant(511) => NONE,
        _ => BARRIER,
    }
}

/// True when the uop may change the flags (an S operation, or the interpreter's handler or a
/// status write, where the pass cannot say which flags): the predicate grouping must not group
/// past it.
pub fn writes_flags(u: &Uop) -> bool {
    effect(u).writes != 0
        || (matches!(u.kind, k::GENERIC | k::GENERIC_END | k::MSR_C) && !mutant(525))
}

#[cfg(test)]
pub(in crate::machine::sb) mod tests {
    use super::*;
    use crate::machine::sb::exec_alu::exec_dp;
    use crate::machine::sb::trace::testexec::suite;

    fn dp(form: u32, op: u32, s: bool, sh: u8) -> Uop {
        let mut u = Uop::new(k::dp(form, op, s), 0x1000);
        u.sh = sh;
        u
    }

    suite! {
        #[test]
        fn arithmetic_s_ops_overwrite_every_flag_and_non_s_ops_touch_none() {
            for op in [2, 3, 4, 5, 6, 7, 10, 11] {
                let e = effect(&dp(1, op, true, 0));
                assert_eq!((e.writes, e.kills), (ALL, ALL), "op {op}");
                assert_eq!(e.reads, if matches!(op, 5..=7) { C } else { 0 }, "op {op}");
            }
            for op in [0, 1, 2, 4, 5, 12, 13, 14, 15] {
                let e = effect(&dp(1, op, false, 0));
                assert_eq!((e.writes, e.kills), (0, 0), "op {op}");
            }
            assert_eq!(effect(&dp(1, 5, false, 0)).reads, C, "adc reads C without S");
        }

        #[test]
        fn logic_s_ops_keep_v_and_only_a_rotated_immediate_overwrites_c() {
            for op in [0, 1, 8, 9, 12, 13, 14, 15] {
                let reg = effect(&dp(1, op, true, 5));
                let want = (N | Z | C, N | Z, C);
                assert_eq!((reg.writes, reg.kills, reg.reads), want, "op {op}");
                let rot = effect(&dp(0, op, true, 0));
                let want = (N | Z | C, N | Z | C, 0);
                assert_eq!((rot.writes, rot.kills, rot.reads), want, "op {op}");
                let unrot = effect(&dp(0, op, true, 1));
                assert_eq!((unrot.kills, unrot.reads), (N | Z, C), "op {op}");
            }
        }

        #[test]
        fn rrx_reads_the_old_carry_even_without_s() {
            assert_eq!(effect(&dp(1, 13, false, 3 << 5)).reads, C);
            assert_eq!(effect(&dp(1, 13, false, 2 << 5)).reads, 0);
            assert_eq!(effect(&dp(2, 13, false, 3 << 5)).reads, 0, "no rrx by register");
        }

        #[test]
        fn specialised_ids_follow_the_executor() {
            let at = |kind| effect(&Uop::new(kind, 0x1000));
            for kind in [
                k::MOV_IMM,
                k::MOV_REG,
                k::MVN_IMM,
                k::ADD_IMM,
                k::SUB_IMM,
                k::ADD_REG,
                k::SUB_REG,
                k::ORR_IMM,
            ] {
                assert_eq!(at(kind), NONE, "kind {kind}");
            }
            assert_eq!((at(k::CMP_IMM).writes, at(k::CMP_REG).kills), (ALL, ALL));
            let tst = Effect {
                reads: 0,
                writes: N | Z,
                kills: N | Z,
            };
            assert_eq!(at(k::TST_REG), tst);
            assert_eq!(at(k::TST_IMM).kills, N | Z | C);
            let mut rot0 = Uop::new(k::TST_IMM, 0x1000);
            rot0.sh = 1;
            assert_eq!((effect(&rot0).kills, effect(&rot0).reads), (N | Z, C));
        }

        #[test]
        fn every_point_where_the_trace_can_end_or_the_interpreter_runs_reads_all_flags() {
            for kind in [
                k::GUARD,
                k::GUARD_REG_EQ,
                k::LOOP_BACK,
                k::EXIT,
                k::EXIT_REG,
                k::EXIT_REG_ARM,
                k::GENERIC,
                k::GENERIC_END,
                k::MRS,
                k::MSR_C,
                k::CMP_GUARD_I,
                k::CMP_GUARD_R,
                k::TST_GUARD_I,
                k::STR_IMM,
                k::STRB_IMM,
                k::STR_REG,
                k::STRB_REG,
                k::STRH,
                k::STRD,
                k::STM,
            ] {
                assert_eq!(effect(&Uop::new(kind, 0x1000)).reads, ALL, "kind {kind}");
            }
            for kind in [
                k::LDR_IMM,
                k::LDRB_IMM,
                k::LDR_REG,
                k::LDRH,
                k::LDRSH,
                k::LDRSB,
                k::LDRD,
                k::LDM,
                k::LDR_ABS,
            ] {
                assert_eq!(effect(&Uop::new(kind, 0x1000)), NONE, "load {kind}");
            }
            assert_eq!(effect(&Uop::new(255, 0x1000)).reads, ALL, "unknown: a barrier");
        }

        #[test]
        fn a_predicate_reads_exactly_the_flags_its_condition_tests() {
            let mut p = Uop::new(k::PRED, 0x1000);
            let (nzv, cz) = (N | Z | V, C | Z);
            let want = [Z, Z, C, C, N, N, V, V, cz, cz, N | V, N | V, nzv, nzv];
            for (cond, want) in want.into_iter().enumerate() {
                p.cond = cond as u8;
                assert_eq!(effect(&p).reads, want, "cond {cond}");
            }
            p.cond = 14;
            assert_eq!(effect(&p).reads, 0);
        }

        #[test]
        fn writes_flags_names_the_uops_a_predicate_run_must_not_cross() {
            for kind in [k::GENERIC, k::GENERIC_END, k::MSR_C, k::CMP_IMM, k::TST_REG, k::dp(1, 4, true)] {
                assert!(writes_flags(&Uop::new(kind, 0x1000)), "kind {kind}");
            }
            for kind in [k::MOV_IMM, k::dp(1, 4, false), k::LDR_IMM, k::STR_IMM, k::MRS] {
                assert!(!writes_flags(&Uop::new(kind, 0x1000)), "kind {kind}");
            }
        }
    }

    #[test]
    fn the_model_covers_what_the_executor_does_to_the_flags() {
        let edge = [
            0u32,
            1,
            2,
            0x7FFF_FFFF,
            0x8000_0000,
            0xFFFF_FFFF,
            31,
            32,
            33,
        ];
        let mut checked = 0u64;
        for kind in 0..=107u8 {
            for sh in [0u8, 1, 5, 2 << 5, 3 << 5, 3 << 5 | 7, 1 << 5 | 31] {
                let mut u = Uop::new(kind, 0x1000);
                (u.rd, u.rn, u.rm, u.rs, u.sh, u.imm) = (1, 2, 3, 4, sh, 0x8000_0001);
                let e = effect(&u);
                for f in 0..16u8 {
                    for &a in &edge {
                        for &b in &edge {
                            let mut r = [0u32; 16];
                            (r[2], r[3], r[4]) = (a, b, b & 0xFF);
                            let (mut r1, mut f1) = (r, f);
                            exec_dp(&mut r1, &mut f1, &u);
                            assert_eq!((f1 ^ f) & !e.writes, 0, "kind {kind} sh {sh}: write");
                            for bit in [N, Z, C, V] {
                                if e.reads & bit != 0 {
                                    continue;
                                }
                                // a flag that is not read changes neither the result nor, unless
                                // it passes through, anything else
                                let (mut r2, mut f2) = (r, f ^ bit);
                                exec_dp(&mut r2, &mut f2, &u);
                                assert_eq!(r1, r2, "kind {kind} sh {sh}: result reads {bit}");
                                let passes = if e.kills & bit != 0 { 0 } else { bit };
                                assert_eq!(f1 ^ f2, passes, "kind {kind} sh {sh}: flag {bit}");
                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(checked > 100_000, "{checked}");
    }
}
