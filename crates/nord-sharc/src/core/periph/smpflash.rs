//! The SMP sample flash behind the external-port banks 1 to 3 (OQ41, D191, opt-in).
//!
//! The SMP page routine (`0x1C0C25`, `0x1C0DA8`, listing `0x3818B4..0x381AB4`) starts a stroke page
//! with a direct read of the page header word, then the audio ISR refills the stream ring with
//! external-port DMA chains. The address arithmetic, from the listing:
//!
//! | Step | Code | Tag |
//! |---|---|---|
//! | page address `R7 = DM(0xB237B + rec[-11] + (rec[-10] >> g)) + (rec[-10] & (2^g - 1)) * s`, `(g, s) = (6, 0x3FE)` for a stereo stroke (`rec[-21] != 0`), `(7, 0x1FF)` for mono | `0x3818C2..0x3818FC`, `0x3819A8..0x3819E2` | VERIFIED (listing) |
//! | latch `R9 = R7 >> 24`, `R7 &= 0xFFFFFF`; `DM(0x8000000 + R7) = R9`; the header word is `DM(0x4000000 + R7)` | `0x3818FE..0x381928`, `0x3819E4..0x381A10` | VERIFIED (listing) |
//! | the DMA chain reads bank 3 at `0xC000000 + R7 + position` into the ring (`rec[-14]`), the latch address in bank 2 first | `0x381AA4..0x381AB4`, `0x381B00..0x381CCA`; `data/smp-page-refill-replay.txt` | VERIFIED (run) |
//!
//! So all three banks are one 16-bit-word flash: the **flash word address** is
//! `latch << 24 | (ext & 0xFFFFFF)`, bank 1 returns the word at it (the page header), bank 2 takes
//! the latch (the written data, not the address) and bank 3 returns the word at it. A word is the
//! big-endian 16 bits of the `.npno` byte stream (`smp-voice.md` s4: the payload words go to the
//! ring unchanged, one per DM word). The file sits at flash word `base` (the file word `i` is the
//! flash word `base + i`).
//!
//! Which flash word holds the file is decided by the ARM through the directory
//! `DM(0xB237B + i)` (its flash file system); the harness has no such system, so [`SmpFlash::dir_value`]
//! gives the linear directory `65408 * i` (a group of 64 stereo pages of 1022 words, or 128 mono
//! pages of 511 words, is 65408 words) and the descriptor law `docs/research/smp-page-refill.md`
//! fixes `base = 65408 * rec[-11] + 74` for that directory.
use std::collections::BTreeMap;

/// External banks seen by the page routine (VERIFIED, listing): header read, latch write, data read.
pub const BANK1: u32 = 0x0400_0000;
pub const BANK2: u32 = 0x0800_0000;
pub const BANK3: u32 = 0x0C00_0000;
/// Words of each bank window (24 address bits, VERIFIED: `R7 &= 0xFFFFFF`).
pub const BANK_WORDS: u32 = 0x0100_0000;
/// Words of a directory group: `64 * 1022 = 128 * 511` (VERIFIED, listing strides `0x3FE`, `0x1FF`).
pub const GROUP_WORDS: u32 = 65_408;
/// Base of the page directory in DM (VERIFIED, `I0 = 0xB237B`).
pub const DIR_BASE: u32 = 0xB_237B;

/// The flash image and its address latch.
#[derive(Debug, Default)]
pub struct SmpFlash {
    bytes: Vec<u8>,
    base: u64,
    latch: u32,
    /// Reads served, by bank (1 header words, 3 data words).
    pub reads: BTreeMap<u32, u64>,
    /// Reads that fell outside the image (answered 0).
    pub outside: u64,
}

