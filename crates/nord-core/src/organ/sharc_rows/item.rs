//! Rows of single instruction items (`r1 = dm(i7, m2)`, `f12 = f1 * f6`, ...), lower case text.
//!
//! Port of `item_row`, `alu_row` and `addr_fields` of `tools/sharc_listing_rust.py`.

use super::alu::{alu_row, mult_parts};
use crate::organ::b3_bass_fw::tables::kind;
use crate::organ::b3_bass_fw::Row;

/// Register number of `name`: data registers `r` 0..15, `s` 16..31, then `i`, `m`, `l`, `b`,
/// `lcntr` (the numbering of `reg_id` in the Python tool).
fn reg_id(name: &str) -> Result<i32, String> {
    if name == "lcntr" {
        return Ok(96);
    }
    let n = number(name.get(1..).unwrap_or(""))?;
    Ok(match name.as_bytes().first() {
        Some(b'r' | b'f') => n,
        Some(b's') => 16 + n,
        Some(b'i') => 32 + n,
        Some(b'm') => 48 + n,
        Some(b'l') => 64 + n,
        Some(b'b') => 80 + n,
        _ => return Err(format!("unknown register {name:?}")),
    })
}

pub(super) fn number(s: &str) -> Result<i32, String> {
    s.parse().map_err(|_| format!("bad number {s:?}"))
}

/// Value of a decimal or `0x` literal as a signed 32-bit word.
pub(super) fn sx(s: &str) -> Result<i32, String> {
    let s = s.trim();
    let (neg, body) = s.strip_prefix('-').map_or((false, s), |b| (true, b));
    let v = match body.strip_prefix("0x") {
        Some(h) => i64::from_str_radix(h, 16),
        None => body.parse(),
    }
    .map_err(|_| format!("bad literal {s:?}"))?;
    Ok((if neg { -v } else { v }) as u32 as i32)
}

pub(super) fn is_data(name: &str) -> bool {
    matches!(name.as_bytes().first(), Some(b'r' | b's' | b'f'))
        && name.len() > 1
        && name[1..].bytes().all(|b| b.is_ascii_digit())
}

/// `[kind, p, q]` of a `dm(...)` operand: 0 absolute, 1 post modify by `m`, 2 post modify by an
/// immediate, 3 pre modify by `m`, 4 pre modify by an immediate.
fn addr_fields(spec: &str) -> Result<[i32; 3], String> {
    let parts: Vec<&str> = spec.split(',').map(str::trim).collect();
    match parts.as_slice() {
        [a] => Ok([0, sx(a)?, 0]),
        [a, b] if a.starts_with('i') => {
            let ia = number(&a[1..])?;
            if let Some(m) = b.strip_prefix('m') {
                Ok([1, ia, number(m)?])
            } else {
                Ok([2, ia, sx(b)?])
            }
        }
        [a, b] if b.starts_with('i') => {
            let ib = number(&b[1..])?;
            if let Some(m) = a.strip_prefix('m') {
                Ok([3, ib, number(m)?])
            } else {
                Ok([4, ib, sx(a)?])
            }
        }
        _ => Err(format!("unsupported address {spec:?}")),
    }
}

/// Comma separated items of one instruction, commas inside parentheses kept.
pub(super) fn split_items(text: &str) -> Vec<String> {
    let (mut out, mut depth, mut cur) = (Vec::new(), 0i32, String::new());
    for ch in text.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if ch == ',' && depth == 0 {
            out.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    out.push(cur.trim().to_string());
    out.retain(|i| !i.is_empty());
    out
}

/// `(lhs, rhs)` of the first `=` outside parentheses.
fn split_assign(item: &str) -> Option<(&str, &str)> {
    let mut depth = 0;
    for (i, ch) in item.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            '=' if depth == 0 => return Some((item[..i].trim(), item[i + 1..].trim())),
            _ => {}
        }
    }
    None
}

/// The text inside `dm(...)` / `pm(...)` of `s`, and what follows the closing parenthesis.
fn mem_operand(s: &str) -> Option<(&str, &str)> {
    let rest = s.strip_prefix("dm(").or_else(|| s.strip_prefix("pm("))?;
    let close = rest.find(')')?;
    Some((&rest[..close], rest[close + 1..].trim()))
}

