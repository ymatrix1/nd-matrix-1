//! Instruction rows of a SHARC routine, decoded from the user's DSP image at setup time.
//!
//! Port of `decode` of `tools/sharc_listing_rust.py` (rules: `docs/research/organ-farfisa-bass.md`
//! s15). The listing text comes from the VISA decoder of `nord-sharc` instead of ADI's `elfdump`
//! (D128: no firmware-derived rows are tracked). Rows are `[kind, a, b, c, d, e, f, g]`:
//!
//! * `INS n cond jcond target` opens an instruction of `n` item rows (reads see the old state,
//!   writes land at the end, `cond` = `if lt`); a jump is an `INS` with no items, `jcond` 1 =
//!   always, 2 = `if ne`, 3 = `if eq` and `target` the absolute row index it continues at;
//! * `LOOP count n` repeats the next `n` rows (`lcntr` / `do ... until lce`).
//!
//! Item parsing is in [`item`]. Only the forms the Farfisa processor uses are decoded (float add, subtract, multiply, pass,
//! integer add, register moves and swaps, DM / PM loads and stores, immediates, `modify`, loops,
//! jumps). Any other form is an error, so an unfamiliar OS version falls back instead of running
//! a wrong routine.

mod item;

use crate::dsp_image::ldr::SRAM_BYTES;
use crate::organ::b3_bass_fw::tables::kind;
use crate::organ::b3_bass_fw::Row;
use item::{item_row, split_items, sx};
use nord_sharc::visa::{decode_text, lw_flag, parcels_le, tile, word48, Insn, Kind};
use std::collections::HashMap;

/// One listed instruction: its short word address relative to the routine start and its text.
struct Line {
    addr: i64,
    text: String,
}

/// `(count, displacement)` of `do (pc, N) until lce, lcntr = C`.
fn parse_loop(text: &str) -> Result<Option<(i32, i64)>, String> {
    let Some(rest) = text.strip_prefix("do (pc,") else {
        return Ok(None);
    };
    let (disp, tail) = rest
        .split_once(')')
        .ok_or_else(|| format!("bad loop {text:?}"))?;
    let count = tail
        .trim()
        .strip_prefix("until lce, lcntr =")
        .ok_or_else(|| format!("bad loop {text:?}"))?;
    Ok(Some((sx(count)?, i64::from(sx(disp)?) & 0x7F_FFFF)))
}

/// `(condition code, target)` of `[if ne|eq] jump 0x...`.
fn parse_jump(text: &str) -> Result<Option<(i32, i64)>, String> {
    let (cond, rest) = match text.strip_prefix("if ") {
        Some(r) => match r.split_once(' ') {
            Some((c, rest)) if rest.starts_with("jump ") => (Some(c), rest),
            _ => return Ok(None),
        },
        None => (None, text),
    };
    let Some(target) = rest.strip_prefix("jump ") else {
        return Ok(None);
    };
    let code = match cond {
        None => 1,
        Some("ne") => 2,
        Some("eq") => 3,
        Some(c) => return Err(format!("unsupported jump condition {c:?}")),
    };
    Ok(Some((code, i64::from(sx(target)?))))
}

/// Closes the loops whose last instruction is `k`: their body length is the rows emitted since.
fn close_loops(open: &mut Vec<(usize, usize)>, rows: &mut [Row], k: usize) {
    let emitted = rows.len();
    while open.last().is_some_and(|&(_, end)| end == k) {
        let (start, _) = open.pop().unwrap_or((0, 0));
        rows[start][2] = (emitted - start - 1) as i32;
    }
}

/// Rows of a listing up to its `rts` (loops become `LOOP count n` plus `n` rows); `base` is the
/// short word address of the first instruction (jump targets are absolute).
fn decode(prog: &[Line], base: i64) -> Result<Vec<Row>, String> {
    let at: HashMap<i64, usize> = prog.iter().enumerate().map(|(k, p)| (p.addr, k)).collect();
    let mut rows: Vec<Row> = Vec::new();
    let mut row_of: HashMap<i64, usize> = HashMap::new();
    let mut fixups: Vec<(usize, i64)> = Vec::new();
    let mut open: Vec<(usize, usize)> = Vec::new();
    for (k, ins) in prog.iter().enumerate() {
        row_of.insert(ins.addr, rows.len());
        let text = ins.text.as_str();
        if text.starts_with("rts") {
            break;
        }
        if let Some((count, disp)) = parse_loop(text)? {
            let end = *at
                .get(&(ins.addr + disp))
                .ok_or_else(|| format!("loop end outside the listing at {:#x}", ins.addr))?;
            open.push((rows.len(), end));
            rows.push([kind::LOOP, count, 0, 0, 0, 0, 0, 0]);
            continue;
        }
        if let Some((code, target)) = parse_jump(text)? {
            fixups.push((rows.len(), target - base));
            rows.push([kind::INS, 0, 0, code, 0, 0, 0, 0]);
            close_loops(&mut open, &mut rows, k);
            continue;
        }
        let (body_text, cond) = match text.strip_prefix("if ") {
            Some(r) => match r.split_once(' ') {
                Some(("lt", rest)) => (rest, 1),
                _ => return Err(format!("unsupported condition in {text:?}")),
            },
            None => (text, 0),
        };
        let body: Vec<Row> = split_items(body_text)
            .iter()
            .filter(|i| !matches!(i.as_str(), "nop" | "rts (db)" | "rts"))
            .map(|i| item_row(i))
            .collect::<Result<_, _>>()?;
        rows.push([kind::INS, body.len() as i32, cond, 0, 0, 0, 0, 0]);
        rows.extend(body);
        close_loops(&mut open, &mut rows, k);
    }
    if !open.is_empty() {
        return Err("a loop is not closed before the routine ends".into());
    }
    for (row, target) in fixups {
        rows[row][4] = *row_of
            .get(&target)
            .ok_or_else(|| format!("jump target {target:#x} is not an instruction"))?
            as i32;
    }
    Ok(rows)
}