impl SmpFlash {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            ..Self::default()
        }
    }

    /// Words of the image.
    pub fn words(&self) -> u64 {
        self.bytes.len() as u64 / 2
    }

    pub fn latch(&self) -> u32 {
        self.latch
    }

    /// Places file word 0 at flash word `base`.
    pub fn set_base(&mut self, base: u64) {
        self.base = base;
    }

    pub fn base(&self) -> u64 {
        self.base
    }

    /// Whether `ext` is in one of the three bank windows.
    pub fn covers(ext: u32) -> bool {
        [BANK1, BANK2, BANK3]
            .iter()
            .any(|b| (*b..*b + BANK_WORDS).contains(&ext))
    }

    /// Flash word address of external word `ext` under the current latch.
    pub fn flash_word(&self, ext: u32) -> u64 {
        u64::from(self.latch) << 24 | u64::from(ext & 0xFF_FFFF)
    }

    /// The word a read of `ext` returns (bank 1 or 3), `None` for another address.
    pub fn read(&mut self, ext: u32) -> Option<u32> {
        let bank = [BANK1, BANK3]
            .into_iter()
            .find(|b| (*b..*b + BANK_WORDS).contains(&ext))?;
        *self.reads.entry(bank).or_default() += 1;
        let fw = self.flash_word(ext);
        let i = fw.wrapping_sub(self.base);
        let at = i
            .checked_mul(2)
            .and_then(|a| usize::try_from(a).ok())
            .filter(|a| a + 2 <= self.bytes.len());
        Some(match at {
            Some(a) => u32::from(u16::from_be_bytes([self.bytes[a], self.bytes[a + 1]])),
            None => {
                self.outside += 1;
                0
            }
        })
    }

    /// A write of `value` to `ext`: a bank 2 write sets the latch (the data, not the address).
    pub fn write(&mut self, ext: u32, value: u32) -> bool {
        if !(BANK2..BANK2 + BANK_WORDS).contains(&ext) {
            return false;
        }
        self.latch = value & 0xFF;
        true
    }

    /// Directory word `i` of the linear directory (see the module text).
    pub fn dir_value(i: u32) -> u32 {
        i.wrapping_mul(GROUP_WORDS)
    }

    /// Flash base word of a file whose directory index starts at `rec11` (`rec[-11]`).
    pub fn base_for(rec11: u32) -> u64 {
        u64::from(rec11) * u64::from(GROUP_WORDS) + 74
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flash() -> SmpFlash {
        SmpFlash::new((0u8..=255).collect())
    }

    #[test]
    fn bank_windows_are_the_three_banks_only() {
        assert!(SmpFlash::covers(BANK1) && SmpFlash::covers(BANK2 + 5) && SmpFlash::covers(BANK3));
        assert!(!SmpFlash::covers(0x0000_1000) && !SmpFlash::covers(0x1000_0000));
    }

    #[test]
    fn a_read_returns_the_big_endian_word_of_the_file() {
        let mut f = flash();
        assert_eq!(f.read(BANK3 + 1), Some(0x0203));
        assert_eq!(f.read(BANK1), Some(0x0001));
        assert_eq!(f.read(BANK2), None, "bank 2 is write only");
        assert_eq!(f.reads[&BANK3], 1);
    }

    #[test]
    fn the_latch_supplies_the_upper_address_bits_and_base_shifts_the_file() {
        let mut f = flash();
        assert!(f.write(BANK2 + 7, 3) && f.latch() == 3);
        assert!(!f.write(BANK3, 1));
        f.set_base(3u64 << 24 | 10);
        assert_eq!(f.read(BANK3 + 10), Some(0x0001));
        assert_eq!(f.read(BANK3 + 11), Some(0x0203));
        assert_eq!(f.read(BANK3 + 9), Some(0), "before the file");
        assert_eq!(f.outside, 1);
    }

    #[test]
    fn linear_directory_and_base_match_the_descriptor_law() {
        // flash word of page v with directory index rec11: dir[rec11 + (v >> 6)] + 1022 * (v & 63)
        let (rec11, v) = (28_271u32, 40_574u32);
        let w = u64::from(SmpFlash::dir_value(rec11 + (v >> 6))) + 1022 * u64::from(v & 63);
        // file word of the page = 1022 * v - 74 (file byte offset 2044 * v - 148)
        assert_eq!(w - SmpFlash::base_for(rec11), 1022 * u64::from(v) - 74);
    }
}