/// The row of one instruction item (lower case text).
pub(super) fn item_row(item: &str) -> Result<Row, String> {
    if let Some((a, b)) = item.split_once("<->") {
        let (a, b) = (a.trim(), b.trim());
        if !(is_data(a) && is_data(b)) {
            return Err(format!("unsupported swap {item:?}"));
        }
        return Ok([kind::SWAP, reg_id(a)?, reg_id(b)?, 0, 0, 0, 0, 0]);
    }
    if let Some(args) = item.strip_prefix("comp(").and_then(|a| a.strip_suffix(')')) {
        let reg = |r: &str| r.trim().strip_prefix('r')?.parse::<i32>().ok();
        if let Some((Some(x), Some(y))) = args.split_once(',').map(|(x, y)| (reg(x), reg(y))) {
            return Ok([kind::COMP, x, y, 0, 0, 0, 0, 0]);
        }
    }
    let (lhs, rhs) = split_assign(item).ok_or_else(|| format!("cannot parse {item:?}"))?;
    if matches!(lhs, "mrf" | "mrb") {
        return match mult_parts(rhs) {
            Some((0, x, y, mode)) => Ok([kind::MRF_SET, x, y, mode, 0, 0, 0, 0]),
            _ => Err(format!("unsupported multiplier register write {item:?}")),
        };
    }
    if let Some((spec, tail)) = mem_operand(lhs) {
        if !tail.is_empty() {
            return Err(format!("cannot parse {item:?}"));
        }
        let (src, lw) = match rhs.strip_suffix("(lw)") {
            Some(r) => (r.trim(), 1),
            None => (rhs, 0),
        };
        let [k, p, q] = addr_fields(spec)?;
        return Ok([kind::STORE, reg_id(src)?, k, p, q, lw, 0, 0]);
    }
    if let Some((spec, tail)) = mem_operand(rhs) {
        let lw = match tail {
            "" => 0,
            "(lw)" => 1,
            _ => return Err(format!("cannot parse {item:?}")),
        };
        let bcast = spec
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|t| t == "i1");
        let [k, p, q] = addr_fields(spec)?;
        return Ok([
            kind::LOAD,
            reg_id(lhs)?,
            k,
            p,
            q,
            lw | i32::from(bcast) << 1,
            0,
            0,
        ]);
    }
    if let Some(args) = rhs.strip_prefix("modify").map(str::trim) {
        let inner = args.strip_prefix('(').and_then(|a| a.strip_suffix(')'));
        let parts: Vec<&str> = inner
            .ok_or("bad modify")?
            .split(',')
            .map(str::trim)
            .collect();
        let [ia, step] = parts.as_slice() else {
            return Err(format!("cannot parse {item:?}"));
        };
        let by_m = step.starts_with('m');
        let val = if by_m { number(&step[1..])? } else { sx(step)? };
        return Ok([
            kind::MODIFY,
            reg_id(lhs)?,
            number(&ia[1..])?,
            i32::from(by_m),
            val,
            0,
            0,
            0,
        ]);
    }
    let is_name = |s: &str| {
        s.starts_with(|c: char| c.is_ascii_lowercase())
            && s.trim_start_matches(|c: char| c.is_ascii_lowercase())
                .bytes()
                .all(|b| b.is_ascii_digit())
    };
    if is_data(lhs) {
        let swap = |n: &str| format!("{}{}", if n.starts_with('s') { 's' } else { 'r' }, &n[1..]);
        let did = reg_id(&swap(lhs))?;
        if let Some(row) = alu_row(did, rhs) {
            return Ok(row);
        }
        if is_data(rhs) {
            return Ok([kind::MOVE_D, did, reg_id(&swap(rhs))?, 0, 0, 0, 0, 0]);
        }
        if is_name(rhs) {
            return Ok([kind::MOVE_DN, did, reg_id(rhs)?, 0, 0, 0, 0, 0]);
        }
        return Ok([kind::MOVE_DI, did, sx(rhs)?, 0, 0, 0, 0, 0]);
    }
    if is_name(rhs) {
        return Ok([kind::MOVE_N, reg_id(lhs)?, reg_id(rhs)?, 0, 0, 0, 0, 0]);
    }
    Ok([kind::MOVE_NI, reg_id(lhs)?, sx(rhs)?, 0, 0, 0, 0, 0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organ::b3_bass_fw::tables::alu_op;

    #[test]
    fn items_split_at_top_level_commas_only() {
        assert_eq!(
            split_items("r1 = dm(i7, m2), r6 = pm(i10, m10), f12 = f1 * f6"),
            ["r1 = dm(i7, m2)", "r6 = pm(i10, m10)", "f12 = f1 * f6"]
        );
    }

    #[test]
    fn address_forms_map_to_the_five_kinds() {
        assert_eq!(addr_fields("0xc27e0").unwrap(), [0, 0xc27e0, 0]);
        assert_eq!(addr_fields("i7, m2").unwrap(), [1, 7, 2]);
        assert_eq!(addr_fields("i4, 3").unwrap(), [2, 4, 3]);
        assert_eq!(addr_fields("m1, i5").unwrap(), [3, 5, 1]);
        assert_eq!(addr_fields("-32, i2").unwrap(), [4, 2, -32]);
        assert_eq!(addr_fields("0xffffffe0, i1").unwrap(), [4, 1, -32]);
        assert!(addr_fields("a, b, c").is_err());
    }

    #[test]
    fn item_rows_follow_the_python_rules() {
        let load = item_row("r1 = dm(-30, i1)").unwrap();
        assert_eq!(
            load,
            [kind::LOAD, 1, 4, 1, -30, 2, 0, 0],
            "i1 is a broadcast"
        );
        let load = item_row("r2 = dm(i7, m2)").unwrap();
        assert_eq!(load, [kind::LOAD, 2, 1, 7, 2, 0, 0, 0]);
        assert_eq!(
            item_row("dm(-24, i3) = r6").unwrap(),
            [kind::STORE, 6, 4, 3, -24, 0, 0, 0]
        );
        assert_eq!(
            item_row("dm(0xc2c16) = r14 (lw)").unwrap(),
            [kind::STORE, 14, 0, 0xc2c16, 0, 1, 0, 0]
        );
        assert_eq!(
            item_row("r5 <-> s4").unwrap(),
            [kind::SWAP, 5, 20, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            item_row("f12 = f1 * f6").unwrap(),
            [kind::ALU, 12, alu_op::FMUL, 1, 6, 0, 0, 0]
        );
        assert_eq!(
            item_row("s3 = pass f8").unwrap(),
            [kind::ALU, 19, alu_op::PASS, 8, 0, 0, 0, 0]
        );
        assert_eq!(
            item_row("r14 = r14 + r15").unwrap(),
            [kind::ALU, 14, alu_op::IADD, 14, 15, 0, 0, 0]
        );
        assert_eq!(
            item_row("r7 = s6").unwrap(),
            [kind::MOVE_D, 7, 22, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            item_row("r9 = m0").unwrap(),
            [kind::MOVE_DN, 9, 48, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            item_row("r8 = 0xc2b52").unwrap(),
            [kind::MOVE_DI, 8, 0xc2b52, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            item_row("i1 = i3").unwrap(),
            [kind::MOVE_N, 33, 35, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            item_row("i7 = 0xc2b56").unwrap(),
            [kind::MOVE_NI, 39, 0xc2b56, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            item_row("i4 = modify(i4, 5)").unwrap(),
            [kind::MODIFY, 36, 4, 0, 5, 0, 0, 0]
        );
        assert_eq!(
            item_row("r1 = mrf + r2 * r3 (ssfr)").unwrap(),
            [kind::ALU, 1, alu_op::MULT, 2, 3, 1, 0b1111, 0]
        );
        assert_eq!(
            item_row("mrf = r2 * r3 (ssf)").unwrap(),
            [kind::MRF_SET, 2, 3, 0b111, 0, 0, 0, 0]
        );
        assert_eq!(
            item_row("comp(r3, r4)").unwrap(),
            [kind::COMP, 3, 4, 0, 0, 0, 0, 0]
        );
        assert!(item_row("mrf = mrf + r2 * r3 (ssf)").is_err());
        assert!(item_row("comp(r3, f4)").is_err());
        assert!(item_row("bit set flags 0x2").is_err());
    }
}
