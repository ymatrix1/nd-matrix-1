//! Pure bit helpers of the shifter (`shifter.rs`): logical and arithmetic shift by a signed count,
//! field deposit and extract, and the short float pack and unpack of `FPACK` / `FUNPACK`.

/// Mask of the low `len` bits (`len` of 32 or more selects all 32).
#[inline]
pub(super) fn low_mask(len: u32) -> u32 {
    if len >= 32 {
        u32::MAX
    } else {
        (1u32 << len) - 1
    }
}

/// `v` shifted left by `s` (right when negative); an off-scale shift gives 0, or the sign
/// fill for an arithmetic right shift.
pub(super) fn shift(v: u32, s: i32, arithmetic: bool) -> u32 {
    if s >= 0 {
        if s >= 32 {
            0
        } else {
            v << s
        }
    } else if s.unsigned_abs() >= 32 {
        if arithmetic {
            ((v as i32) >> 31) as u32
        } else {
            0
        }
    } else if arithmetic {
        ((v as i32) >> -s) as u32
    } else {
        v >> -s
    }
}

/// `FPACK`: an IEEE single to the 16-bit short float (sign 15, exponent 14..11, fraction 10..0).
/// Returns the packed value and whether it overflowed (programming reference 11-84).
///
/// Source exponent `e`: above 135 the largest magnitude; 121..=135 the exponent is the MSB of `e`
/// followed by its three LSBs and the fraction is the upper 11 bits, rounded; 110..=120 a
/// denormal `1.f * 2^(e - 110)` (gradual underflow, rounded); below 110 zero. ASSUMED: rounding
/// is to nearest with ties to even, and a rounding carry increments the exponent.
pub(super) fn fpack(x: u32) -> (u32, bool) {
    let sign = (x >> 31) << 15;
    let e = (x >> 23) & 0xFF;
    let frac = x & 0x7F_FFFF;
    let round = |value: u32, drop: u32| -> u32 {
        // value >> drop with round to nearest even
        let kept = value >> drop;
        let rest = value & ((1u32 << drop) - 1);
        let half = 1u32 << (drop - 1);
        kept + u32::from(rest > half || (rest == half && kept & 1 == 1))
    };
    if e > 135 {
        return (sign | 0x7FFF, true);
    }
    let magnitude = if e > 120 {
        let packed = ((((e >> 7) & 1) << 3) | (e & 7)) << 11;
        packed + round(frac, 12) // a carry out of the fraction increments the exponent
    } else if e > 109 {
        round(0x80_0000 | frac, 133 - e)
    } else {
        0
    };
    if magnitude > 0x7FFF {
        return (sign | 0x7FFF, true);
    }
    (sign | magnitude, false)
}

/// `FUNPACK`: the 16-bit short float in the low half of `x` to an IEEE single.
pub(super) fn funpack(x: u32) -> u32 {
    let sign = ((x >> 15) & 1) << 31;
    let exp = (x >> 11) & 0xF;
    let frac = x & 0x7FF;
    if exp != 0 {
        let msb = exp >> 3;
        let e8 = (msb << 7) | ((1 - msb) * 0xF) << 3 | (exp & 7);
        return sign | (e8 << 23) | (frac << 12);
    }
    if frac == 0 {
        return sign;
    }
    // denormal: exponent 120 - N for N leading zeros of the 11-bit fraction, hidden 1 stripped
    let leading = frac.leading_zeros() - 21;
    let normalised = (frac << (leading + 1)) & 0x7FF;
    sign | ((120 - leading) << 23) | (normalised << 12)
}

/// Field deposit: the low `len` bits of `x` placed at bit `pos`; returns the 32-bit result and
/// whether bits fell off the left end (`len + pos > 32`). With `sign_extend` the bits above the
/// field are filled with the field's MSB unless that MSB is off-scale left.
pub(super) fn deposit(x: u32, len: u32, pos: u32, sign_extend: bool) -> (u32, bool) {
    let len = len.min(32);
    let field = u64::from(x & low_mask(len));
    let mut wide = field << pos;
    let msb_pos = pos + len; // one above the field's MSB
    if sign_extend && len > 0 && msb_pos <= 32 && (field >> (len - 1)) & 1 == 1 {
        wide |= (u64::MAX << msb_pos) & u64::from(u32::MAX);
    }
    (wide as u32, len + pos > 32)
}

/// Field extract: `len` bits of `x` starting at bit `pos`, right aligned; returns the result and
/// whether bits came from beyond the left end (`len + pos > 32`).
pub(super) fn extract(x: u32, len: u32, pos: u32, sign_extend: bool) -> (u32, bool) {
    let len = len.min(32);
    let src = if pos >= 32 { 0 } else { x >> pos };
    let mut v = src & low_mask(len);
    if sign_extend && len > 0 && pos + len <= 32 && (v >> (len - 1)) & 1 == 1 {
        v |= !low_mask(len);
    }
    (v, len + pos > 32)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn short_float_pack_and_unpack() {
        let f = |x: f32| x.to_bits();
        assert_eq!(
            fpack(f(1.0)),
            (0x3800, false),
            "exponent 127 -> MSB 0, LSBs 7"
        );
        assert_eq!(
            fpack(f(2.0)),
            (0x4000, false),
            "exponent 128 -> MSB 1, LSBs 0"
        );
        assert_eq!(fpack(f(-1.0)).0, 0xB800);
        assert_eq!(funpack(0x3800), f(1.0));
        assert_eq!(funpack(0x4000), f(2.0));
        assert_eq!(funpack(0xB800), f(-1.0));
        // a fraction survives with 11 bits: 1.5 and 1 + 2^-11
        assert_eq!(funpack(fpack(f(1.5)).0), f(1.5));
        assert_eq!(
            funpack(fpack(f(1.0 + 1.0 / 2048.0)).0),
            f(1.0 + 1.0 / 2048.0)
        );
        // rounding carries into the exponent: the largest fraction below 2.0 rounds up to 2.0
        assert_eq!(fpack(0x3FFF_FFFF), (0x4000, false));
        // overflow gives the largest magnitude and sets SV, zero stays zero
        assert_eq!(fpack(f(1.0e10)), (0x7FFF, true));
        assert_eq!(fpack(f(-1.0e10)), (0xFFFF, true));
        assert_eq!(fpack(0), (0, false));
        // gradual underflow: 2^-8 packs as the denormal 0x0200 and unpacks exactly
        assert_eq!(fpack(f(2f32.powi(-8))), (0x0200, false));
        assert_eq!(funpack(0x0200), f(2f32.powi(-8)));
        assert_eq!(funpack(0x0001), f(2f32.powi(-17)), "the smallest denormal");
        assert_eq!(fpack(f(2f32.powi(-40))), (0, false), "too small");
        // the exponent range maps back for every packed value
        for packed in (0u32..0x8000).step_by(7) {
            let (again, overflow) = fpack(funpack(packed));
            assert!(!overflow);
            assert_eq!(again, packed, "round trip of {packed:#x}");
        }
    }

    #[test]
    fn a_shift_count_of_i32_min_does_not_overflow() {
        // census diff: a random shift count reached `-s` with s = i32::MIN and panicked
        assert_eq!(shift(0x8000_0000, i32::MIN, true), u32::MAX);
        assert_eq!(shift(0x8000_0000, i32::MIN, false), 0);
        assert_eq!(shift(1, i32::MAX, false), 0);
    }
}
