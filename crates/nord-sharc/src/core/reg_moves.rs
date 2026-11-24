//! Register-to-register transfers of [`Core`]: the 40-bit extension that travels with a float
//! register move, the capture / apply split that lets a compute in the same instruction read the
//! old registers, conditional moves and the `Dreg <-> CDreg` swaps.
use super::Core;

/// The captured source of a register transfer (see [`Core::capture_move`]).
pub(super) struct MoveSrc {
    pub(super) v: u32,
    pub(super) ext: Option<(u32, u8)>,
    comp: u32,
    comp_ext: Option<(u32, u8)>,
}

impl Core {
    /// Extension of the data register (R: code 0..15, S: code 0x50..0x5F) `code`, if it is one.
    fn ext_slot(&self, code: u8) -> Option<(u32, u8)> {
        let n = usize::from(code & 15);
        match code >> 4 {
            0 => Some((self.fx.tag[n], self.fx.lo[n])),
            5 => Some((self.fx_y.tag[n], self.fx_y.lo[n])),
            _ => None,
        }
    }

    fn put_ext_slot(&mut self, code: u8, slot: (u32, u8)) {
        let n = usize::from(code & 15);
        match code >> 4 {
            0 => (self.fx.tag[n], self.fx.lo[n]) = slot,
            5 => (self.fx_y.tag[n], self.fx_y.lo[n]) = slot,
            _ => {}
        }
    }

    /// `Ureg = Ureg` transfer. VERIFIED on the simulator (`docs/research/sharc-round/`,
    /// `sharc-flags/`): a move between data registers, R or S, keeps the 40-bit extension; any other
    /// destination drops it.
    #[cfg(test)]
    pub(super) fn move_ureg(&mut self, dst: u8, src: u8) {
        let cap = self.capture_move(src);
        self.apply_move(dst, src, &cap);
    }

    /// Reads the source of a register transfer: the value, its extension, and the value of the
    /// PEy complement (read together because a compute in the same instruction sees the old
    /// registers).
    pub(super) fn capture_move(&self, src: u8) -> MoveSrc {
        let n = usize::from(src & 15);
        let (comp, comp_ext) = match src >> 4 {
            0 => (self.s[n], self.ext_slot(0x50 | src)),
            5 => (self.r[n], self.ext_slot(src & 15)),
            // ASSUMED for STKY (ASTAT measured): the complement is STKYy
            7 if src == 0x76 || src == 0x78 => (self.read_ureg(src + 1), None),
            _ => (self.read_ureg(src), None),
        };
        MoveSrc {
            v: self.read_ureg(src),
            ext: self.ext_slot(src),
            comp,
            comp_ext,
        }
    }

    /// Writes a captured transfer to `dst`. In SIMD mode the PEy side moves too:
    ///
    /// VERIFIED (`simd_measured.rs`, census diff): `Rn = Rm` also copies `Sm` to `Sn`, `Sn = Rm`
    /// also copies `Sm` to `Rn` (every data register has an opposite-element twin), `Rn = ASTAT`
    /// loads `Sn` with ASTATy and `ASTAT = Rn` loads ASTATy from `Sn`. ASSUMED: STKY likewise
    /// pairs with STKYy, and a source that is no data register is broadcast (`Sn = Rn`, as for the
    /// immediate loads).
    #[cfg(test)]
    pub(super) fn apply_move(&mut self, dst: u8, src: u8, cap: &MoveSrc) {
        self.apply_move_if(dst, src, cap, true, true);
    }

    /// [`Core::apply_move`] with a condition per element (VERIFIED, census diff: in SIMD mode
    /// `IF cc Sn = Rm` follows PEy's condition and `IF cc Rn = Rm` PEx's).
    pub(super) fn apply_move_if(
        &mut self,
        dst: u8,
        src: u8,
        cap: &MoveSrc,
        x_ok: bool,
        y_ok: bool,
    ) {
        if x_ok {
            self.write_ureg(dst, cap.v);
            if let (Some(e), true) = (cap.ext, self.ext_slot(dst).is_some()) {
                self.put_ext_slot(dst, e);
            }
        }
        if !self.simd() || !y_ok {
            return;
        }
        let twin = match dst >> 4 {
            0 => Some(0x50 | dst),
            5 => Some(dst & 15),
            7 if (dst == 0x76 || dst == 0x78) && src < 0x60 && src & 0x80 == 0 => Some(dst + 1),
            _ => None,
        };
        if let Some(t) = twin {
            if t >> 4 == 7 && src >> 4 != 0 && src >> 4 != 5 {
                return; // only a data register carries a PEy value into ASTATy/STKYy
            }
            self.write_ureg(t, cap.comp);
            if let (Some(e), true) = (cap.comp_ext, self.ext_slot(t).is_some()) {
                self.put_ext_slot(t, e);
            }
        }
    }

    /// `Rx <-> Sy`: swaps the registers with their extensions (VERIFIED: the extension survives).
    #[cfg(test)]
    pub(super) fn swap_rs(&mut self, r: usize, s: usize) {
        let (er, es) = (self.ext_slot(r as u8), self.ext_slot(0x50 | s as u8));
        std::mem::swap(&mut self.r[r], &mut self.s[s]);
        if let (Some(er), Some(es)) = (er, es) {
            self.put_ext_slot(r as u8, es);
            self.put_ext_slot(0x50 | s as u8, er);
        }
    }

