//! Instruction executor of the SHARC machine: one decoded instruction at a time.
//!
//! The rules are `tools/sharc_listing_vm.py` `Vm.step` / `tools/sharc_listing_ext.py` `GenVm`
//! (VERIFIED against `sharc-chain` over 4696 generator + processor calls, research s14): every item
//! of an instruction reads the old state, the writes land together at the end (memory, 40 bit data
//! registers, 32 bit registers, index registers), `if lt` keeps the registers of a PE whose
//! `AN & !AZ` flag is false, `MRF=` and `comp` take effect after the instruction.

use super::f40::{self, M32};
use super::tables::{alu_op as A, kind as K};
use super::{Machine, Row};

/// Writes collected during one instruction.
#[derive(Default)]
pub(super) struct Pending {
    mem: [(u32, u32); 6],
    nmem: usize,
    w40: [(u8, u64); 8],
    n40: usize,
    w32: [(u8, u32); 8],
    n32: usize,
    wi: [(u8, u32); 3],
    ni: usize,
    mr: Option<[i128; 2]>,
    flags: Option<[(bool, bool); 2]>,
}

impl Pending {
    fn reset(&mut self) {
        self.nmem = 0;
        self.n40 = 0;
        self.n32 = 0;
        self.ni = 0;
        self.mr = None;
        self.flags = None;
    }
    fn mem(&mut self, a: u32, v: u32) {
        self.mem[self.nmem] = (a, v);
        self.nmem += 1;
    }
    fn w40(&mut self, id: usize, v: u64) {
        self.w40[self.n40] = (id as u8, v);
        self.n40 += 1;
    }
    fn w32(&mut self, id: usize, v: u32) {
        self.w32[self.n32] = (id as u8, v);
        self.n32 += 1;
    }
}

/// ALU operations that set AN / AZ (float pass, add, sub, abs and the fixed add, sub, clip, and);
/// the multiplier and the shifter leave them alone. VERIFIED: `if ne jump` of the Farfisa
/// processor follows `pass` (research s16, 300 calls against `sharc-chain` in both branches).
pub(super) const fn sets_flags(op: i32) -> bool {
    matches!(
        op,
        A::PASS | A::FADD | A::FSUB | A::ABS | A::IADD | A::ISUB | A::CLIP | A::AND | A::ADDI
    )
}

/// `(AN, AZ)` of an ALU result `v` (40 bit word): float results are zero when the exponent is 0.
pub(super) fn alu_flags(op: i32, v: u64) -> (bool, bool) {
    let float = matches!(op, A::PASS | A::FADD | A::FSUB | A::ABS);
    let zero = if float {
        (v >> 31) & 0xFF == 0
    } else {
        (v >> 8) as u32 == 0
    };
    (v >> 39 == 1, zero)
}

pub(super) const fn twin(id: usize) -> usize {
    if id < 32 {
        id ^ 16
    } else {
        id
    }
}

impl Machine {
    /// Effective address of a decoded operand and the index register update (`ia`, new `I`).
    fn addr(&self, row: &Row, at: usize) -> (u32, Option<(usize, u32)>) {
        let (kind, p, q) = (row[at], row[at + 1], row[at + 2]);
        let step = |by_m: bool| if by_m { self.m[q as usize] as i32 } else { q };
        match kind {
            0 => (p as u32, None),
            1 | 2 => {
                let ia = p as usize;
                let new = self.wrap(ia, i64::from(self.i[ia]) + i64::from(step(kind == 1)));
                (self.i[ia], Some((ia, new)))
            }
            _ => {
                let ib = p as usize;
                (
                    self.wrap(ib, i64::from(self.i[ib]) + i64::from(step(kind == 3))),
                    None,
                )
            }
        }
    }

    pub(super) fn wrap(&self, idx: usize, a: i64) -> u32 {
        let (n, base) = (i64::from(self.l[idx]), i64::from(self.b[idx]));
        let a = if n == 0 {
            a
        } else if a >= base + n {
            a - n
        } else if a < base {
            a + n
        } else {
            a
        };
        a as u32
    }

