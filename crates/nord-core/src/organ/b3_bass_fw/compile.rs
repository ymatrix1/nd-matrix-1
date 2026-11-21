//! Compiles the decoded rows into a flat program whose instructions run item by item in place.
//!
//! The reference executor (`exec.rs`) collects the writes of one instruction in a [`Pending`]
//! buffer and applies them together, because every item of a SHARC instruction reads the old state.
//! That buffer and the register file copy of a conditional instruction cost more than the
//! arithmetic. Here every instruction is analysed once, at setup time: the read set and write set
//! of each item are bit masks over the machine state, and the items are put in an order in which
//! plain in-place execution reads every old value before it is overwritten. An instruction with a
//! cycle (a swap), a double write to a register, or a condition keeps the reference path
//! (`Op::Ref`). Both paths are bit exact (`fastexec::tests` runs them side by side on the oracle
//! chords and compares the whole machine state).

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

/// Control ops of the flat program; every other op is an item row (`tables::kind`).
pub(super) mod ctl {
    /// `[JUMP, kind, target op]`: kind 1 always, 2 `if ne`, 3 `if eq`.
    pub const JUMP: i32 = 100;
    /// `[LOOP, count, body ops]`: `count` passes over the next `body` ops.
    pub const LOOP: i32 = 101;
    /// `[REF, start, n, cond]`: an instruction that keeps the reference path (`refs`).
    pub const REF: i32 = 102;
}

/// A compiled routine: one flat list of item rows (the items of an instruction in a safe order,
/// one dispatch per item) and control ops.
#[derive(Clone, Debug, Default)]
pub(super) struct Program {
    pub(super) ops: Vec<Row>,
    /// Original item rows of the `REF` instructions.
    pub(super) refs: Vec<Row>,
}

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

/// An order of `items` in which in place execution equals the read-old-write-new semantics, or
/// `None` when no such order exists (cycle) or two items write the same register.
fn safe_order(items: &[Row]) -> Option<Vec<usize>> {
    let n = items.len();
    let sets: Vec<(u128, u128)> = items.iter().map(rw).collect();
    // before[j] bit i: item i must run before item j
    let mut before = vec![0u32; n];
    for i in 0..n {
        for j in i + 1..n {
            let (ri, wi) = sets[i];
            let (rj, wj) = sets[j];
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
    let (mut done, mut order) = (0u32, Vec::with_capacity(n));
    while order.len() < n {
        let next = (0..n).find(|&k| done >> k & 1 == 0 && before[k] & !done == 0)?;
        done |= 1 << next;
        order.push(next);
    }
    Some(order)
}

/// Compiles decoded rows (setup time only: allocates).
pub(super) fn compile(rows: &[Row]) -> Program {
    let mut prog = Program::default();
    let mut row_op = vec![u32::MAX; rows.len() + 1];
    let mut fixups: Vec<(usize, usize)> = Vec::new(); // (op index, target row)
    emit(rows, 0, rows.len(), &mut prog, &mut row_op, &mut fixups);
    for (op, target) in fixups {
        let t = row_op[target];
        assert_ne!(t, u32::MAX, "jump into the middle of a row body");
        prog.ops[op][2] = t as i32;
    }
    prog
}

fn emit(
    rows: &[Row],
    lo: usize,
    hi: usize,
    prog: &mut Program,
    row_op: &mut [u32],
    fixups: &mut Vec<(usize, usize)>,
) {
    let mut p = lo;
    while p < hi {
        let row = rows[p];
        row_op[p] = prog.ops.len() as u32;
        if row[0] == K::INS {
            let n = row[1] as usize;
            if row[3] != 0 {
                fixups.push((prog.ops.len(), row[4] as usize));
                prog.ops.push([ctl::JUMP, row[3], 0, 0, 0, 0, 0, 0]);
            } else {
                let items = &rows[p + 1..p + 1 + n];
                match (row[2] == 0 && n <= 30)
                    .then(|| safe_order(items))
                    .flatten()
                {
                    Some(order) => prog.ops.extend(order.iter().map(|&k| items[k])),
                    None => {
                        let start = prog.refs.len() as i32;
                        prog.refs.extend_from_slice(items);
                        prog.ops
                            .push([ctl::REF, start, n as i32, row[2], 0, 0, 0, 0]);
                    }
                }
            }
            p += 1 + n;
        } else {
            let n = row[2] as usize;
            let at = prog.ops.len();
            prog.ops.push([ctl::LOOP, row[1], 0, 0, 0, 0, 0, 0]);
            emit(rows, p + 1, p + 1 + n, prog, row_op, fixups);
            prog.ops[at][2] = (prog.ops.len() - at - 1) as i32;
            p += 1 + n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organ::b3_bass_fw::{GEN_ROWS, PROC_ROWS};

    fn ins(items: &[Row]) -> Vec<Row> {
        let mut v = vec![[K::INS, items.len() as i32, 0, 0, 0, 0, 0, 0]];
        v.extend_from_slice(items);
        v
    }

    #[test]
    fn a_write_to_a_register_that_a_store_reads_runs_after_the_store() {
        // `dm(0x100) = r1` and `r1 = 5`: the store reads the old r1, so it goes first
        let store = [K::STORE, 1, 0, 0x100, 0, 0, 0, 0];
        let set = [K::MOVE_DI, 1, 5, 0, 0, 0, 0, 0];
        assert_eq!(safe_order(&[set, store]).unwrap(), vec![1, 0]);
    }

    #[test]
    fn cycles_and_double_register_writes_keep_the_reference_path() {
        // a load and a store that read each other's destination: no in place order exists
        let store = [K::STORE, 1, 0, 0x100, 0, 0, 0, 0];
        let load = [K::LOAD, 1, 0, 0x200, 0, 2, 0, 0];
        assert!(safe_order(&[load, store]).is_none());
        let (a, b) = (
            [K::MOVE_N, 33, 34, 0, 0, 0, 0, 0],
            [K::MOVE_N, 34, 33, 0, 0, 0, 0, 0],
        );
        assert!(safe_order(&[a, b]).is_none());
        let a = [K::MOVE_DI, 3, 1, 0, 0, 0, 0, 0];
        let b = [K::MOVE_DI, 3, 2, 0, 0, 0, 0, 0];
        assert!(safe_order(&[a, b]).is_none());
        let p = compile(&ins(&[a, b]));
        assert_eq!((p.ops[0][0], p.ops[0][3]), (ctl::REF, 0));
    }

    #[test]
    fn a_swap_is_one_item_and_runs_in_place() {
        let swap = [K::SWAP, 0, 1, 0, 0, 0, 0, 0];
        assert_eq!(safe_order(&[swap]).unwrap(), vec![0]);
    }

    #[test]
    fn a_conditional_instruction_keeps_the_reference_path() {
        let mut rows = ins(&[[K::MOVE_DI, 5, 99, 0, 0, 0, 0, 0]]);
        rows[0][2] = 1;
        assert_eq!(compile(&rows).ops[0][..4], [ctl::REF, 0, 1, 1]);
    }

    #[test]
    fn nearly_every_instruction_of_the_routines_is_reordered_not_referenced() {
        for rows in [&GEN_ROWS[..], &PROC_ROWS[..]] {
            let p = compile(rows);
            let slow = p.ops.iter().filter(|o| o[0] == ctl::REF).count();
            assert!(
                slow <= 6 && p.ops.len() > 300,
                "reference {slow} of {}",
                p.ops.len()
            );
        }
    }
}
