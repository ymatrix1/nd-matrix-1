//! Ordering of the items of one instruction for in place execution (see `compile.rs`).
//!
//! Every item of a SHARC instruction reads the old state. The read set and write set of each item
//! are bit masks over the machine state; the items are put in an order in which plain in place
//! execution reads every old value before it is overwritten. When no such order exists (a
//! multiply and an add that feed each other, a store and a load of the same register) one or two
//! items are split into a compute half (reads now, result into a temp slot) and a commit half
//! (the write), which breaks the cycle. Only an instruction with a double register write or a
//! swap that still has no order keeps the reference path.

use super::tables::{alu_op as A, kind as K};
use super::{exec, Row};

/// Resource bit positions: data registers 0..32, `I` 32.., `M` 48.., `L` 64.., `B` 80.., then
/// the loop counter, memory, the multiplier accumulators and the AN / AZ flags.
const I0: u32 = 32;
const M0: u32 = 48;
const L0: u32 = 64;
const B0: u32 = 80;
const LC: u128 = 1 << 96;
const MEM: u128 = 1 << 97;
const MR: u128 = 1 << 98;
const FL: u128 = 1 << 99;
/// Resources whose double write keeps source order (applied in item order by the reference too).
const ORDERED: u128 = MEM | MR | FL;

const fn d(id: usize) -> u128 {
    1 << id
}

/// Read resources of a 32 bit register id (`Machine::get32`).
fn rd32(id: usize) -> u128 {
    match id {
        0..=31 => d(id),
        32..=47 => 1 << (I0 as usize + id - 32),
        48..=63 => 1 << (M0 as usize + id - 48),
        64..=79 => 1 << (L0 as usize + id - 64),
        80..=95 => 1 << (B0 as usize + id - 80),
        _ => LC,
    }
}

/// Write resources of a 32 bit register id (`Machine::put32`: a `B` write loads `I` too).
fn wr32(id: usize) -> u128 {
    match id {
        80..=95 => rd32(id) | rd32(id - 48),
        _ => rd32(id),
    }
}

/// Resources of the address operand at `at`: (reads, writes) (`Machine::addr`).
fn addr_rw(row: &Row, at: usize) -> (u128, u128) {
    let (kind, p, q) = (row[at], row[at + 1] as usize, row[at + 2] as usize);
    if kind == 0 {
        return (0, 0);
    }
    let ip = rd32(32 + p) | rd32(64 + p) | rd32(80 + p);
    let m = if kind == 1 || kind == 3 {
        rd32(48 + q)
    } else {
        0
    };
    (ip | m, if kind <= 2 { rd32(32 + p) } else { 0 })
}

const fn twin(id: usize) -> usize {
    exec::twin(id)
}

/// Reads and writes of one item row (mirrors `Machine::item`).
fn rw(row: &Row) -> (u128, u128) {
    let r = |k: usize| row[k] as usize;
    match row[0] {
        K::SWAP => (d(r(1)) | d(r(2)), d(r(1)) | d(r(2))),
        K::STORE => {
            let (ar, aw) = addr_rw(row, 2);
            let reg = r(1);
            let src = if row[5] & 1 != 0 {
                rd32(reg & !1) | rd32(reg | 1)
            } else {
                rd32(reg) | rd32(twin(reg))
            };
            (ar | src, aw | MEM)
        }
        K::LOAD => {
            let (ar, aw) = addr_rw(row, 2);
            let reg = r(1);
            let dst = if row[5] & 1 != 0 {
                wr32(reg & !1) | wr32(reg | 1)
            } else if reg < 32 {
                wr32(reg) | wr32(twin(reg))
            } else {
                wr32(reg)
            };
            (ar | MEM, aw | dst)
        }
        K::MODIFY => {
            let ia = r(2);
            let m = if row[3] != 0 { rd32(48 + r(4)) } else { 0 };
            (
                rd32(32 + ia) | rd32(64 + ia) | rd32(80 + ia) | m,
                wr32(r(1)),
            )
        }
        K::MOVE_N => (rd32(r(2)), wr32(r(1))),
        K::MOVE_NI => (0, wr32(r(1))),
        K::MOVE_D => (rd32(r(2)) | rd32(twin(r(2))), wr32(r(1)) | wr32(twin(r(1)))),
        K::MOVE_DN => (rd32(r(2)), wr32(r(1)) | wr32(twin(r(1)))),
        K::MOVE_DI => (0, wr32(r(1)) | wr32(twin(r(1)))),
        K::ALU => {
            let (x, y) = (r(3), r(4));
            let op = row[2];
            let both = |n: usize| d(n) | d(16 + n);
            let reads = match op {
                A::PASS | A::ABS | A::FEXT_I | A::FDEP_I | A::ADDI => both(x),
                A::MULT => both(x) | both(y) | MR,
                _ => both(x) | both(y),
            };
            let flags = if exec::sets_flags(op) { FL } else { 0 };
            (reads, d(r(1)) | d(twin(r(1))) | flags)
        }
        K::MRF_SET => (d(r(1)) | d(16 + r(1)) | d(r(2)) | d(16 + r(2)) | MR, MR),
        _ => (d(r(1)) | d(16 + r(1)) | d(r(2)) | d(16 + r(2)), FL), // COMP
    }
}