    pub(super) fn get32(&self, id: usize) -> u32 {
        match id {
            0..=31 => (self.d[id] >> 8) as u32,
            32..=47 => self.i[id - 32],
            48..=63 => self.m[id - 48],
            64..=79 => self.l[id - 64],
            80..=95 => self.b[id - 80],
            _ => self.lcntr,
        }
    }

    pub(super) fn get40(&self, id: usize) -> u64 {
        if id < 32 {
            self.d[id]
        } else {
            u64::from(self.get32(id)) << 8
        }
    }

    pub(super) fn put32(&mut self, id: usize, v: u32) {
        match id {
            0..=31 => self.d[id] = u64::from(v) << 8,
            32..=47 => self.i[id - 32] = v,
            48..=63 => self.m[id - 48] = v,
            64..=79 => self.l[id - 64] = v,
            80..=95 => {
                self.b[id - 80] = v;
                self.i[id - 80] = v;
            }
            _ => self.lcntr = v,
        }
    }

    /// ALU result of `row` on element `pe` (0 = PEx on `r`, 1 = PEy on `s`).
    pub(super) fn alu(&self, row: &Row, pe: usize) -> u64 {
        let base = pe * 16;
        let reg = |n: i32| self.d[base + n as usize];
        let w = |n: i32| (reg(n) >> 8) as u32;
        let (x, y, p, q) = (row[3], row[4], row[5], row[6]);
        match row[2] {
            A::PASS => f40::pass(reg(x)),
            A::FMUL => f40::mul(reg(x), reg(y)),
            A::FADD => f40::add(reg(x), reg(y), false),
            A::FSUB => f40::add(reg(x), reg(y), true),
            A::IADD => u64::from(w(x).wrapping_add(w(y))) << 8,
            A::ISUB => u64::from(w(x).wrapping_sub(w(y))) << 8,
            A::FLOATBY => f40::float_by(w(x) as i32, w(y) as i32),
            A::ABS => f40::abs(reg(x)),
            A::FEXT_R | A::FDEP_R => {
                let ry = w(y);
                let (len, pos) = ((ry >> 6) & 63, ry & 63);
                let f = if row[2] == A::FEXT_R {
                    f40::fext
                } else {
                    f40::fdep
                };
                u64::from(f(w(x), len, pos, p != 0)) << 8
            }
            A::FEXT_I | A::FDEP_I => {
                let f = if row[2] == A::FEXT_I {
                    f40::fext
                } else {
                    f40::fdep
                };
                u64::from(f(w(x), p as u32, y as u32, q != 0)) << 8
            }
            A::ASHIFT | A::LSHIFT => {
                u64::from(f40::shift(w(x), w(y) as i32, row[2] == A::ASHIFT)) << 8
            }
            A::CLIP => {
                let lim = i64::from(w(y) as i32).abs();
                (u64::from((i64::from(w(x) as i32)).clamp(-lim, lim) as u32)) << 8
            }
            A::AND => u64::from(w(x) & w(y)) << 8,
            A::ADDI => u64::from(w(x).wrapping_add(y as u32)) << 8,
            A::MULT => u64::from(self.mult(pe, row, true).1) << 8,
            _ => 0,
        }
    }

    pub(super) fn mult(&self, pe: usize, row: &Row, with_acc: bool) -> (i128, u32) {
        let (x, y, mr_op, mode) = if with_acc {
            (row[3], row[4], row[5], row[6])
        } else {
            (row[1], row[2], 0, row[3])
        };
        let base = pe * 16;
        let w = |n: i32| (self.d[base + n as usize] >> 8) as u32;
        f40::mult(w(x), w(y), self.mr[pe], mr_op, mode)
    }