/// Lower case listing text of one instruction; `decode_text` leaves out the `(LW)` flag of a long
/// word move, which the rows need, so it is appended here (single move forms only).
fn listing_text(insn: &Insn) -> Result<String, String> {
    let mut text = decode_text(insn).to_ascii_lowercase();
    let moves = match insn.kind {
        Kind::T1a | Kind::T1b => 2,
        Kind::T3a | Kind::T3b | Kind::T4a | Kind::T4b | Kind::T5a => 1,
        _ => 0,
    };
    let items = split_items(&text);
    if moves > 0 && items.len() > moves {
        // ADI lists the compute of a move + compute instruction first, `decode_text` last; the
        // rows keep ADI's order (items are independent: reads see the old state)
        let (mv, compute) = items.split_at(moves);
        text = [compute, mv].concat().join(", ");
    }
    if lw_flag(insn.kind, word48(insn.words())) {
        if split_items(&text).len() != 1 {
            return Err(format!("unsupported long word form {text:?}"));
        }
        text.push_str(" (lw)");
    }
    Ok(text)
}

/// Rows of the routine `[entry, end)` (short word addresses) of a flat SRAM image.
pub(super) fn routine_rows(sram: &[u8], entry: u32, end: u32) -> Result<Vec<Row>, String> {
    let (lo, hi) = (
        2 * entry as usize,
        (2 * end as usize + 0x20).min(SRAM_BYTES),
    );
    let bytes = sram
        .get(lo..hi.min(sram.len()))
        .ok_or("routine outside the SRAM image")?;
    let (insns, _exact) = tile(&parcels_le(bytes));
    let mut pc = 0i64;
    let mut prog = Vec::with_capacity(insns.len());
    for insn in &insns {
        prog.push(Line {
            addr: pc,
            text: listing_text(insn)?,
        });
        pc += insn.len as i64;
    }
    decode(&prog, i64::from(entry))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(texts: &[&str]) -> Vec<Line> {
        texts
            .iter()
            .enumerate()
            .map(|(k, t)| Line {
                addr: k as i64,
                text: (*t).into(),
            })
            .collect()
    }

    #[test]
    fn a_counted_loop_a_conditional_jump_and_the_end_are_decoded() {
        let prog = lines(&[
            "do (pc, 2) until lce, lcntr = 3",
            "r1 = dm(i0, m1)",
            "f2 = f2 + f1",
            "if ne jump 0x5",
            "nop",
            "jump 0x6",
            "rts",
            "r0 = 1",
        ]);
        let rows = decode(&prog, 0).unwrap();
        let ins = |n, cond, j, t| [kind::INS, n, cond, j, t, 0, 0, 0];
        assert_eq!(rows.len(), 8);
        assert_eq!(
            rows[0],
            [kind::LOOP, 3, 4, 0, 0, 0, 0, 0],
            "body is the four rows after it"
        );
        assert_eq!(rows[1], ins(1, 0, 0, 0));
        assert_eq!(rows[3], ins(1, 0, 0, 0));
        assert_eq!(
            rows[5],
            ins(0, 0, 2, 7),
            "jump if ne to the instruction at 5 (row 7)"
        );
        assert_eq!(rows[6], ins(0, 0, 0, 0), "a nop is an empty instruction");
        assert_eq!(
            rows[7],
            ins(0, 0, 1, 8),
            "jump to the rts, one past the last row"
        );
    }

    #[test]
    fn unsupported_text_and_dangling_loops_are_errors() {
        assert!(decode(&lines(&["if gt r1 = r2"]), 0).is_err());
        assert!(
            decode(
                &lines(&["do (pc, 2) until lce, lcntr = 2", "nop", "rts"]),
                0
            )
            .is_err(),
            "the routine ends inside the loop"
        );
        assert!(decode(&lines(&["do (pc, 9) until lce, lcntr = 2"]), 0).is_err());
        assert!(decode(&lines(&["jump 0x40"]), 0).is_err());
        assert!(decode(&lines(&["if ge jump 0x0"]), 0).is_err());
        assert!(routine_rows(&[0u8; 4], 10, 20).is_err());
    }
}
