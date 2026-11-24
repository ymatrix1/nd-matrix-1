//! Compiles the decoded rows into a flat program whose instructions run item by item in place.
//!
//! The reference executor (`exec.rs`) collects the writes of one instruction in a [`Pending`]
//! buffer and applies them together, because every item of a SHARC instruction reads the old state.
//! That buffer and the register file copy of a conditional instruction cost more than the
//! arithmetic. Here every instruction is analysed once, at setup time: the read set and write set
//! of each item are bit masks over the machine state, and the items are put in an order in which
//! plain in-place execution reads every old value before it is overwritten (`plan.rs`; a cycle is
//! broken by splitting an item into compute and commit halves). A conditional ALU item runs as
//! `COND_ALU`. A swap, a double write to a register or any other condition keeps the reference
//! path (`ctl::REF`). All paths are bit exact (`fastexec::tests` runs them side by side on the oracle
//! chords and compares the whole machine state).

use super::plan::plan;
use super::tables::kind as K;
use super::Row;

/// Op kind of an ALU item in a compiled program: `ALUK + operation`, so one dispatch per item
/// reaches the specialised arm (`fastexec.rs`). The kinds below are `tables::kind`.
pub(super) const ALUK: i32 = 32;

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
                let planned = match (row[2], items) {
                    (0, _) => plan(items),
                    (_, [item]) if item[0] == K::ALU => {
                        let mut cond = *item;
                        cond[0] = K::COND_ALU;
                        Some(vec![cond])
                    }
                    _ => None,
                };
                match planned {
                    Some(order) => prog.ops.extend(order.into_iter().map(|mut r| {
                        if r[0] == K::ALU {
                            r[0] = ALUK + r[2];
                        }
                        r
                    })),
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
    use crate::organ::b3_bass_fw::real_image;

    fn ins(items: &[Row]) -> Vec<Row> {
        let mut v = vec![[K::INS, items.len() as i32, 0, 0, 0, 0, 0, 0]];
        v.extend_from_slice(items);
        v
    }

    #[test]
    fn a_conditional_instruction_keeps_the_reference_path_unless_it_is_one_alu_item() {
        let mut rows = ins(&[[K::MOVE_DI, 5, 99, 0, 0, 0, 0, 0]]);
        rows[0][2] = 1;
        assert_eq!(compile(&rows).ops[0][..4], [ctl::REF, 0, 1, 1]);
        let mut rows = ins(&[[K::ALU, 5, 4, 5, 6, 0, 0, 0]]);
        rows[0][2] = 1;
        assert_eq!(compile(&rows).ops[0][0], K::COND_ALU);
    }

    #[test]
    fn a_cycle_of_two_alu_items_is_compiled_not_referenced() {
        let rows = ins(&[
            [K::ALU, 12, 1, 0, 5, 0, 0, 0],
            [K::ALU, 0, 2, 8, 12, 0, 0, 0],
        ]);
        let p = compile(&rows);
        assert!(p.refs.is_empty() && p.ops.len() == 3);
    }

    #[test]
    fn nearly_every_instruction_of_the_routines_is_reordered_not_referenced() {
        // needs the user's OS update file (`NS4_WAVE_TABLES`); skipped with a note without it
        let Some(image) = real_image() else { return };
        for rows in [&image.gen[..], &image.proc[..]] {
            let p = compile(rows);
            let slow = p.ops.iter().filter(|o| o[0] == ctl::REF).count();
            assert!(
                slow <= 3 && p.ops.len() > 300,
                "reference {slow} of {}",
                p.ops.len()
            );
        }
    }
}
