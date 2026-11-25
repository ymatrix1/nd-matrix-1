//! External delay-line window of the FX, PNO and ORG chips: a dense word store at the normal-word
//! addresses `0x0800_0000..0x0804_0000` (1 MiB), instead of the sparse device map.
//!
//! Mapping (`ns4-lle4`, `docs/research/sharc-sdram-window.md`):
//! `loader byte address = 0x2000_0000 + 4 * (word address - 0x0800_0000)`.
//!
//! - VERIFIED: the FX loader streams zero-fill exactly 409,600 B = 5 lines x 0x5000 words x 4 B at
//!   `0x2001_0000` (Core1 section) and `0x2008_0000` (Core2 section) (`python tools/ns4_dsp_ldr.py
//!   target/DSP/dsp_image_0_FX.bin`), and the reverb instances point their lines at the word
//!   addresses `0x0800_4000 + 0x5000 k` (Core1) and `0x0802_0000 + 0x5000 k` (Core2)
//!   (`fx-reverb-leslie.md` s11): `0x0800_4000` maps to `0x2001_0000`, `0x0802_0000` to
//!   `0x2008_0000`. Checked by the unit tests below against those constants.
//! - ASSUMED: the physical device (the chip's L2 or an SDRAM bank) is not identified; the model
//!   is plain read/write memory, as the sparse store was, with no latency and no refresh.
//! - ASSUMED: the window is 1 MiB, the size of the SC57x L2; no firmware access beyond the five
//!   lines of a core was seen.
//!
//! The words are allocated on first write so cores that never touch the window pay nothing.

/// First normal-word address of the window.
pub const BASE: u32 = 0x0800_0000;
/// Window size in 32-bit words (1 MiB).
pub const WORDS: u32 = 0x4_0000;
/// Loader byte address of the first window word.
pub const LOADER_BASE: u32 = 0x2000_0000;

/// Dense word store of the window.
#[derive(Default)]
pub struct Sdram {
    mem: Vec<u32>,
}

impl Sdram {
    /// Whether the normal-word address `addr` lies in the window.
    #[inline]
    pub fn contains(addr: u32) -> bool {
        addr.wrapping_sub(BASE) < WORDS
    }

    /// Loader byte address of the word at `addr`.
    pub fn loader_addr(addr: u32) -> u32 {
        LOADER_BASE + 4 * addr.wrapping_sub(BASE)
    }

    /// The word at `addr` (0 when never written; `addr` must be inside the window).
    #[inline]
    pub fn read(&self, addr: u32) -> u32 {
        self.mem
            .get(addr.wrapping_sub(BASE) as usize)
            .copied()
            .unwrap_or(0)
    }

    /// Stores the word at `addr` (inside the window).
    #[inline]
    pub fn write(&mut self, addr: u32, value: u32) {
        if self.mem.is_empty() {
            self.mem = vec![0; WORDS as usize];
        }
        self.mem[addr.wrapping_sub(BASE) as usize] = value;
    }

    /// Copies a loader block (`bytes` little-endian words) to the loader byte address `at`.
    /// Returns false when the block does not lie inside the window.
    pub fn load(&mut self, at: u32, bytes: &[u8]) -> bool {
        let off = at.wrapping_sub(LOADER_BASE);
        if !off.is_multiple_of(4) || off as usize + bytes.len() > 4 * WORDS as usize {
            return false;
        }
        for (k, w) in bytes.chunks(4).enumerate() {
            let mut b = [0u8; 4];
            b[..w.len()].copy_from_slice(w);
            self.write(BASE + off / 4 + k as u32, u32::from_le_bytes(b));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_bounds_and_untouched_words_read_zero() {
        assert!(Sdram::contains(BASE) && Sdram::contains(BASE + WORDS - 1));
        assert!(!Sdram::contains(BASE - 1) && !Sdram::contains(BASE + WORDS));
        let s = Sdram::default();
        assert_eq!(s.read(0x0800_4000), 0);
    }

    /// The loader mapping against the VERIFIED FX constants (see the module docs).
    #[test]
    fn the_fx_line_bases_map_onto_the_loader_zero_blocks() {
        const LINE: u32 = 0x5000;
        assert_eq!(Sdram::loader_addr(0x0800_4000), 0x2001_0000);
        assert_eq!(Sdram::loader_addr(0x0802_0000), 0x2008_0000);
        // five lines of 0x5000 words fill 409,600 B and end at the loader block ends
        assert_eq!(4 * 5 * LINE, 409_600);
        assert_eq!(Sdram::loader_addr(0x0800_4000 + 5 * LINE), 0x2007_4000);
        assert_eq!(Sdram::loader_addr(0x0802_0000 + 5 * LINE), 0x200E_4000);
    }

    #[test]
    fn words_round_trip_and_loader_blocks_land_in_place() {
        let mut s = Sdram::default();
        s.write(0x0802_0010, 0xDEAD_BEEF);
        assert_eq!(s.read(0x0802_0010), 0xDEAD_BEEF);
        assert!(s.load(0x2008_0020, &[1, 0, 0, 0, 2, 0, 0, 0, 3]));
        assert_eq!(
            (
                s.read(0x0802_0008),
                s.read(0x0802_0009),
                s.read(0x0802_000A)
            ),
            (1, 2, 3)
        );
        assert!(!s.load(0x2008_0001, &[0; 4]), "unaligned");
        assert!(!s.load(0x2010_0000, &[0; 4]), "outside the window");
    }
}
