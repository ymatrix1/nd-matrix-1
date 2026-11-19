//! Data and register transfers of [`Core`]: immediates, DM/PM moves with the various addressing
//! forms, register-to-register moves, `Dreg <-> CDreg` swaps, bit operations and index modify.
use super::decode_cache::MoveOp;
use super::exec::Decoded;
use super::{Core, Fault};
use crate::visa::Kind;

impl Core {
    /// The values a store of `ureg` writes (PEx, and PEy for a data register), read before the
    /// instruction's compute runs: all halves of one instruction read the old registers.
    pub(super) fn capture(&self, ureg: u8) -> (u32, u32) {
        // VERIFIED (census diff): in SIMD mode a store of any other universal register also writes
        // its PEy complement to `addr + 1`; for ASTAT and STKY that is ASTATy/STKYy (ASSUMED, the
        // census only saw M0 stores), for the rest the same value (M0 stores).
        let y = match ureg {
            0..=15 => self.s[ureg as usize],
            0x50..=0x5F => self.r[usize::from(ureg & 15)],
            0x76 | 0x78 => self.read_ureg(ureg + 1),
            _ => self.read_ureg(ureg),
        };
        (self.read_ureg(ureg), y)
    }

    /// Moves universal register `ureg` to or from data memory at `addr`. In SIMD mode a data
    /// register transfer also moves the PEy register to or from `addr + 1` (ASSUMED from the
    /// duplicated word pairs of the image data and the SIMD description of the reference).
    pub(super) fn mem_transfer(
        &mut self,
        d: &Decoded,
        addr: u32,
        ureg: u8,
        write: bool,
    ) -> Result<(), Fault> {
        let captured = self.capture(ureg);
        self.mem_transfer_with(d, addr, ureg, write, captured)
    }

    /// [`Core::mem_transfer`] storing the values captured before a parallel compute.
    pub(super) fn mem_transfer_with(
        &mut self,
        d: &Decoded,
        addr: u32,
        ureg: u8,
        write: bool,
        captured: (u32, u32),
    ) -> Result<(), Fault> {
        let bad = |address| Fault::BadAddress { pc: d.pc, address };
        if Self::lw_flag(d.kind(), d.w) {
            // VERIFIED (census diff, scalar mode, 126 firmware forms): a long-word access moves the
            // register pair `Un, Un+1` (n even; an odd register selects its pair) to or from the two
            // words at `addr & !1`. ASSUMED in SIMD mode (the simulator also changes S0 there: open).
            // ASTATx/y and STKYx/y have no pair: VERIFIED (ADSP-21489 simulator, `census_extra_cases.txt`,
            // `STKYx = DM(a) (LW)` 6 cases and the 0x76..0x79 sweep): both words load the named
            // register in turn, so it ends with the word at `addr + 1` and its partner is untouched.
            let partner = if (0x76..=0x79).contains(&ureg) {
                ureg
            } else {
                ureg ^ 1
            };
            let (u0, u1) = (ureg, partner); // the named register takes the lower address
            let a = addr & !1;
            if write {
                let (v0, v1) = (self.read_ureg(u0), self.read_ureg(u1));
                self.dm_write(a, v0).map_err(bad)?;
                self.dm_write(a.wrapping_add(1), v1).map_err(bad)?;
            } else {
                let (v0, v1) = (
                    self.dm_read(a).map_err(bad)?,
                    self.dm_read(a.wrapping_add(1)).map_err(bad)?,
                );
                self.write_ureg(u0, v0);
                self.write_ureg(u1, v1);
            }
            return Ok(());
        }
        // VERIFIED (census diff): the condition of a conditional move is evaluated per element (the
        // word at `addr` follows PEx's condition, the one at `addr + 1` PEy's), and an S register
        // transfers like an R register with the roles swapped (`S4 = DM(a)` also loads R4 from
        // `a + 1`).
        let (x_ok, y_ok) = match d.kind() {
            Kind::T3a | Kind::T3b | Kind::T4a | Kind::T4b => (d.cond_ok, !self.simd() || d.cond_y),
            _ => (true, true),
        };
        let data = ureg < 16 || (0x50..0x60).contains(&ureg);
        let twin = if ureg < 16 { 0x50 | ureg } else { ureg & 15 };
        if write {
            if x_ok {
                self.dm_write(addr, captured.0).map_err(bad)?;
            }
            if self.simd() && y_ok {
                self.dm_write(addr.wrapping_add(1), captured.1)
                    .map_err(bad)?;
            }
        } else {
            // VERIFIED (ADSP-21489 simulator, `census_extra_cases.txt`, `IF cc M12 = DM(I0, M0)` over
            // 10 conditions x SIMD states): a register that is not a data register has no PEy twin,
            // so in SIMD mode the load happens when either element's condition holds
            if x_ok || (self.simd() && !data && y_ok) {
                let v = self.dm_read(addr).map_err(bad)?;
                self.write_ureg(ureg, v);
            }
            if self.simd() && data && y_ok {
                let v = self.dm_read(addr.wrapping_add(1)).map_err(bad)?;
                self.write_ureg(twin, v);
            }
        }
        Ok(())
    }

