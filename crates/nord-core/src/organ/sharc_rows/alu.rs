//! Compute expressions of single instruction items (`f12 = f1 * f6`, `r3 = fext r1 by 6:12`, ...).
//!
//! Port of `alu_row`, `mode_bits` and the multiplier rows of `tools/sharc_listing_rust.py`. The
//! expression is the right hand side of an item, lower case; the destination register id is
//! resolved by the caller ([`super::item`]).

use crate::organ::b3_bass_fw::tables::{alu_op, kind};
use crate::organ::b3_bass_fw::Row;

fn float_reg(s: &str) -> Option<i32> {
    s.strip_prefix('f')?.parse().ok()
}

fn int_reg(s: &str) -> Option<i32> {
    s.strip_prefix('r')?.parse().ok()
}

/// A float or integer data register (`f3`, `r3`), for `float rx by ry`.
fn data_reg(s: &str) -> Option<i32> {
    float_reg(s).or_else(|| int_reg(s))
}

/// Value of a decimal or `0x` literal.
fn literal(s: &str) -> Option<i32> {
    match s.strip_prefix("0x") {
        Some(h) => i32::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

/// `(x, y)` of `<head> rx by ry` with `reg` selecting the register kind.
fn by_regs(e: &str, head: &str, reg: fn(&str) -> Option<i32>) -> Option<(i32, i32)> {
    let (x, y) = e.strip_prefix(head)?.split_once(" by ")?;
    Some((reg(x.trim())?, reg(y.trim())?))
}

/// Multiplier mode bits of `ssfr`, `ssf`, ...: bit 0 signed x, bit 1 signed y, bit 2 fractional,
/// bit 3 rounded (`mode_bits` of the Python tool).
pub(super) fn mode_bits(mode: &str) -> i32 {
    let b = mode.as_bytes();
    i32::from(b.first() == Some(&b's'))
        | i32::from(b.get(1) == Some(&b's')) << 1
        | i32::from(b.get(2) == Some(&b'f')) << 2
        | i32::from(mode.ends_with('r')) << 3
}

/// `(accumulate, x, y, mode)` of `[mrf|mrb (+|-)] rx * ry (mode)`; accumulate 0 none, 1 add, 2
/// subtract.
pub(super) fn mult_parts(e: &str) -> Option<(i32, i32, i32, i32)> {
    let (body, mode) = e.trim().strip_suffix(')')?.rsplit_once('(')?;
    let body: String = body.split_whitespace().collect();
    let (acc, rest) = match body
        .strip_prefix("mrf")
        .or_else(|| body.strip_prefix("mrb"))
    {
        Some(r) => match r.as_bytes().first() {
            Some(b'+') => (1, &r[1..]),
            Some(b'-') => (2, &r[1..]),
            _ => return None,
        },
        None => (0, body.as_str()),
    };
    let (x, y) = rest.split_once('*')?;
    Some((acc, int_reg(x)?, int_reg(y)?, mode_bits(mode.trim())))
}

/// `fext` / `fdep` of a register or a `pos:len` field: `[ALU, dst, op, x, y | pos, se | len, ..]`.
fn field_row(dst: i32, e: &str) -> Option<Row> {
    let (extract, rest) = match e.split_once(' ')? {
        ("fext", r) => (true, r),
        ("fdep", r) => (false, r),
        _ => return None,
    };
    let (rest, se) = match rest.strip_suffix("(se)") {
        Some(r) => (r.trim(), 1),
        None => (rest, 0),
    };
    let (x, field) = rest.split_once(" by ")?;
    let x = int_reg(x.trim())?;
    let field = field.trim();
    if let Some(y) = int_reg(field) {
        let op = if extract {
            alu_op::FEXT_R
        } else {
            alu_op::FDEP_R
        };
        return Some([kind::ALU, dst, op, x, y, se, 0, 0]);
    }
    let (pos, len) = field.split_once(':')?;
    let op = if extract {
        alu_op::FEXT_I
    } else {
        alu_op::FDEP_I
    };
    Some([kind::ALU, dst, op, x, literal(pos)?, literal(len)?, se, 0])
}

/// `[ALU, dst, op, x, y, p, q, 0]` of a compute expression, or `None`.
pub(super) fn alu_row(dst: i32, src: &str) -> Option<Row> {
    let e = src.trim();
    let alu = |op, x, y| Some([kind::ALU, dst, op, x, y, 0, 0, 0]);
    if let Some(x) = e.strip_prefix("pass ").and_then(|r| float_reg(r.trim())) {
        return alu(alu_op::PASS, x, 0);
    }
    if let Some(x) = e.strip_prefix("abs ").and_then(|r| float_reg(r.trim())) {
        return alu(alu_op::ABS, x, 0);
    }
    if let Some((x, y)) = by_regs(e, "float ", data_reg) {
        return alu(alu_op::FLOATBY, x, y);
    }
    if let Some(row) = field_row(dst, e) {
        return Some(row);
    }
    for (head, op) in [
        ("ashift ", alu_op::ASHIFT),
        ("lshift ", alu_op::LSHIFT),
        ("clip ", alu_op::CLIP),
    ] {
        if let Some((x, y)) = by_regs(e, head, int_reg) {
            return alu(op, x, y);
        }
    }
    if let Some((x, y)) = e
        .split_once(" and ")
        .and_then(|(x, y)| Some((int_reg(x.trim())?, int_reg(y.trim())?)))
    {
        return alu(alu_op::AND, x, y);
    }
    if let Some((acc, x, y, mode)) = mult_parts(e) {
        return Some([kind::ALU, dst, alu_op::MULT, x, y, acc, mode, 0]);
    }
    let c: String = e.split_whitespace().collect();
    let at = c.find(['*', '+', '-'])?;
    let (l, r) = (&c[..at], &c[at + 1..]);
    let op = c.as_bytes()[at];
    if let (Some(x), Some(y)) = (float_reg(l), float_reg(r)) {
        let o = match op {
            b'*' => alu_op::FMUL,
            b'+' => alu_op::FADD,
            _ => alu_op::FSUB,
        };
        return alu(o, x, y);
    }
    let x = int_reg(l)?;
    if op == b'*' {
        return None;
    }
    if let Some(y) = int_reg(r) {
        let o = if op == b'+' {
            alu_op::IADD
        } else {
            alu_op::ISUB
        };
        return alu(o, x, y);
    }
    let y: i32 = r.parse().ok()?;
    alu(alu_op::ADDI, x, if op == b'+' { y } else { -y })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(src: &str) -> Row {
        alu_row(3, src).unwrap_or_else(|| panic!("no row for {src:?}"))
    }

    #[test]
    fn float_and_integer_arithmetic_rows() {
        assert_eq!(row("f1 * f6"), [kind::ALU, 3, alu_op::FMUL, 1, 6, 0, 0, 0]);
        assert_eq!(row("f1 - f6"), [kind::ALU, 3, alu_op::FSUB, 1, 6, 0, 0, 0]);
        assert_eq!(row("pass f8"), [kind::ALU, 3, alu_op::PASS, 8, 0, 0, 0, 0]);
        assert_eq!(row("abs f2"), [kind::ALU, 3, alu_op::ABS, 2, 0, 0, 0, 0]);
        assert_eq!(row("r1 - r2"), [kind::ALU, 3, alu_op::ISUB, 1, 2, 0, 0, 0]);
        assert_eq!(row("r4 + 7"), [kind::ALU, 3, alu_op::ADDI, 4, 7, 0, 0, 0]);
        assert_eq!(row("r4 - 7"), [kind::ALU, 3, alu_op::ADDI, 4, -7, 0, 0, 0]);
        assert_eq!(row("r1 and r2"), [kind::ALU, 3, alu_op::AND, 1, 2, 0, 0, 0]);
        assert_eq!(
            row("clip r1 by r2"),
            [kind::ALU, 3, alu_op::CLIP, 1, 2, 0, 0, 0]
        );
        assert!(
            alu_row(3, "r1 * r2").is_none(),
            "an integer product needs a mode"
        );
    }

    #[test]
    fn conversion_shift_and_field_rows() {
        let alu = |op, x, y, p, q| [kind::ALU, 3, op, x, y, p, q, 0];
        assert_eq!(row("float r1 by r2"), alu(alu_op::FLOATBY, 1, 2, 0, 0));
        assert_eq!(row("ashift r1 by r2"), alu(alu_op::ASHIFT, 1, 2, 0, 0));
        assert_eq!(row("lshift r1 by r2"), alu(alu_op::LSHIFT, 1, 2, 0, 0));
        assert_eq!(row("fext r1 by r2"), alu(alu_op::FEXT_R, 1, 2, 0, 0));
        assert_eq!(row("fdep r1 by r2 (se)"), alu(alu_op::FDEP_R, 1, 2, 1, 0));
        assert_eq!(row("fext r1 by 6:12"), alu(alu_op::FEXT_I, 1, 6, 12, 0));
        assert_eq!(
            row("fdep r1 by 0x4:8 (se)"),
            alu(alu_op::FDEP_I, 1, 4, 8, 1)
        );
        assert!(alu_row(3, "r3 or fdep r1 by r2").is_none());
    }

    #[test]
    fn multiplier_rows_and_modes() {
        assert_eq!(mode_bits("ssf"), 0b111);
        assert_eq!(mode_bits("ssfr"), 0b1111);
        assert_eq!(mode_bits("uui"), 0);
        let mult = |acc, mode| [kind::ALU, 3, alu_op::MULT, 1, 2, acc, mode, 0];
        assert_eq!(row("mrf + r1 * r2 (ssfr)"), mult(1, 0b1111));
        assert_eq!(row("mrb - r1*r2 (ssf)"), mult(2, 0b111));
        assert_eq!(row("r1 * r2 (ssf)"), mult(0, 0b111));
        assert_eq!(mult_parts("mrf * r1 * r2 (ssf)"), None);
        assert_eq!(mult_parts("r1 + r2"), None);
    }
}
