//! In place executor of a compiled [`Program`] (see `compile.rs` for why the order is safe).
//!
//! Same rules as `exec.rs`, without the pending write buffer: loads, stores, index updates and ALU
//! results are applied as the item runs. The 40 bit float add and multiply are the shared
//! `f40` routines (64 bit fast paths, zero operands included).

use super::compile::{ctl, Program};
use super::exec::{self, alu_flags, sets_flags, twin};
use super::f40;
use super::tables::{alu_op as A, kind as K};
use super::{Machine, Row};

impl Machine {
    /// One routine call of a compiled program with the `I0` / `I1` presets.
    pub(super) fn call_compiled(&mut self, prog: &Program, preset: (u32, u32)) {
        self.i[0] = preset.0;
        self.i[1] = preset.1;
        let mut pd = exec::Pending::default();
        self.run_ops(prog, 0, prog.ops.len(), &mut pd);
    }

    fn run_ops(&mut self, prog: &Program, mut p: usize, end: usize, pd: &mut exec::Pending) {
        while p < end {
            let row = &prog.ops[p];
            p += 1;
            match row[0] {
                ctl::JUMP => {
                    // 1 always, 2 `if ne` (AZ clear), 3 `if eq`; PEx flags, no delay slots
                    let taken = row[1] == 1 || (row[1] == 2) != self.az[0];
                    if taken {
                        p = row[2] as usize;
                    }
                }
                ctl::LOOP => {
                    let body = row[2] as usize;
                    for _ in 0..row[1] {
                        self.run_ops(prog, p, p + body, pd);
                    }
                    self.lcntr = 0;
                    p += body;
                }
                ctl::REF => {
                    let (s, n) = (row[1] as usize, row[2] as usize);
                    self.exec(&prog.refs[s..s + n], row[3] != 0, pd);
                }
                _ => self.apply(row),
            }
        }
    }

    /// Effective address of the operand at `at`; a post modify updates the index in place.
    #[inline(always)]
    fn ea(&mut self, row: &Row, at: usize) -> u32 {
        let (kind, p, q) = (row[at], row[at + 1], row[at + 2]);
        if kind == 0 {
            return p as u32;
        }
        let ia = p as usize;
        let step = if kind == 1 || kind == 3 {
            self.m[q as usize] as i32
        } else {
            q
        };
        let sum = i64::from(self.i[ia]) + i64::from(step);
        let moved = if self.l[ia] == 0 {
            sum as u32
        } else {
            self.wrap(ia, sum)
        };
        if kind <= 2 {
            std::mem::replace(&mut self.i[ia], moved)
        } else {
            moved
        }
    }

    #[inline(always)]
    fn apply(&mut self, row: &Row) {
        let r = |k: usize| row[k] as usize;
        match row[0] {
            K::LOAD => {
                let a = self.ea(row, 2);
                let reg = r(1);
                if row[5] & 1 != 0 {
                    let n = reg & !1;
                    let (lo, hi) = (self.rd(a & !1), self.rd(a | 1));
                    self.put32(n, lo);
                    self.put32(n | 1, hi);
                } else if reg < 32 {
                    let step = if row[5] & 2 != 0 { 0 } else { 1 };
                    let (lo, hi) = (self.rd(a), self.rd(a.wrapping_add(step)));
                    self.d[reg] = u64::from(lo) << 8;
                    self.d[twin(reg)] = u64::from(hi) << 8;
                } else {
                    let v = self.rd(a);
                    self.put32(reg, v);
                }
            }
            K::STORE => {
                let a = self.ea(row, 2);
                let reg = r(1);
                if row[5] & 1 != 0 {
                    let n = reg & !1;
                    let (lo, hi) = (self.get32(n), self.get32(n | 1));
                    self.wr(a & !1, lo);
                    self.wr(a | 1, hi);
                } else {
                    let (lo, hi) = (self.get32(reg), self.get32(twin(reg)));
                    self.wr(a, lo);
                    self.wr(a.wrapping_add(1), hi);
                }
            }
            K::SWAP => {
                let (a, b) = (self.get40(r(1)), self.get40(r(2)));
                self.d[r(1)] = b;
                self.d[r(2)] = a;
            }
            K::ALU => self.apply_alu(row),
            K::MODIFY => {
                let ia = r(2);
                let step = if row[3] != 0 {
                    self.m[r(4)] as i32
                } else {
                    row[4]
                };
                let v = self.wrap(ia, i64::from(self.i[ia]) + i64::from(step));
                self.put32(r(1), v);
            }
            K::MOVE_N => {
                let v = self.get32(r(2));
                self.put32(r(1), v);
            }
            K::MOVE_NI => self.put32(r(1), row[2] as u32),
            K::MOVE_D => {
                let (a, b) = (self.get40(r(2)), self.get40(twin(r(2))));
                self.d[r(1)] = a;
                self.d[twin(r(1))] = b;
            }
            K::MOVE_DN | K::MOVE_DI => {
                let v = if row[0] == K::MOVE_DN {
                    self.get32(r(2))
                } else {
                    row[2] as u32
                };
                self.d[r(1)] = u64::from(v) << 8;
                self.d[twin(r(1))] = u64::from(v) << 8;
            }
            K::MRF_SET => {
                let a = self.mult_for(0, row);
                let b = self.mult_for(1, row);
                self.mr = [a, b];
            }
            K::COMP => {
                for pe in 0..2 {
                    let w = |n: usize| (self.d[pe * 16 + n] >> 8) as u32 as i32;
                    let (x, y) = (w(r(1)), w(r(2)));
                    (self.an[pe], self.az[pe]) = (x < y, x == y);
                }
            }
            _ => unreachable!("decoded rows hold only the item kinds above"),
        }
    }