    /// Executes the item rows of one instruction.
    pub(super) fn exec(&mut self, items: &[Row], cond: bool, pd: &mut Pending) {
        let keep = cond.then_some(self.d);
        pd.reset();
        for row in items {
            self.item(row, pd);
        }
        for &(a, v) in &pd.mem[..pd.nmem] {
            self.wr(a, v);
        }
        for &(id, v) in &pd.w40[..pd.n40] {
            self.d[id as usize] = v & 0xFF_FFFF_FFFF;
        }
        for &(id, v) in &pd.w32[..pd.n32] {
            self.put32(id as usize, v);
        }
        for &(ia, v) in &pd.wi[..pd.ni] {
            self.i[ia as usize] = v;
        }
        let mut ran = [true; 2];
        if let Some(old) = keep {
            for (pe, r) in ran.iter_mut().enumerate() {
                *r = self.an[pe] && !self.az[pe];
                if !*r {
                    self.d[pe * 16..pe * 16 + 16].copy_from_slice(&old[pe * 16..pe * 16 + 16]);
                }
            }
        }
        if let Some(mr) = pd.mr {
            self.mr = mr;
        }
        if let Some(fl) = pd.flags {
            for (pe, (an, az)) in fl.into_iter().enumerate() {
                if ran[pe] {
                    (self.an[pe], self.az[pe]) = (an, az);
                }
            }
        }
    }

    fn item(&self, row: &Row, pd: &mut Pending) {
        let r = |k: usize| row[k] as usize;
        match row[0] {
            K::SWAP => {
                let (a, b) = (r(1), r(2));
                pd.w40(a, self.get40(b));
                pd.w40(b, self.get40(a));
            }
            K::STORE => {
                let (a, upd) = self.addr(row, 2);
                if let Some(u) = upd {
                    pd.wi[pd.ni] = (u.0 as u8, u.1);
                    pd.ni += 1;
                }
                let reg = r(1);
                if row[5] & 1 != 0 {
                    let n = reg & !1;
                    pd.mem(a & !1, self.get32(n));
                    pd.mem(a | 1, self.get32(n | 1));
                } else {
                    pd.mem(a, self.get32(reg));
                    pd.mem(a.wrapping_add(1), self.get32(twin(reg)));
                }
            }
            K::LOAD => {
                let (a, upd) = self.addr(row, 2);
                if let Some(u) = upd {
                    pd.wi[pd.ni] = (u.0 as u8, u.1);
                    pd.ni += 1;
                }
                let reg = r(1);
                if row[5] & 1 != 0 {
                    let n = reg & !1;
                    pd.w32(n, self.rd(a & !1));
                    pd.w32(n | 1, self.rd(a | 1));
                } else if reg < 32 {
                    let step = if row[5] & 2 != 0 { 0 } else { 1 };
                    pd.w32(reg, self.rd(a));
                    pd.w32(twin(reg), self.rd(a.wrapping_add(step)));
                } else {
                    pd.w32(reg, self.rd(a));
                }
            }
            K::MODIFY => {
                let ia = r(2);
                let step = if row[3] != 0 {
                    self.m[r(4)] as i32
                } else {
                    row[4]
                };
                pd.w32(r(1), self.wrap(ia, i64::from(self.i[ia]) + i64::from(step)));
            }
            K::MOVE_N => pd.w32(r(1), self.get32(r(2))),
            K::MOVE_NI => pd.w32(r(1), row[2] as u32),
            K::MOVE_D => {
                pd.w40(r(1), self.get40(r(2)));
                pd.w40(twin(r(1)), self.get40(twin(r(2))));
            }
            K::MOVE_DN | K::MOVE_DI => {
                let v = if row[0] == K::MOVE_DN {
                    self.get32(r(2))
                } else {
                    row[2] as u32
                };
                pd.w40(r(1), u64::from(v) << 8);
                pd.w40(twin(r(1)), u64::from(v) << 8);
            }
            K::ALU => {
                let mut fl = [(false, false); 2];
                for (pe, slot) in fl.iter_mut().enumerate() {
                    let id = if pe == 0 { r(1) } else { twin(r(1)) };
                    let v = self.alu(row, pe);
                    pd.w40(id, v);
                    *slot = alu_flags(row[2], v);
                }
                if sets_flags(row[2]) {
                    pd.flags = Some(fl);
                }
            }
            K::MRF_SET => {
                let mut mr = [0i128; 2];
                for (pe, slot) in mr.iter_mut().enumerate() {
                    *slot = self.mult(pe, row, false).0;
                }
                pd.mr = Some(mr);
            }
            _ => {
                let mut fl = [(false, false); 2];
                for (pe, slot) in fl.iter_mut().enumerate() {
                    let w = |n: usize| (self.d[pe * 16 + n] >> 8) as u32 as i32;
                    *slot = (w(r(1)) < w(r(2)), w(r(1)) == w(r(2)));
                }
                pd.flags = Some(fl);
            }
        }
    }
}

