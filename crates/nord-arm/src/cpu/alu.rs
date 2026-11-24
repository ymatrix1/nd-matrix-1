//! Shifter, add-with-carry and saturation helpers shared by the ARM and Thumb decoders.

/// Barrel shifter result `(value, carry_out)`. `ty`: 0 LSL, 1 LSR, 2 ASR, 3 ROR. `imm` selects the
/// immediate-amount encodings (amount 0 of LSR/ASR means 32, of ROR means RRX); register amounts
/// use the low byte of the register with amount 0 meaning "unchanged".
pub fn shift(ty: u32, amount: u32, val: u32, carry: bool, imm: bool) -> (u32, bool) {
    if imm {
        return match (ty, amount) {
            (0, 0) => (val, carry),
            (0, n) => (val << n, val >> (32 - n) & 1 != 0),
            (1, 0) => (0, val >> 31 != 0),
            (1, n) => (val >> n, val >> (n - 1) & 1 != 0),
            (2, 0) => (((val as i32) >> 31) as u32, val >> 31 != 0),
            (2, n) => (((val as i32) >> n) as u32, val >> (n - 1) & 1 != 0),
            (_, 0) => ((val >> 1) | ((carry as u32) << 31), val & 1 != 0),
            (_, n) => (val.rotate_right(n), val >> (n - 1) & 1 != 0),
        };
    }
    let n = amount & 0xFF;
    if n == 0 {
        return (val, carry);
    }
    match ty {
        0 => match n {
            1..=31 => (val << n, val >> (32 - n) & 1 != 0),
            32 => (0, val & 1 != 0),
            _ => (0, false),
        },
        1 => match n {
            1..=31 => (val >> n, val >> (n - 1) & 1 != 0),
            32 => (0, val >> 31 != 0),
            _ => (0, false),
        },
        2 => match n {
            1..=31 => (((val as i32) >> n) as u32, val >> (n - 1) & 1 != 0),
            _ => (((val as i32) >> 31) as u32, val >> 31 != 0),
        },
        _ => {
            let r = n & 31;
            if r == 0 {
                (val, val >> 31 != 0)
            } else {
                (val.rotate_right(r), val >> (r - 1) & 1 != 0)
            }
        }
    }
}

/// `a + b + carry_in` with carry-out and signed overflow.
#[inline(always)]
pub fn add_c(a: u32, b: u32, c: bool) -> (u32, bool, bool) {
    let wide = a as u64 + b as u64 + c as u64;
    let r = wide as u32;
    (r, wide >> 32 != 0, (!(a ^ b) & (a ^ r)) >> 31 != 0)
}

/// Signed saturating add; second field is true when it saturated.
pub fn qadd(a: i32, b: i32) -> (i32, bool) {
    match a.checked_add(b) {
        Some(v) => (v, false),
        None => (if a < 0 { i32::MIN } else { i32::MAX }, true),
    }
}

/// Signed saturating subtract; second field is true when it saturated.
pub fn qsub(a: i32, b: i32) -> (i32, bool) {
    match a.checked_sub(b) {
        Some(v) => (v, false),
        None => (if a < 0 { i32::MIN } else { i32::MAX }, true),
    }
}

/// Expand an 8-bit immediate rotated right by `2 * rot` (ARM data-processing immediates).
#[inline(always)]
pub fn arm_imm(imm8: u32, rot: u32) -> u32 {
    imm8.rotate_right(rot * 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immediate_shift_special_cases() {
        assert_eq!(shift(1, 0, 0x8000_0000, false, true), (0, true)); // LSR #32
        assert_eq!(shift(2, 0, 0x8000_0000, false, true), (0xFFFF_FFFF, true)); // ASR #32
        assert_eq!(shift(3, 0, 0x3, true, true), (0x8000_0001, true)); // RRX
        assert_eq!(shift(0, 4, 0xF000_0001, false, true), (0x10, true));
    }

    #[test]
    fn register_shift_amounts() {
        assert_eq!(shift(0, 0, 5, true, false), (5, true));
        assert_eq!(shift(0, 32, 1, false, false), (0, true));
        assert_eq!(shift(0, 33, 1, false, false), (0, false));
        assert_eq!(shift(3, 32, 0x8000_0000, false, false), (0x8000_0000, true));
        assert_eq!(shift(2, 40, 0x8000_0000, false, false), (0xFFFF_FFFF, true));
    }

    #[test]
    fn add_carry_and_overflow() {
        assert_eq!(add_c(0xFFFF_FFFF, 1, false), (0, true, false));
        assert_eq!(add_c(0x7FFF_FFFF, 1, false), (0x8000_0000, false, true));
        assert_eq!(add_c(1, 1, true), (3, false, false));
    }

    #[test]
    fn saturation_and_rotated_immediates() {
        assert_eq!(qadd(i32::MAX, 1), (i32::MAX, true));
        assert_eq!(qsub(i32::MIN, 1), (i32::MIN, true));
        assert_eq!(qadd(1, 2), (3, false));
        assert_eq!(arm_imm(0xFF, 4), 0xFF00_0000);
    }
}