    fn mult_for(&self, pe: usize, row: &Row) -> i128 {
        self.mult(pe, row, false).0
    }

    /// ALU item on both elements. The float and the common integer operations are inlined with
    /// the match outside the per element loop; the rest goes through `Machine::alu`.
    #[inline(always)]
    fn apply_alu(&mut self, row: &Row) {
        let (rn, op) = (row[1] as usize & 31, row[2]);
        let (x, y) = (row[3] as usize & 15, row[4] as usize & 15);
        let (x0, x1, y0, y1) = (self.d[x], self.d[16 + x], self.d[y], self.d[16 + y]);
        let w = |v: u64| (v >> 8) as u32;
        let int = |v: u32| u64::from(v) << 8;
        let (p, q) = (row[5], row[6]);
        let (r0, r1) = match op {
            A::FMUL => (f40::mul(x0, y0), f40::mul(x1, y1)),
            A::FADD => (f40::add(x0, y0, false), f40::add(x1, y1, false)),
            A::FSUB => (f40::add(x0, y0, true), f40::add(x1, y1, true)),
            A::PASS => (f40::pass(x0), f40::pass(x1)),
            A::IADD => (
                int(w(x0).wrapping_add(w(y0))),
                int(w(x1).wrapping_add(w(y1))),
            ),
            A::ISUB => (
                int(w(x0).wrapping_sub(w(y0))),
                int(w(x1).wrapping_sub(w(y1))),
            ),
            A::ADDI => (
                int(w(x0).wrapping_add(row[4] as u32)),
                int(w(x1).wrapping_add(row[4] as u32)),
            ),
            A::AND => (int(w(x0) & w(y0)), int(w(x1) & w(y1))),
            A::FLOATBY => (
                f40::float_by(w(x0) as i32, w(y0) as i32),
                f40::float_by(w(x1) as i32, w(y1) as i32),
            ),
            A::FEXT_I => (
                int(f40::fext(w(x0), p as u32, row[4] as u32, q != 0)),
                int(f40::fext(w(x1), p as u32, row[4] as u32, q != 0)),
            ),
            A::FDEP_I => (
                int(f40::fdep(w(x0), p as u32, row[4] as u32, q != 0)),
                int(f40::fdep(w(x1), p as u32, row[4] as u32, q != 0)),
            ),
            A::ASHIFT | A::LSHIFT => {
                let arith = op == A::ASHIFT;
                (
                    int(f40::shift(w(x0), w(y0) as i32, arith)),
                    int(f40::shift(w(x1), w(y1) as i32, arith)),
                )
            }
            _ => (self.alu(row, 0), self.alu(row, 1)),
        };
        let flags = sets_flags(op).then(|| [alu_flags(op, r0), alu_flags(op, r1)]);
        self.d[rn] = r0;
        self.d[twin(rn)] = r1;
        if let Some(fl) = flags {
            for (pe, (an, az)) in fl.into_iter().enumerate() {
                (self.an[pe], self.az[pe]) = (an, az);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::compile::compile;
    use super::super::{GEN_PRESET, GEN_ROWS, PROC_PRESET, PROC_ROWS};
    use super::*;

    fn chord_machine() -> Machine {
        let mut m = Machine::new();
        m.load_image(include_str!(
            "../../../../../docs/research/data/bass-fw-image.csv"
        ));
        m.load_image(include_str!(
            "../../../../../docs/research/data/bass-fw-chord-delta.csv"
        ));
        m
    }

    #[test]
    fn compiled_chain_equals_the_reference_machine_state_for_state() {
        // 600 generator + processor calls of the oracle chord: after every call the whole machine
        // (registers, index registers, flags, accumulators, DM) of both executors is equal.
        let (gen, proc_) = (compile(&GEN_ROWS), compile(&PROC_ROWS));
        let (mut a, mut b) = (chord_machine(), chord_machine());
        b.set_programs(Some(&GEN_ROWS), &PROC_ROWS);
        for call in 0..600 {
            a.call(&GEN_ROWS, GEN_PRESET);
            a.call(&PROC_ROWS, PROC_PRESET);
            b.call_compiled(&gen, GEN_PRESET);
            b.call_compiled(&proc_, PROC_PRESET);
            assert_eq!(a.d, b.d, "d, call {call}");
            assert_eq!(
                (a.i, a.m, a.l, a.b),
                (b.i, b.m, b.l, b.b),
                "index, call {call}"
            );
            assert_eq!(
                (a.mr, a.an, a.az, a.lcntr),
                (b.mr, b.an, b.az, b.lcntr),
                "flags"
            );
            assert!(a.mem == b.mem, "DM, call {call}");
        }
    }
}
