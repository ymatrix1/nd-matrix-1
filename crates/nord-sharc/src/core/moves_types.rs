//! Move and modify instruction types of [`Core`] (`exec_move`): immediates, register-to-register
//! moves, `Dreg <-> CDreg` swaps, index modify and bit operations. The memory transfer helpers and
//! the decoded-op dispatcher are in `moves.rs`.
use super::exec::Decoded;
use super::{sign_extend, Core, Fault};
use crate::visa::{bits, ureg_load, Kind};

impl Core {
    /// Executes the move and modify types: 3a/3b/3c, 4a/4b, 5a/5b, 7a/7b, 14a, 15a/15b, 16a,
    /// 17a/17b and 18a. Types with a compute field run it before the memory access.
    pub(super) fn exec_move(&mut self, d: &Decoded) -> Result<(), Fault> {
        let (w, kind) = (d.w, d.kind());
        let with_compute = matches!(kind, Kind::T3a | Kind::T4a | Kind::T5a | Kind::T7a);
        let write_bit = |bit| bits(w, bit, bit) == 1;
        match kind {
            Kind::T17a | Kind::T17b => {
                let (code, imm) = ureg_load(&d.insn).ok_or_else(|| self.fault(d.pc, &d.insn))?;
                let value = if kind == Kind::T17b {
                    sign_extend(imm, 16)
                } else {
                    imm
                };
                self.write_ureg(code, value);
                // broadcast to the twin register of the other element (VERIFIED, census diff:
                // `S0 = 0xc0f7c` in SIMD mode also sets R0)
                if self.simd() && (code < 16 || code >> 4 == 5) {
                    self.write_ureg(if code < 16 { 0x50 | code } else { code & 15 }, value);
                } else if self.simd() && (code == 0x76 || code == 0x78) {
                    self.write_ureg(code + 1, value); // VERIFIED: `STKYx = 0x...` also sets STKYy
                }
            }
            Kind::T14a => {
                let (addr, ureg) = (bits(w, 31, 0) as u32, bits(w, 38, 32) as u8);
                self.mem_transfer(d, addr, ureg, write_bit(40))?;
            }
            Kind::T15a => {
                let ii = bits(w, 43, 41) as usize + Self::dag_bank(bits(w, 44, 44));
                let addr = self.dag_out(ii, self.i[ii].wrapping_add(bits(w, 31, 0) as u32));
                let ureg = bits(w, 38, 32) as u8;
                self.mem_transfer(d, addr, ureg, write_bit(40))?;
                self.broadcast_load(ii, ureg, write_bit(40));
            }
            Kind::T15b => {
                // UREG 29..23, signed DATA7 22..16 (VERIFIED, docs 5.28)
                let offset = sign_extend(bits(w, 22, 16) as u32, 7);
                // G bit 37 selects program memory with the DAG2 index (ADI `i5=pm(0x8,i9)`)
                let ii = bits(w, 43, 41) as usize + Self::dag_bank(bits(w, 37, 37));
                let addr = self.dag_out(ii, self.i[ii].wrapping_add(offset));
                let ureg = bits(w, 29, 23) as u8;
                self.mem_transfer(d, addr, ureg, write_bit(40))?;
                self.broadcast_load(ii, ureg, write_bit(40));
            }
            Kind::T16a => {
                // DM/PM(Ii, Mm) = <data32>, post-modify with update
                let bank = Self::dag_bank(bits(w, 37, 37));
                let ii = bits(w, 43, 41) as usize + bank;
                let mi = bits(w, 40, 38) as usize + bank;
                let addr = self.dag_out(ii, self.i[ii]);
                let next_i = self.modified(ii, mi);
                let bad = |address| Fault::BadAddress { pc: d.pc, address };
                self.dm_write(addr, bits(w, 31, 0) as u32).map_err(bad)?;
                if self.simd() {
                    // VERIFIED (census diff): an immediate store also fills the next word
                    self.dm_write(addr.wrapping_add(1), bits(w, 31, 0) as u32)
                        .map_err(bad)?;
                }
                self.i[ii] = next_i;
            }
            Kind::T16b => {
                // DM/PM(Ii, Mm) = <data16> (sign extended like Type 17b), post-modify
                let bank = Self::dag_bank(bits(w, 37, 37));
                let ii = bits(w, 43, 41) as usize + bank;
                let mi = bits(w, 40, 38) as usize + bank;
                let addr = self.dag_out(ii, self.i[ii]);
                let next_i = self.modified(ii, mi);
                let value = sign_extend(bits(w, 31, 16) as u32, 16);
                let bad = |address| Fault::BadAddress { pc: d.pc, address };
                self.dm_write(addr, value).map_err(bad)?;
                if self.simd() {
                    self.dm_write(addr.wrapping_add(1), value).map_err(bad)?; // see Type 16a
                }
                self.i[ii] = next_i;
            }
            Kind::T18a => self.bit_op(d)?,
            Kind::T5a | Kind::T5b if d.cond_ok || self.simd() && d.cond_y => {
                if bits(w, 43, 43) == 1 {
                    // `Rx <-> Sy` [, compute]: Rx bits 26..23, Sy bits 41..38 (VERIFIED against
                    // ADI elfdump on 1,087 swaps, docs 5.34). In SIMD mode the swap is the same
                    // exchange (the PEy file is the S file).
                    let (s, r) = (bits(w, 41, 38) as usize, bits(w, 26, 23) as usize);
                    self.run_move_compute(d, with_compute)?;
                    self.swap_each_pe(d, r, s);
                } else {
                    let src = ((bits(w, 42, 38) << 2) | bits(w, 32, 31)) as u8;
                    // the compute and the transfer both read the registers as they were before
                    // the instruction (VERIFIED, census diff: `F12 = F8 * F7, R8 = R0`)
                    let dst = bits(w, 29, 23) as u8;
                    let cap = self.capture_move(src);
                    self.run_move_compute(d, with_compute)?;
                    self.apply_move_if(dst, src, &cap, d.cond_ok, !self.simd() || d.cond_y);
                }
            }
            Kind::T3a | Kind::T3b if d.cond_ok || self.simd() && d.cond_y => {
                let bank = Self::dag_bank(bits(w, 32, 32));
                let ii = bits(w, 43, 41) as usize + bank;
                let mi = bits(w, 40, 38) as usize + bank;
                // post-modify: access at I and update it (circular wrap applies); pre-modify:
                // access at I + M, no write-back and **no wrap** (the immediate form already
                // behaves so; the FX reverb core `0x123F61` relies on it: it stores a pair at
                // `I6` and at `I6 + L6` as two copies of its all-pass buffer, which only works
                // if the pre-modified address is not folded back by `L6`)
                let (addr, update) = if bits(w, 44, 44) == 1 {
                    (self.dag_out(ii, self.i[ii]), Some(self.modified(ii, mi)))
                } else {
                    (self.dag_out(ii, self.i[ii].wrapping_add(self.m[mi])), None)
                };
                let ureg = bits(w, 29, 23) as u8;
                let captured = self.capture(ureg);
                self.run_move_compute(d, with_compute)?;
                self.mem_transfer_with(d, addr, ureg, write_bit(31), captured)?;
                self.broadcast_load(ii, ureg, write_bit(31));
                if let Some(u) = update {
                    self.i[ii] = u;
                }
            }
            Kind::T3c => {
                // DM(Ii, Mm) <-> Rn, post-modify, DAG1 (bits 43..41 I, 40..38 M, 37 D, 35..32 Rn)
                let (ii, mi) = (bits(w, 43, 41) as usize, bits(w, 40, 38) as usize);
                let addr = self.dag_out(ii, self.i[ii]);
                let next_i = self.modified(ii, mi);
                self.mem_transfer(d, addr, bits(w, 35, 32) as u8, write_bit(37))?;
                self.broadcast_load(ii, bits(w, 35, 32) as u8, write_bit(37));
                self.i[ii] = next_i;
            }
            Kind::T7a | Kind::T7b if d.cond_ok => {
                // [IF cond] [compute,] [Id =] MODIFY(Is, M): G bit 38 selects DAG2, the
                // destination is Is XOR bits 26..24 (stays in the bank), the modify always
                // honours L/B (circular). VERIFIED against ADI elfdump on 51 samples (docs 5.34).
                let bank = Self::dag_bank(bits(w, 38, 38));
                let src = bits(w, 32, 30) as usize + bank;
                let mi = bits(w, 29, 27) as usize + bank;
                let dst = src ^ bits(w, 26, 24) as usize;
                self.run_move_compute(d, with_compute)?;
                self.i[dst] = self.modified_always(src, self.m[mi]);
            }
            Kind::T4a | Kind::T4b if d.cond_ok || self.simd() && d.cond_y => {
                let ii = bits(w, 43, 41) as usize + Self::dag_bank(bits(w, 40, 40));
                // DATA6 is a signed offset; the U bit (38) selects post-modify with update
                // (`dm(i6,2)`: access at I, then I += DATA6) instead of the pre-modify offset
                // `dm(2,i6)` (VERIFIED against ADI elfdump, 220 post and 1,576 pre samples, docs 5.34)
                let data = sign_extend(bits(w, 32, 27) as u32, 6);
                let (addr, update) = if bits(w, 38, 38) == 1 {
                    (
                        self.dag_out(ii, self.i[ii]),
                        Some(self.modified_by(ii, data)),
                    )
                } else {
                    (self.dag_out(ii, self.i[ii].wrapping_add(data)), None)
                };
                let ureg = bits(w, 26, 23) as u8;
                let captured = self.capture(ureg);
                self.run_move_compute(d, with_compute)?;
                self.mem_transfer_with(d, addr, ureg, write_bit(39), captured)?;
                self.broadcast_load(ii, ureg, write_bit(39));
                if let Some(u) = update {
                    self.i[ii] = u;
                }
            }
            // a conditional form whose condition failed does nothing
            Kind::T5a
            | Kind::T5b
            | Kind::T3a
            | Kind::T3b
            | Kind::T7a
            | Kind::T7b
            | Kind::T4a
            | Kind::T4b => {}
            _ => return Err(self.fault(d.pc, &d.insn)),
        }
        Ok(())
    }

