//! OWNER: W2-NMEM. The ARM condition codes as `i32` values (0 or 1) over the flag variables of
//! `NCtx` (`flag(3)` N, `flag(2)` Z, `flag(1)` C, `flag(0)` V).
//!
//! Each arm is the definition of the condition (`Cpu::cond`, the table `COND_MASK` that
//! `uop::holds` reads): EQ Z, NE !Z, CS C, CC !C, MI N, PL !N, VS V, VC !V, HI C and !Z, LS !C or
//! Z, GE N == V, LT N != V, GT !Z and N == V, LE Z or N != V; 14 and 15 always hold. The native
//! tests run every condition over all 16 flag nibbles against `holds`.

use super::super::lower_mem::mutant;
use crate::machine::sb::native::emit::NCtx;
use cranelift_codegen::ir::{InstBuilder, Value};

/// Flag bits of the nibble (`n << 3 | z << 2 | c << 1 | v`), the index `NCtx::flag` takes.
const N: u8 = 3;
const Z: u8 = 2;
const C: u8 = 1;
const V: u8 = 0;

/// Whether `cond` is one of the 16 condition codes.
pub(super) fn is_cond(cond: u8) -> bool {
    cond < 16
}

fn not(cx: &mut NCtx<'_>, v: Value) -> Value {
    cx.b.ins().bxor_imm(v, 1)
}

/// `1` when `cond` holds on the current flags, else `0`; `None` for a value above 15, with
/// nothing emitted.
pub(super) fn cond_value(cx: &mut NCtx<'_>, cond: u8) -> Option<Value> {
    if !is_cond(cond) {
        return None;
    }
    let v = match cond {
        0 => cx.flag(Z),
        1 => {
            let z = cx.flag(Z);
            not(cx, z)
        }
        2 if mutant(891) => {
            let c = cx.flag(C);
            not(cx, c)
        }
        2 => cx.flag(C),
        3 => {
            let c = cx.flag(C);
            not(cx, c)
        }
        4 => cx.flag(N),
        5 => {
            let n = cx.flag(N);
            not(cx, n)
        }
        6 => cx.flag(V),
        7 => {
            let v = cx.flag(V);
            not(cx, v)
        }
        8 => {
            let (c, z) = (cx.flag(C), cx.flag(Z));
            if mutant(883) {
                cx.b.ins().bor(c, z)
            } else {
                let nz = not(cx, z);
                cx.b.ins().band(c, nz)
            }
        }
        9 => {
            let (c, z) = (cx.flag(C), cx.flag(Z));
            let nc = not(cx, c);
            if mutant(888) {
                cx.b.ins().band(nc, z)
            } else {
                cx.b.ins().bor(nc, z)
            }
        }
        10..=13 => {
            let (n, v, z) = (cx.flag(N), cx.flag(V), cx.flag(Z));
            let differ = cx.b.ins().bxor(n, v);
            match cond {
                10 if mutant(884) => differ,
                10 => not(cx, differ),
                11 => differ,
                12 => {
                    let same = not(cx, differ);
                    if mutant(889) {
                        return Some(same);
                    }
                    let nz = not(cx, z);
                    cx.b.ins().band(nz, same)
                }
                _ if mutant(890) => cx.b.ins().band(z, differ),
                _ => cx.b.ins().bor(z, differ),
            }
        }
        _ => cx.const32(1),
    };
    Some(v)
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn a_condition_code_is_a_four_bit_value() {
        assert!((0..16).all(is_cond));
        assert!((16..=255).all(|c| !is_cond(c)));
        assert_eq!((N, Z, C, V), (3, 2, 1, 0), "the NCtx flag indices");
    }
}
