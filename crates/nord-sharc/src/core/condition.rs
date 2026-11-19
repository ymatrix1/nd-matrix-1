//! Condition codes of `IF cond` for PEx and PEy (programming reference rev 2.4, table 4-37).
use super::float;
use super::Core;

impl Core {
    /// Whether condition code `c` (0..32, `31` = TRUE) holds for PEx.
    pub(super) fn cond(&self, c: u64) -> bool {
        self.cond_for(false, c)
    }

    /// Whether condition code `c` holds for PEx (`y` = false) or PEy.
    #[inline]
    pub(super) fn cond_for(&self, y: bool, c: u64) -> bool {
        if c == 31 {
            return true;
        }
        let f = if y { self.flags_y } else { self.flags };
        let af = self.sys.astat_extra[usize::from(y)] >> 10 & 1 == 1;
        let sat = self.mode1 & float::MODE1_ALUSAT != 0;
        let lce = self.loops.last().is_some_and(|l| l.remaining > 1);
        let base = match c & 15 {
            0 => f.az,
            // VERIFIED (census diff, `if CC Rd = Rs` for all 64 combinations of AZ AV AN AC AS AI, with
            // and without CACC): LT = AN xor AV, LE = (AN xor AV) or AZ; GE and GT are their negations
            // VERIFIED (census diff, 64 flag combinations x AF in {0, 1}): after a floating-point
            // ALU operation (AF = 1) LT is AN and not AZ, LE is AN or AZ; after a fixed-point
            // one they compare AN with AV
            // with ALUSAT set and AF clear LT is AN alone and LE is AN or AZ (AV is ignored: an
            // overflow saturates instead of flipping the sign) -- VERIFIED the same way
            1 if af => f.an && !f.az,
            1 if sat => f.an,
            1 => f.an != f.av,
            2 if af || sat => f.an || f.az,
            2 => (f.an != f.av) || f.az,
            3 => f.ac,
            4 => f.av,
            // MV and MS test the multiplier overflow and sign flags (ASTAT bits 7 and 6) of the
            // element; NOT MV / NOT MS (21, 22) are their negations. VERIFIED: ADSP-21489
            // simulator, `census_extra_cases.txt` (all 32 condition codes on one instruction,
            // 6 random states), the census `IF NOT MS` triple operation (5 cases).
            5 => self.sys.astat_extra[usize::from(y)] >> 7 & 1 == 1,
            6 => self.sys.astat_extra[usize::from(y)] >> 6 & 1 == 1,
            7 => f.sv,
            8 => f.sz,
            // FLAG0_IN..FLAG3_IN: the pin level of an input flag (PRM Table 4-37)
            n @ 9..=12 => self.flags_value() >> (2 * (n - 9)) & 1 == 1,
            // BM on ADSP-2136x/2146x; on the ADSP-214xx the same code selects SF (table 10-4)
            14 => f.fifo.sf(),
            13 => f.tf,
            15 => !lce,
            _ => false,
        };
        match c {
            0..=15 => base,
            31 => true,
            16..=30 => !base,
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lt_and_le_follow_af_and_alusat() {
        // census diff: 288 + 256 simulator cases over AZ AV AN AC x AF (x ALUSAT)
        let mut c = Core::new(&[]);
        c.flags.an = true;
        c.flags.av = true; // AN xor AV = 0
        assert!(!c.cond(1) && c.cond(17), "fixed: LT false, GE true");
        c.sys.astat_extra[0] |= 1 << 10; // AF: float rule
        assert!(c.cond(1), "after a float op LT = AN and not AZ");
        c.sys.astat_extra[0] &= !(1 << 10);
        c.mode1 |= 1 << 13; // ALUSAT
        assert!(c.cond(1), "with ALUSAT LT = AN");
        c.flags.az = true;
        assert!(c.cond(1) && c.cond(2));
    }

    /// Direct check of codes 5 (MV), 6 (MS), 21 (NOT MV) and 22 (NOT MS), independent of the
    /// census replay: they read ASTAT bits 7 and 6 of the element, PEx and PEy separately.
    /// VERIFIED values of the bits: ADSP-21489 simulator, `census_extra_cases.txt` (all 32 codes).
    #[test]
    fn mv_and_ms_follow_astat_bits_7_and_6_of_each_element() {
        for (code, bit) in [(5u64, 7u32), (6, 6)] {
            let not = code + 16;
            let mut c = Core::new(&[]);
            assert!(
                !c.cond(code) && c.cond(not),
                "clear bit: code {code} false, {not} true"
            );
            assert!(!c.cond_for(true, code) && c.cond_for(true, not));
            c.sys.astat_extra[0] = 1 << bit;
            assert!(c.cond(code) && !c.cond(not), "PEx bit {bit} set");
            assert!(
                !c.cond_for(true, code) && c.cond_for(true, not),
                "PEy untouched"
            );
            c.sys.astat_extra[0] = 0;
            c.sys.astat_extra[1] = 1 << bit;
            assert!(!c.cond(code) && c.cond(not), "PEx untouched");
            assert!(
                c.cond_for(true, code) && !c.cond_for(true, not),
                "PEy bit {bit} set"
            );
            // the neighbouring bit (MS for MV, MV for MS) does not leak in
            c.sys.astat_extra = [1 << (13 - bit); 2];
            assert!(!c.cond(code) && !c.cond_for(true, code));
        }
    }
}