    /// Runs the compute field in bits 22..0 when the instruction type has one.
    #[inline]
    fn run_move_compute(&mut self, d: &Decoded, present: bool) -> Result<(), Fault> {
        if present {
            self.run_compute(d, bits(d.w, 22, 0))
        } else {
            Ok(())
        }
    }

    /// Type 18a: `BIT SET|CLR|TGL|TST|XOR sreg, <data32>`.
    fn bit_op(&mut self, d: &Decoded) -> Result<(), Fault> {
        let sreg = (0x70 | bits(d.w, 35, 32)) as u8;
        let data = bits(d.w, 31, 0) as u32;
        let cur = self.read_ureg(sreg);
        match bits(d.w, 39, 37) {
            0 => self.write_ureg(sreg, cur | data),
            1 => self.write_ureg(sreg, cur & !data),
            2 => self.write_ureg(sreg, cur ^ data),
            4 => self.flags.tf = cur & data == data,
            5 => self.flags.tf = cur == data,
            _ => return Err(self.fault(d.pc, &d.insn)),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::support::{core_at_zero, core_with};

    #[test]
    fn immediates_registers_and_bit_ops() {
        // I0 = 0xc0430; M1 = 1; BIT SET MODE1 0x4f8; BIT CLR MODE1 0x8; NOP
        let mut c = core_with(&[
            0x0F10, 0x000C, 0x0430, 0x0FA1, 0x0001, 0x1402, 0x0000, 0x04F8, 0x1422, 0x0000, 0x0008,
            0x0001,
        ]);
        assert_eq!(c.run(5), None);
        assert_eq!(c.mode1, 0x4F0);
        // BIT SET MODE1 0x4F8 selected the secondary sets (banks.rs): the registers written
        // before it are in the hidden primary set
        assert_eq!((c.i[0], c.m[1]), (0, 0));
        c.write_ureg(0x72, 0);
        assert_eq!((c.i[0], c.m[1]), (0xC_0430, 1));
        assert_eq!(c.steps, 5);
    }

    #[test]
    fn type_7b_modifies_the_index_register_by_the_modify_register() {
        // IF TRUE MODIFY(I1, M3): VISA word 043e 583f at short-word 0
        let mut c = core_at_zero(&[0x043e, 0x583f]);
        c.i[1] = 100;
        c.m[3] = 5;
        assert!(c.step().is_ok());
        assert_eq!(c.i[1], 105);
        assert_eq!(c.pc, 2);
    }

    #[test]
    fn modify_destination_is_source_xor_the_field() {
        let mut c = core_at_zero(&[0x043e, 0xe73f]);
        (c.i[3], c.m[4]) = (100, 7);
        assert!(c.step().is_ok());
        assert_eq!((c.i[4], c.i[3]), (107, 100));
    }

    #[test]
    fn type_5a_swaps_a_data_register_with_its_pey_register() {
        // ADI: `7a3e 0480 1001` is `r0=r0+r1, r9<->s8`
        let mut c = core_at_zero(&[0x7a3e, 0x0480, 0x1001]);
        (c.r[9], c.s[8], c.r[0], c.r[1]) = (7, 9, 1, 2);
        assert!(c.step().is_ok());
        assert_eq!((c.r[9], c.s[8], c.r[0]), (9, 7, 3));
        assert_eq!(c.pc, 3);
        // `703e 933f` is `m6=r1`, `753e 93bf` is `m7=s1`
        let mut c = core_at_zero(&[0x703e, 0x933f, 0x753e, 0x93bf]);
        (c.r[1], c.s[1]) = (11, 22);
        assert_eq!(c.run(2), None);
        assert_eq!((c.m[6], c.m[7]), (11, 22));
    }

    #[test]
    fn type_19a_modifies_an_index_by_an_immediate_with_circular_wrap() {
        // ADI (SY1SY2 image): `1603 0000 0310` is `i3 = modify(i3, 0x310)`
        let mut c = core_at_zero(&[0x1603, 0x0000, 0x0310]);
        c.i[3] = 0x1000;
        assert!(c.step().is_ok());
        assert_eq!((c.i[3], c.pc), (0x1310, 3));
        // circular buffer base 0x100 length 4: 0x103 + 1 wraps to 0x100
        let mut c = core_at_zero(&[0x1603, 0x0000, 0x0001]);
        (c.i[3], c.l[3], c.b[3]) = (0x103, 4, 0x100);
        assert!(c.step().is_ok());
        assert_eq!(c.i[3], 0x100);
    }

    #[test]
    fn flush_cache_is_a_no_op() {
        // ADI: `1702 0000 0000` is `flush cache`
        let mut c = core_at_zero(&[0x1702, 0x0000, 0x0000]);
        assert!(c.step().is_ok());
        assert_eq!(c.pc, 3);
    }

    #[test]
    fn simd_immediate_load_into_s_also_sets_the_r_twin() {
        // census diff: `S0 = 0xc0f7c` (Type 17a)
        let mut c = crate::core::support::core_at_zero(&[0x0f50, 0x000c, 0x0f7c]);
        c.mode1 = 1 << 21;
        assert_eq!(c.step(), Ok(()));
        assert_eq!((c.s[0], c.r[0]), (0xC0F7C, 0xC0F7C));
    }
}
