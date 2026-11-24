//! In place executor of a compiled [`Program`] (see `compile.rs` for why the order is safe).
//!
//! Same rules as `exec.rs`, without the pending write buffer: loads, stores, index updates and ALU
//! results are applied as the item runs. The 40 bit float add and multiply are the shared
//! `f40` routines (64 bit fast paths, zero operands included).

use super::compile::{ctl, Program, ALUK};
use super::exec::{self, alu_flags, sets_flags, twin};
use super::f40;
use super::tables::{alu_op as A, kind as K};
use super::{Machine, Row};

const ALU_FMUL: i32 = ALUK + A::FMUL;
const ALU_FADD: i32 = ALUK + A::FADD;
const ALU_FSUB: i32 = ALUK + A::FSUB;
const ALU_PASS: i32 = ALUK + A::PASS;
const ALU_IADD: i32 = ALUK + A::IADD;
const ALU_ISUB: i32 = ALUK + A::ISUB;
const ALU_ADDI: i32 = ALUK + A::ADDI;
const ALU_AND: i32 = ALUK + A::AND;
const ALU_FLOATBY: i32 = ALUK + A::FLOATBY;
const ALU_FEXT_I: i32 = ALUK + A::FEXT_I;
const ALU_FDEP_I: i32 = ALUK + A::FDEP_I;
const ALU_ASHIFT: i32 = ALUK + A::ASHIFT;
const ALU_LSHIFT: i32 = ALUK + A::LSHIFT;

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
                _ => self.run_item(row),
            }
        }
    }

    /// One item row of a compiled program (not a control op).
    #[inline(always)]
    pub(super) fn run_item(&mut self, row: &Row) {
        match row[0] {
            ALU_FMUL => self.alu_item::<{ A::FMUL }>(row),
            ALU_FADD => self.alu_item::<{ A::FADD }>(row),
            ALU_FSUB => self.alu_item::<{ A::FSUB }>(row),
            ALU_PASS => self.alu_item::<{ A::PASS }>(row),
            ALU_IADD => self.alu_item::<{ A::IADD }>(row),
            ALU_ISUB => self.alu_item::<{ A::ISUB }>(row),
            ALU_ADDI => self.alu_item::<{ A::ADDI }>(row),
            ALU_AND => self.alu_item::<{ A::AND }>(row),
            ALU_FLOATBY => self.alu_item::<{ A::FLOATBY }>(row),
            ALU_FEXT_I => self.alu_item::<{ A::FEXT_I }>(row),
            ALU_FDEP_I => self.alu_item::<{ A::FDEP_I }>(row),
            ALU_ASHIFT => self.alu_item::<{ A::ASHIFT }>(row),
            ALU_LSHIFT => self.alu_item::<{ A::LSHIFT }>(row),
            k if k >= ALUK => self.alu_item::<-1>(row), // the rare operations: decoded at run time
            _ => self.apply(row),
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
                let (lo, hi) = self.load_words(row, a);
                self.load_commit(row, lo, hi);
            }
            K::STORE => {
                let a = self.ea(row, 2);
                let (a0, a1, lo, hi) = self.store_words(row, a);
                self.wr(a0, lo);
                self.wr(a1, hi);
            }
            K::LOAD_T => {
                let a = self.ea(row, 2);
                let (lo, hi) = self.load_words(row, a);
                self.tmp[row[7] as usize * 4..][..2]
                    .copy_from_slice(&[u64::from(lo), u64::from(hi)]);
            }
            K::STORE_T => {
                let a = self.ea(row, 2);
                let (a0, a1, lo, hi) = self.store_words(row, a);
                let t = &mut self.tmp[row[7] as usize * 4..][..4];
                t.copy_from_slice(&[u64::from(a0), u64::from(a1), u64::from(lo), u64::from(hi)]);
            }
            K::COMMIT_LOAD => {
                let t = &self.tmp[row[7] as usize * 4..];
                let (lo, hi) = (t[0] as u32, t[1] as u32);
                self.load_commit(row, lo, hi);
            }
            K::COMMIT_STORE => {
                let t = &self.tmp[row[7] as usize * 4..];
                let (a0, a1, lo, hi) = (t[0] as u32, t[1] as u32, t[2] as u32, t[3] as u32);
                self.wr(a0, lo);
                self.wr(a1, hi);
            }
            K::ALU_T => {
                let (r0, r1) = self.alu_pair::<-1>(row);
                self.tmp[row[7] as usize * 4..][..2].copy_from_slice(&[r0, r1]);
            }
            K::COMMIT_ALU => {
                let (r0, r1) = (
                    self.tmp[row[7] as usize * 4],
                    self.tmp[row[7] as usize * 4 + 1],
                );
                self.commit_alu::<-1>(row, r0, r1);
            }
            K::COND_ALU => {
                // `if lt`: an element whose `AN & !AZ` flag is clear keeps its registers and flags
                let (r0, r1) = self.alu_pair::<-1>(row);
                let (rn, op) = (row[1] as usize & 31, row[2]);
                for (pe, res) in [r0, r1].into_iter().enumerate() {
                    if self.an[pe] && !self.az[pe] {
                        self.d[if pe == 0 { rn } else { twin(rn) }] = res;
                        if sets_flags(op) {
                            (self.an[pe], self.az[pe]) = alu_flags(op, res);
                        }
                    }
                }
            }
            K::SWAP => {
                let (a, b) = (self.get40(r(1)), self.get40(r(2)));
                self.d[r(1)] = b;
                self.d[r(2)] = a;
            }
            K::ALU => self.alu_item::<-1>(row),
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

    /// Words read by a load at `a`: (low, high); the register write follows in `load_commit`.
    #[inline(always)]
    fn load_words(&self, row: &Row, a: u32) -> (u32, u32) {
        let reg = row[1] as usize;
        if row[5] & 1 != 0 {
            (self.rd(a & !1), self.rd(a | 1))
        } else if reg < 32 {
            let step = if row[5] & 2 != 0 { 0 } else { 1 };
            (self.rd(a), self.rd(a.wrapping_add(step)))
        } else {
            (self.rd(a), 0)
        }
    }

    #[inline(always)]
    fn load_commit(&mut self, row: &Row, lo: u32, hi: u32) {
        let reg = row[1] as usize;
        if row[5] & 1 != 0 {
            let n = reg & !1;
            self.put32(n, lo);
            self.put32(n | 1, hi);
        } else if reg < 32 {
            self.d[reg] = u64::from(lo) << 8;
            self.d[twin(reg)] = u64::from(hi) << 8;
        } else {
            self.put32(reg, lo);
        }
    }

    /// Addresses and words of a store at `a`: (low address, high address, low word, high word).
    #[inline(always)]
    fn store_words(&self, row: &Row, a: u32) -> (u32, u32, u32, u32) {
        let reg = row[1] as usize;
        if row[5] & 1 != 0 {
            let n = reg & !1;
            (a & !1, a | 1, self.get32(n), self.get32(n | 1))
        } else {
            (a, a.wrapping_add(1), self.get32(reg), self.get32(twin(reg)))
        }
    }

    /// One ALU item; `OP` is the operation of a specialised op (one dispatch per item), -1 reads
    /// it from the row.
    #[inline(always)]
    fn alu_item<const OP: i32>(&mut self, row: &Row) {
        let (r0, r1) = self.alu_pair::<OP>(row);
        self.commit_alu::<OP>(row, r0, r1);
    }

    /// Writes the ALU results of both elements and their flags.
    #[inline(always)]
    fn commit_alu<const OP: i32>(&mut self, row: &Row, r0: u64, r1: u64) {
        let op = if OP < 0 { row[2] } else { OP };
        let rn = row[1] as usize & 31;
        let flags = sets_flags(op).then(|| [alu_flags(op, r0), alu_flags(op, r1)]);
        self.d[rn] = r0;
        self.d[twin(rn)] = r1;
        if let Some(fl) = flags {
            for (pe, (an, az)) in fl.into_iter().enumerate() {
                (self.an[pe], self.az[pe]) = (an, az);
            }
        }
    }

    /// ALU item on both elements. The float and the common integer operations are inlined with
    /// the match outside the per element loop; the rest goes through `Machine::alu`.
    #[inline(always)]
    fn alu_pair<const OP: i32>(&self, row: &Row) -> (u64, u64) {
        let op = if OP < 0 { row[2] } else { OP };
        let (x, y) = (row[3] as usize & 15, row[4] as usize & 15);
        let (x0, x1, y0, y1) = (self.d[x], self.d[16 + x], self.d[y], self.d[16 + y]);
        let w = |v: u64| (v >> 8) as u32;
        let int = |v: u32| u64::from(v) << 8;
        let (p, q) = (row[5], row[6]);
        match op {
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::compile::compile;
    use super::super::image::BassImage;
    use super::super::{real_image, synthetic_image, GEN_PRESET, PROC_PRESET};
    use super::*;

    fn chord_machine(image: &BassImage) -> Machine {
        let mut m = Machine::new();
        m.load_image(&image.csv);
        m.load_image(include_str!(
            "../../../../../docs/research/data/bass-fw-chord-delta.csv"
        ));
        m.set_programs(Some(&image.gen), &image.proc);
        m
    }

    /// `calls` generator + processor calls on the reference and the compiled executor: after
    /// every call the whole machine (registers, index registers, flags, accumulators, DM) is equal.
    fn assert_executors_agree(image: &BassImage, calls: usize) {
        let (gen, proc_) = (compile(&image.gen), compile(&image.proc));
        let (mut a, mut b) = (chord_machine(image), chord_machine(image));
        for call in 0..calls {
            a.call(&image.gen, GEN_PRESET);
            a.call(&image.proc, PROC_PRESET);
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

    #[test]
    fn compiled_chain_equals_the_reference_machine_state_for_state() {
        // 600 calls of the oracle chord; needs the user's OS update file (`NS4_WAVE_TABLES`),
        // skipped with a note without it
        let Some(image) = real_image() else { return };
        assert_executors_agree(&image, 600);
    }

    #[test]
    fn compiled_synthetic_routines_equal_the_reference_machine() {
        assert_executors_agree(&synthetic_image(), 3);
    }
}