    /// Executes a pre-decoded move instruction (`MoveOp`), falling back to `exec_move` for `Other`.
    pub(super) fn exec_move_op(&mut self, d: &Decoded, op: MoveOp) -> Result<(), Fault> {
        match op {
            MoveOp::LoadImm { ureg, value } => {
                self.write_ureg(ureg, value);
                if self.simd() && (ureg < 16 || ureg >> 4 == 5) {
                    self.write_ureg(if ureg < 16 { 0x50 | ureg } else { ureg & 15 }, value);
                } else if self.simd() && (ureg == 0x76 || ureg == 0x78) {
                    self.write_ureg(ureg + 1, value);
                }
                Ok(())
            }
            MoveOp::DirectMem { addr, ureg, write } => self.mem_transfer(d, addr, ureg, write),
            MoveOp::RegSwap { r, s, .. } => {
                if d.cond_ok || self.simd() && d.cond_y {
                    if let Some(cop) = match op {
                        MoveOp::RegSwap { compute, .. } => compute,
                        _ => None,
                    } {
                        self.on_each_pe(d, d.cond_ok, d.cond_y, |core| core.exec_compute_op(cop))?;
                    }
                    self.swap_each_pe(d, r as usize, s as usize);
                }
                Ok(())
            }
            MoveOp::RegMove {
                src, dst, compute, ..
            } => {
                if d.cond_ok || self.simd() && d.cond_y {
                    let cap = self.capture_move(src);
                    if let Some(cop) = compute {
                        self.on_each_pe(d, d.cond_ok, d.cond_y, |core| core.exec_compute_op(cop))?;
                    }
                    self.apply_move_if(dst, src, &cap, d.cond_ok, !self.simd() || d.cond_y);
                }
                Ok(())
            }
            MoveOp::DagMem {
                ii,
                mi,
                ureg,
                write,
                post_modify,
                cond,
                compute,
            } => {
                if cond == 31 || d.cond_ok || self.simd() && d.cond_y {
                    let (ii, mi) = (ii as usize, mi as usize);
                    let (addr, update) = if post_modify {
                        (self.dag_out(ii, self.i[ii]), Some(self.modified(ii, mi)))
                    } else {
                        (self.dag_out(ii, self.i[ii].wrapping_add(self.m[mi])), None)
                    };
                    let captured = self.capture(ureg);
                    if let Some(cop) = compute {
                        self.on_each_pe(d, d.cond_ok, d.cond_y, |core| core.exec_compute_op(cop))?;
                    }
                    self.mem_transfer_with(d, addr, ureg, write, captured)?;
                    self.broadcast_load(ii, ureg, write);
                    if let Some(u) = update {
                        self.i[ii] = u;
                    }
                }
                Ok(())
            }
            MoveOp::Other => self.exec_move(d),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::support::core_at_zero;

    #[test]
    fn type_3c_loads_and_stores_with_post_modify() {
        // R0 = DM(I2, M2): 16-bit word 0x9?: DMI=2, DMM=2, D=0, bit36=1, DREG=0
        let p: u16 = (0b1001 << 12) | (2 << 9) | (2 << 6) | (1 << 4);
        let mut c = core_at_zero(&[p]);
        c.i[2] = 0x9_0010;
        c.m[2] = 3;
        c.dm_write(0x9_0010, 42).unwrap();
        assert!(c.step().is_ok());
        assert_eq!(c.r[0], 42);
        assert_eq!(c.i[2], 0x9_0013);
    }

    #[test]
    fn type_15b_loads_into_the_register_field_with_a_signed_offset() {
        // I8 = DM(0, I7): 9e08 0c00 (UREG 0x18 in bits 29..23, DATA7 = 0)
        let mut c = core_at_zero(&[0x9e08, 0x0c00]);
        c.i[7] = 0x9_0100;
        c.dm_write(0x9_0100, 0x12_538C).unwrap();
        assert!(c.step().is_ok());
        assert_eq!(
            c.i[8], 0x12_538C,
            "the module record's code pointer lands in I8"
        );
        // R0 = DM(-6, I1): 9208 007a (UREG 0, DATA7 = 0x7a = -6)
        let mut c = core_at_zero(&[0x9208, 0x007a]);
        c.i[1] = 0x9_0200;
        c.dm_write(0x9_0200 - 6, 0xFFFE).unwrap();
        assert!(c.step().is_ok());
        assert_eq!(c.r[0], 0xFFFE);
    }

    #[test]
    fn simd_transfers_move_the_pey_register_to_the_next_word() {
        // R4 = DM(0, I2) in SIMD mode: 3c 16-bit word, DMI=2 DMM=2 D=0
        let p: u16 = (0b1001 << 12) | (2 << 9) | (2 << 6) | (1 << 4) | 4;
        let mut c = core_at_zero(&[p]);
        c.mode1 = 1 << 21;
        c.i[2] = 0x9_0010;
        c.dm_write(0x9_0010, 11).unwrap();
        c.dm_write(0x9_0011, 22).unwrap();
        assert!(c.step().is_ok());
        assert_eq!((c.r[4], c.s[4]), (11, 22));
    }

    /// ADI: `9228 0a88` is `i5=pm(0x8,i9)`: G bit 37 selects PM and the DAG2 index.
    #[test]
    fn type_15b_program_memory_form_uses_the_dag2_index() {
        let mut c = core_at_zero(&[0x9228, 0x0a88]);
        (c.i[1], c.i[9]) = (0x9_0000, 0x9_0200);
        c.dm_write(0x9_0208, 0x1234).unwrap();
        assert!(c.step().is_ok());
        assert_eq!(c.i[5], 0x1234);
    }

    #[test]
    fn type_4_offset_is_signed() {
        // ADI: `6a3f 218d a00f` is `f0=float r0 by r15, r3=dm(0xffffffe4,i5)` (offset -28)
        let mut c = core_at_zero(&[0x6a3f, 0x218d, 0xa00f]);
        c.i[5] = 0x9_0100;
        c.r[0] = 3;
        c.dm_write(0x9_0100 - 28, 77).unwrap();
        assert!(c.step().is_ok());
        assert_eq!(c.r[3], 77);
        assert_eq!(f32::from_bits(c.r[0]), 3.0);
    }

    /// ADI: `6c7e 10bf` is `r1=dm(i6,0x2)`: post-modify, the access is at I6 and I6 moves by 2.
    #[test]
    fn type_4_post_modify_updates_the_index() {
        let mut c = core_at_zero(&[0x6c7e, 0x10bf]);
        c.i[6] = 0x9_0100;
        c.dm_write(0x9_0100, 5).unwrap();
        c.dm_write(0x9_0102, 6).unwrap();
        assert!(c.step().is_ok());
        assert_eq!((c.r[1], c.i[6]), (5, 0x9_0102));
        // the pre-modify offset form `dm(0x2,i6)` leaves the index alone: 6c3e = bit 38 clear
        let mut c = core_at_zero(&[0x6c3e, 0x10bf]);
        c.i[6] = 0x9_0100;
        c.dm_write(0x9_0102, 6).unwrap();
        assert!(c.step().is_ok());
        assert_eq!((c.r[1], c.i[6]), (6, 0x9_0100));
    }

    /// ADI: `043e e73f` is `i4=modify(i3,m4)`: the destination is the source XOR bits 26..24.
    #[test]
    fn simd_store_of_a_non_data_register_writes_addr_plus_one_too() {
        // census diff: `DM(addr) = M0` in SIMD mode wrote M0 at addr and addr + 1 on the simulator
        let mut c = crate::core::Core::new(&[]);
        c.mode1 = 1 << 21;
        c.m[0] = 2;
        let d = c.fetch(0);
        c.mem_transfer(&d, 0xB2300, 0x20, true).unwrap();
        assert_eq!((c.dm_read(0xB2300), c.dm_read(0xB2301)), (Ok(2), Ok(2)));
        let mut scalar = crate::core::Core::new(&[]);
        scalar.m[0] = 2;
        scalar.mem_transfer(&d, 0xB2300, 0x20, true).unwrap();
        assert_eq!(scalar.dm_read(0xB2301), Ok(0));
    }

    #[test]
    fn long_word_access_moves_a_register_pair_at_an_even_address() {
        // census diff: `R0 = DM(0xB2181) (LW)` is parcels 1080 000b 2181 in the firmware
        let mut c = crate::core::support::core_at_zero(&[0x1080, 0x000B, 0x2181]);
        c.dm_write(0xB2180, 0x1111).unwrap();
        c.dm_write(0xB2181, 0x2222).unwrap();
        c.r[1] = 9;
        assert_eq!(c.step(), Ok(()));
        assert_eq!(
            (c.r[0], c.r[1]),
            (0x1111, 0x2222),
            "address aligned down, R0 and R1"
        );
        // the same word without the LW bit loads R0 only
        let mut plain = crate::core::support::core_at_zero(&[0x1000, 0x000B, 0x2181]);
        plain.dm_write(0xB2181, 0x2222).unwrap();
        plain.r[1] = 9;
        assert_eq!(plain.step(), Ok(()));
        assert_eq!((plain.r[0], plain.r[1]), (0x2222, 9));
    }

    #[test]
    fn long_word_access_with_an_odd_register_gives_it_the_lower_address() {
        // census diff: `R3 = DM(I2, M0) (LW)` loads R3 from the lower word and R2 from the upper
        let mut c = crate::core::support::core_at_zero(&[0x443e, 0x41bf]);
        c.i[2] = 0xB2200;
        c.m[0] = 0;
        c.dm_write(0xB2200, 0x1111).unwrap();
        c.dm_write(0xB2201, 0x2222).unwrap();
        assert_eq!(c.step(), Ok(()));
        assert_eq!((c.r[3], c.r[2]), (0x1111, 0x2222));
    }

    #[test]
    fn simd_immediate_store_fills_the_next_word_too() {
        // census diff: `DM(I0, M1) = 0` (Type 16a, parcels 9040 0000 0000) in SIMD mode
        let mut c = crate::core::support::core_at_zero(&[0x9040, 0x0000, 0x0000]);
        c.mode1 = 1 << 21;
        c.i[0] = 0xB2400;
        c.m[1] = 1;
        c.dm_write(0xB2401, 0x55).unwrap();
        let _ = c.step();
        assert_eq!(
            c.dm_read(0xB2401),
            Ok(0),
            "the twin word is written as well"
        );
    }
}