/// One executable piece of a planned instruction: its row and its resource masks.
struct Piece {
    row: Row,
    reads: u128,
    writes: u128,
}

/// An order of `pieces` in which in place execution equals the read-old-write-new semantics, or
/// `None` when no such order exists (cycle) or two pieces write the same register. `after` lists
/// `(first, second)` pairs that must run in that order (a compute half before its commit).
fn order(pieces: &[Piece], after: &[(usize, usize)]) -> Option<Vec<usize>> {
    let n = pieces.len();
    // before[j] bit i: piece i must run before piece j
    let mut before = vec![0u64; n];
    for i in 0..n {
        for j in i + 1..n {
            let (ri, wi) = (pieces[i].reads, pieces[i].writes);
            let (rj, wj) = (pieces[j].reads, pieces[j].writes);
            if wi & wj & !ORDERED != 0 {
                return None;
            }
            if wi & wj != 0 {
                before[j] |= 1 << i;
            }
            if ri & wj != 0 {
                before[j] |= 1 << i; // i reads the old value first
            }
            if wi & rj != 0 {
                before[i] |= 1 << j; // j reads the old value first
            }
        }
    }
    for &(first, second) in after {
        before[second] |= 1 << first;
    }
    let (mut done, mut out) = (0u64, Vec::with_capacity(n));
    while out.len() < n {
        let next = (0..n).find(|&k| done >> k & 1 == 0 && before[k] & !done == 0)?;
        done |= 1 << next;
        out.push(next);
    }
    Some(out)
}

fn piece(row: &Row) -> Piece {
    let (reads, writes) = rw(row);
    Piece {
        row: *row,
        reads,
        writes,
    }
}

/// An order of `items` for in place execution without splitting, or `None` (see [`order`]).
#[cfg(test)]
pub(super) fn safe_order(items: &[Row]) -> Option<Vec<usize>> {
    let pieces: Vec<Piece> = items.iter().map(piece).collect();
    order(&pieces, &[])
}

/// The deferred part of an item: (resources written at the commit, compute kind, commit kind).
fn deferred(row: &Row, writes: u128) -> Option<(u128, i32, i32)> {
    match row[0] {
        K::ALU => Some((writes, K::ALU_T, K::COMMIT_ALU)),
        K::STORE => Some((MEM, K::STORE_T, K::COMMIT_STORE)),
        K::LOAD => {
            let reg = row[1] as usize;
            let dst = if row[5] & 1 != 0 {
                wr32(reg & !1) | wr32(reg | 1)
            } else if reg < 32 {
                wr32(reg) | wr32(twin(reg))
            } else {
                wr32(reg)
            };
            Some((dst, K::LOAD_T, K::COMMIT_LOAD))
        }
        _ => None,
    }
}