    /// [`Core::swap_each_pe`] with both conditions true (unconditional `Rr <-> Ss`).
    pub(super) fn swap_regs(&mut self, r: usize, s: usize) {
        let (er, es) = (self.ext_slot(r as u8), self.ext_slot(0x50 | s as u8));
        let (rv, sv) = (self.r[r], self.s[s]);
        self.r[r] = sv;
        if let (Some(es), Some(_)) = (es, er) {
            self.put_ext_slot(r as u8, es);
        }
        self.s[s] = rv;
        if let (Some(er), Some(_)) = (er, es) {
            self.put_ext_slot(0x50 | s as u8, er);
        }
    }

    /// `Rx <-> Sy` under the instruction's condition. Both halves read the old values; the write
    /// to `Rx` follows PEx's condition and the write to `Sy` follows PEy's (in scalar mode only
    /// PEx's exists, so the swap is whole or skipped). There is no PEy-side duplicate swap.
    /// VERIFIED: ADSP-21489 simulator, `census_extra_cases.txt` (`IF cc R14 <-> S3` triple
    /// operation, all 32 conditions, 6 states incl. SIMD), census `IF EQ R2 <-> S0`.
    pub(super) fn swap_each_pe(&mut self, d: &super::exec::Decoded, r: usize, s: usize) {
        let (er, es) = (self.ext_slot(r as u8), self.ext_slot(0x50 | s as u8));
        let (rv, sv) = (self.r[r], self.s[s]);
        if d.cond_ok {
            self.r[r] = sv;
            if let (Some(es), Some(_)) = (es, er) {
                self.put_ext_slot(r as u8, es);
            }
        }
        if (self.simd() && d.cond_y) || (!self.simd() && d.cond_ok) {
            self.s[s] = rv;
            if let (Some(er), Some(_)) = (er, es) {
                self.put_ext_slot(0x50 | s as u8, er);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(op: u64, n: u64, x: u64, y: u64) -> u64 {
        (op << 12) | (n << 8) | (x << 4) | y
    }

    fn setf(c: &mut Core, n: usize, v: f32) {
        c.r[n] = v.to_bits();
    }

    #[test]
    fn swaps_and_moves_through_the_s_file_keep_the_extension() {
        // VERIFIED rows: docs/research/sharc-flags/flags_measured.csv, kind `swap`
        const CSV: &str = include_str!("../../../../docs/research/sharc-flags/flags_measured.csv");
        for line in CSV.lines().filter(|l| l.starts_with("swap")) {
            let f: Vec<&str> = line.split(',').collect();
            let hex = |s: &str| u32::from_str_radix(s, 16).unwrap_or(0);
            let mut c = Core::new(&[]);
            c.mode1 = hex(f[1]);
            setf(&mut c, 1, 1.0);
            setf(&mut c, 2, f32::from_bits(0x3080_0000)); // 2^-30
            assert!(c.compute(word(0x81, 5, 1, 2))); // F5 = F1 + F2
            c.swap_rs(5, 2);
            c.swap_rs(5, 2);
            assert!(c.compute(word(0x82, 3, 5, 1)));
            let a = c.r[3];
            c.move_ureg(0x53, 5); // S3 = R5
            c.swap_rs(6, 3); // R6 <-> S3
            assert!(c.compute(word(0x82, 3, 6, 1)));
            let b = c.r[3];
            c.move_ureg(0x53, 5);
            c.move_ureg(9, 0x53); // R9 = S3
            assert!(c.compute(word(0x82, 3, 9, 1)));
            assert_eq!([a, b, c.r[3]], [hex(f[5]), hex(f[6]), hex(f[7])], "{line}");
        }
    }

    #[test]
    fn register_move_keeps_the_extension() {
        let mut c = Core::new(&[]);
        setf(&mut c, 1, 1.0);
        setf(&mut c, 2, f32::from_bits(0x3380_0000));
        assert!(c.compute(word(0x81, 3, 1, 2)));
        c.move_ureg(7, 3); // R7 = R3
        assert_eq!(c.f40(7), c.f40(3));
        assert_eq!(c.f40(7) & 0xFF, 0x80);
        c.move_ureg(0x10, 3); // I0 = R3 is not a data register
        c.move_ureg(7, 0x10); // R7 = I0 has no extension
        assert_eq!(c.f40(7) & 0xFF, 0);
    }

    #[test]
    fn simd_register_moves_carry_the_pey_complement() {
        let mut c = Core::new(&[]);
        c.mode1 = 1 << 21;
        (c.r[1], c.s[1]) = (11, 22);
        c.move_ureg(9, 1); // R9 = R1 also copies S1 to S9
        assert_eq!((c.r[9], c.s[9]), (11, 22));
        c.set_astat(0, 0x400);
        c.set_astat(1, 0x404);
        c.move_ureg(7, 0x76); // R7 = ASTAT: S7 = ASTATy
        assert_eq!((c.r[7], c.s[7]), (0x400, 0x404));
        (c.r[6], c.s[6]) = (0, 0);
        c.set_astat(1, 0xFF);
        c.move_ureg(0x76, 6); // ASTAT = R6 clears ASTATy from S6
        assert_eq!((c.astat(0), c.astat(1)), (0, 0));
    }
}