const _: () = assert!(M32 == 0xFFFF_FFFF);

#[cfg(test)]
mod tests {
    use super::*;

    fn ins(items: &[Row], cond: bool) -> Vec<Row> {
        let mut v = vec![[K::INS, items.len() as i32, i32::from(cond), 0, 0, 0, 0, 0]];
        v.extend_from_slice(items);
        v
    }

    #[test]
    fn post_modify_load_moves_the_pair_and_updates_the_index() {
        let mut m = Machine::new();
        m.wr(0xB_3000, 0x11);
        m.wr(0xB_3001, 0x22);
        m.i[0] = 0xB_3000;
        // r1 = dm(i0, 2): PEx r1 <- word, PEy s1 <- next word; i0 += 2 (kind 2: post immediate)
        m.run(&ins(&[[K::LOAD, 1, 2, 0, 2, 0, 0, 0]], false));
        assert_eq!((m.d[1] >> 8, m.d[17] >> 8), (0x11, 0x22));
        assert_eq!(m.i[0], 0xB_3002);
    }

    #[test]
    fn broadcast_load_gives_both_elements_the_same_word() {
        let mut m = Machine::new();
        m.wr(0xB_3010, 0x33);
        m.wr(0xB_3011, 0x44);
        m.i[1] = 0xB_3010;
        // flag bit 1 = broadcast (loads through I1)
        m.run(&ins(&[[K::LOAD, 2, 4, 1, 0, 2, 0, 0]], false));
        assert_eq!((m.d[2] >> 8, m.d[18] >> 8), (0x33, 0x33));
    }

    #[test]
    fn long_word_store_writes_the_register_pair() {
        let mut m = Machine::new();
        m.i[12] = 0xAAAA;
        m.i[13] = 0xBBBB;
        // dm(0xB32D6) = i12 (LW): words `a & !1` and `a | 1`
        m.run(&ins(&[[K::STORE, 32 + 12, 0, 0xB_32D6, 0, 1, 0, 0]], false));
        assert_eq!((m.rd(0xB_32D6), m.rd(0xB_32D7)), (0xAAAA, 0xBBBB));
    }

    #[test]
    fn conditional_instruction_keeps_the_register_file_of_a_false_pe() {
        let mut m = Machine::new();
        m.d[5] = 7 << 8;
        m.d[21] = 7 << 8;
        m.an = [true, false];
        m.az = [false, false];
        // `if lt r5 = 99`: PEx (AN set) executes, PEy keeps its old register file
        m.run(&ins(&[[K::MOVE_DI, 5, 99, 0, 0, 0, 0, 0]], true));
        assert_eq!((m.d[5] >> 8, m.d[21] >> 8), (99, 7));
    }

    #[test]
    fn comp_sets_the_flags_after_the_instruction() {
        let mut m = Machine::new();
        m.d[1] = 3 << 8;
        m.d[2] = 5 << 8;
        m.run(&ins(&[[K::COMP, 1, 2, 0, 0, 0, 0, 0]], false));
        assert_eq!((m.an[0], m.az[0]), (true, false));
    }

    #[test]
    fn circular_buffer_wraps_the_post_modified_index() {
        let mut m = Machine::new();
        m.b[6] = 0xB_3100;
        m.l[6] = 4;
        m.i[6] = 0xB_3103;
        // i6 = modify(i6, 2) wraps inside the 4 word buffer
        m.run(&ins(&[[K::MODIFY, 32 + 6, 6, 0, 2, 0, 0, 0]], false));
        assert_eq!(m.i[6], 0xB_3101);
    }
}