/// The rows of `items` in a safe order with the items in `split` cut into compute and commit
/// halves (temp slot = position in `split`), or `None`.
fn plan_split(items: &[Row], split: &[usize]) -> Option<Vec<Row>> {
    let mut pieces = Vec::new();
    let mut after = Vec::new();
    for (k, row) in items.iter().enumerate() {
        let Some(slot) = split.iter().position(|&s| s == k) else {
            pieces.push(piece(row));
            continue;
        };
        let (reads, writes) = rw(row);
        let (late, compute, commit) = deferred(row, writes)?;
        let (mut c, mut w) = (*row, *row);
        (c[0], c[7], w[0], w[7]) = (compute, slot as i32, commit, slot as i32);
        after.push((pieces.len(), pieces.len() + 1));
        pieces.push(Piece {
            row: c,
            reads,
            writes: writes & !late,
        });
        pieces.push(Piece {
            row: w,
            reads: 0,
            writes: late,
        });
    }
    let ordered = order(&pieces, &after)?;
    Some(ordered.into_iter().map(|k| pieces[k].row).collect())
}

/// Rows for in place execution of one unconditional instruction: the plain safe order when it
/// exists, else the first split of one, then of two items that gives an order; `None` keeps the
/// reference path.
pub(super) fn plan(items: &[Row]) -> Option<Vec<Row>> {
    let n = items.len();
    if n > 28 {
        return None;
    }
    if let Some(rows) = plan_split(items, &[]) {
        return Some(rows);
    }
    let cands: Vec<usize> = (0..n)
        .filter(|&k| matches!(items[k][0], K::ALU | K::STORE | K::LOAD))
        .collect();
    for &a in &cands {
        if let Some(rows) = plan_split(items, &[a]) {
            return Some(rows);
        }
    }
    for (x, &a) in cands.iter().enumerate() {
        for &b in &cands[x + 1..] {
            if let Some(rows) = plan_split(items, &[a, b]) {
                return Some(rows);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_to_a_register_that_a_store_reads_runs_after_the_store() {
        // `dm(0x100) = r1` and `r1 = 5`: the store reads the old r1, so it goes first
        let store = [K::STORE, 1, 0, 0x100, 0, 0, 0, 0];
        let set = [K::MOVE_DI, 1, 5, 0, 0, 0, 0, 0];
        assert_eq!(safe_order(&[set, store]).unwrap(), vec![1, 0]);
    }

    #[test]
    fn cycles_and_double_register_writes_have_no_plain_order() {
        // a load and a store that read each other's destination: no in place order exists
        let store = [K::STORE, 1, 0, 0x100, 0, 0, 0, 0];
        let load = [K::LOAD, 1, 0, 0x200, 0, 2, 0, 0];
        assert!(safe_order(&[load, store]).is_none());
        let (a, b) = (
            [K::MOVE_N, 33, 34, 0, 0, 0, 0, 0],
            [K::MOVE_N, 34, 33, 0, 0, 0, 0, 0],
        );
        assert!(safe_order(&[a, b]).is_none());
        assert!(plan(&[a, b]).is_none(), "moves are not splittable");
        let a = [K::MOVE_DI, 3, 1, 0, 0, 0, 0, 0];
        let b = [K::MOVE_DI, 3, 2, 0, 0, 0, 0, 0];
        assert!(safe_order(&[a, b]).is_none());
        assert!(plan(&[a, b]).is_none());
    }

    #[test]
    fn a_multiply_add_pair_that_feeds_itself_is_split_not_referenced() {
        // `r12 = r0 * r5, r0 = r8 + r12`: each reads what the other writes
        let mul = [K::ALU, 12, A::FMUL, 0, 5, 0, 0, 0];
        let add = [K::ALU, 0, A::FADD, 8, 12, 0, 0, 0];
        assert!(safe_order(&[mul, add]).is_none());
        let rows = plan(&[mul, add]).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().any(|r| r[0] == K::ALU_T));
        assert!(rows.iter().any(|r| r[0] == K::COMMIT_ALU));
    }

    #[test]
    fn a_store_and_a_load_that_trade_through_memory_are_split() {
        let store = [K::STORE, 3, 0, 0x100, 0, 0, 0, 0];
        let load = [K::LOAD, 3, 0, 0x100, 0, 0, 0, 0];
        let rows = plan(&[store, load]).unwrap();
        assert!(rows.iter().any(|r| r[0] == K::STORE_T || r[0] == K::LOAD_T));
    }

    #[test]
    fn a_swap_is_one_item_and_runs_in_place() {
        let swap = [K::SWAP, 0, 1, 0, 0, 0, 0, 0];
        assert_eq!(safe_order(&[swap]).unwrap(), vec![0]);
    }
}
