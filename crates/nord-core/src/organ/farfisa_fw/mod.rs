//! Farfisa processor `0x122B85` on the firmware machine (decoded rows, 40 bit float).
//!
//! Research: `docs/research/organ-farfisa-bass.md` s16. The routine is not re-derived: its listing
//! is decoded into rows ([`rows`]: jumps and the AZ / AN flags included, the rules of
//! `tools/sharc_listing_rust.py`) and executed on the machine of [`super::b3_bass_fw`], which the
//! Bass stage proved bit exact. The rows and the DM image are built at setup time from the user's
//! own OS update file ([`image`], D128): nothing of the firmware is tracked, and without the file
//! the stage does not exist (the layer plays its non-firmware Farfisa model, D22). The input is
//! the generator ring (frame 0 at `DM 0xC2B52`, frame 1 at `0xC2BB4`), the weights are the
//! key-down words of the ARM.
//!
//! Real-time safety: construction allocates the DM image; `step` does not allocate, lock or do I/O.

mod image;

use super::b3_bass_fw::Machine;

pub use image::firmware_status;
#[cfg(test)]
pub(in crate::organ) use image::tests::{real_image, synthetic_image};
pub(in crate::organ) use image::{from_env, FarfisaImage};

/// Presets of the processor call (`I0` output stream, `I1` record; VERIFIED as the oracle harness).
const PRESET: (u32, u32) = (0xC_4000, 0xB_474C);
/// Ring frame 0 / frame 1 and the words per frame (VERIFIED, research s3: `0x62` = 98).
pub(super) const RING: [u32; 2] = [0xC_2B52, 0xC_2BB4];
pub(super) const RING_LEN: usize = 98;
/// Key-down weight words of key `n`: `WEIGHT_BASE + n + off` for `off` in `WEIGHT_OFFSETS`, each
/// with one bias word (VERIFIED: `keydown-organ-models.csv`, model 2, notes 36, 60, 84).
const WEIGHT_BASE: u32 = 0x9_5FE4;
const WEIGHT_OFFSETS: [u32; 3] = [0x00, 0x62, 0xD0];
const BIAS_WORDS: [u32; 3] = [0xC_2D58, 0xC_2D59, 0xC_2D5C];
/// Key-down weight `1/48` and bias word (VERIFIED: the words the ARM writes).
const KEY_WEIGHT: u32 = 0x3CAA_AA80;
const BIAS: u32 = 0xBC1D_5580;
/// Lattice flag word (`dm(i3 - 0x14)`, `i3 = 0xC2C40`): 0 selects the first lattice table, a non
/// zero value the second (VERIFIED against `sharc-chain`, both branches, research s16).
#[cfg(test)]
pub(super) const LATTICE_FLAG: u32 = 0xC_2C2C;

/// The processor machine: DM image, key weights and the ring input.
#[derive(Debug, Clone)]
pub(super) struct FarfisaFw {
    machine: Machine,
}

impl FarfisaFw {
    /// Machine on the model 2 image built from the user's OS update file (setup time only:
    /// allocates and compiles the rows).
    pub(super) fn new(image: &FarfisaImage) -> Self {
        let mut machine = Machine::new();
        machine.load_image(&image.csv);
        machine.set_programs(None, &image.rows);
        Self { machine }
    }

    /// Writes the key-down words of `key`: the weight `1/48 * gain` at the three sums (`gain` 0
    /// clears them; the gain is the velocity scale, the processor is linear in the weights).
    pub(super) fn set_key(&mut self, key: u32, gain: f32) {
        let w = f32::from_bits(KEY_WEIGHT) * gain;
        for off in WEIGHT_OFFSETS {
            self.machine.wr_f(WEIGHT_BASE + key + off, w);
        }
    }

    /// Writes (`on`) or clears the three shared bias words.
    pub(super) fn set_bias(&mut self, on: bool) {
        for a in BIAS_WORDS {
            self.machine.wr(a, if on { BIAS } else { 0 });
        }
    }

    /// Selects the lattice table (`flag` non zero: second table).
    #[cfg(test)]
    pub(super) fn set_lattice_flag(&mut self, flag: bool) {
        self.machine
            .wr_f(LATTICE_FLAG, if flag { 1.0 } else { 0.0 });
    }

    /// Writes one ring frame (`frame` 0 or 1) of [`RING_LEN`] words.
    pub(super) fn set_ring(&mut self, frame: usize, ring: &[f32; RING_LEN]) {
        for (k, &v) in ring.iter().enumerate() {
            self.machine.wr_f(RING[frame] + k as u32, v);
        }
    }

    /// Writes the raw words of both frames (golden tests): 196 words from `DM 0xC2B52`.
    #[cfg(test)]
    pub(super) fn set_ring_words(&mut self, words: &[u32]) {
        for (k, &w) in words.iter().enumerate() {
            self.machine.wr(RING[0] + k as u32, w);
        }
    }

    /// One processor call (two frames): the two output samples of lane PEx.
    pub(super) fn step(&mut self) -> [f32; 2] {
        self.machine.call_main(PRESET);
        [self.machine.rd_f(PRESET.0), self.machine.rd_f(PRESET.0 + 2)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(call, out0, out1, ring words)` rows of a golden CSV.
    fn golden(csv: &str) -> Vec<([u32; 2], Vec<u32>)> {
        let hex = |s: &str| u32::from_str_radix(s, 16).unwrap();
        csv.lines()
            .filter(|l| !l.starts_with('#'))
            .map(|l| {
                let f: Vec<&str> = l.split(',').collect();
                (
                    [hex(f[1]), hex(f[2])],
                    f[3].split(' ').map(hex).collect::<Vec<u32>>(),
                )
            })
            .collect()
    }

    fn chord_machine(image: &FarfisaImage, flag: bool) -> FarfisaFw {
        let mut m = FarfisaFw::new(image);
        for key in [60, 64, 67] {
            m.set_key(key, 1.0);
        }
        m.set_bias(true);
        if flag {
            m.set_lattice_flag(true);
        }
        m
    }

    fn run(image: &FarfisaImage, golden_csv: &str, flag: bool) -> usize {
        let mut m = chord_machine(image, flag);
        let rows = golden(golden_csv);
        for (call, (want, ring)) in rows.iter().enumerate() {
            m.set_ring_words(ring);
            let got = m.step();
            assert_eq!([got[0].to_bits(), got[1].to_bits()], *want, "call {call}");
        }
        rows.len()
    }

    #[test]
    fn decoded_processor_is_bit_exact_against_the_oracle_golden() {
        // Needs the user's OS update file (`NS4_WAVE_TABLES`); skipped with a note without it.
        let Some(image) = real_image() else { return };
        // `farfisa-fw-golden.csv`: 24 calls of `sharc-chain` (boot + model 2 install + key-down 60
        // 64 67; the ring of each call is the generator's). VERIFIED: every output word equal in the
        // first lattice table (flag 0), i.e. branches, flags, SIMD access and the 40 bit float.
        assert_eq!(
            run(
                &image,
                include_str!("../../../../../docs/research/data/farfisa-fw-golden.csv"),
                false
            ),
            24
        );
    }

    #[test]
    fn second_lattice_table_is_bit_exact_too() {
        let Some(image) = real_image() else { return };
        // the `if ne jump 0x122D88` branch (flag word non zero), 12 calls, VERIFIED the same way
        assert_eq!(
            run(
                &image,
                include_str!("../../../../../docs/research/data/farfisa-fw-golden-flag1.csv"),
                true
            ),
            12
        );
    }

    #[test]
    fn compiled_processor_equals_the_reference_executor_state_for_state() {
        let Some(image) = real_image() else { return };
        // both lattice tables (flag 0 and 1: the conditional jump), the golden rings of the oracle:
        // after every call the whole machine of the compiled and the row executor is equal
        for (csv, flag) in [
            (
                include_str!("../../../../../docs/research/data/farfisa-fw-golden.csv"),
                false,
            ),
            (
                include_str!("../../../../../docs/research/data/farfisa-fw-golden-flag1.csv"),
                true,
            ),
        ] {
            let (mut fast, mut slow) = (chord_machine(&image, flag), chord_machine(&image, flag));
            for (call, (_, ring)) in golden(csv).iter().enumerate() {
                fast.set_ring_words(ring);
                slow.set_ring_words(ring);
                fast.machine.call_main(PRESET);
                slow.machine.call(&image.rows, PRESET);
                let (a, b) = (&fast.machine, &slow.machine);
                assert_eq!(
                    (a.d, a.i, a.mr, a.an, a.az),
                    (b.d, b.i, b.mr, b.an, b.az),
                    "call {call}"
                );
                assert!(a.mem == b.mem, "DM, call {call}");
            }
        }
    }

    #[test]
    fn silent_without_keys_and_ring_input() {
        let Some(image) = real_image() else { return };
        let mut m = FarfisaFw::new(&image);
        for _ in 0..4 {
            assert_eq!(m.step(), [0.0, 0.0]);
        }
    }

    #[test]
    fn key_words_land_at_the_three_sums() {
        let mut m = FarfisaFw::new(&image::tests::synthetic_image());
        m.set_key(60, 1.0);
        assert_eq!(m.machine.rd(0x9_6020), KEY_WEIGHT);
        assert_eq!(m.machine.rd(0x9_6020 + 0x62), KEY_WEIGHT);
        m.set_key(60, 0.0);
        assert_eq!(m.machine.rd(0x9_6020 + 0xD0), 0);
        m.set_bias(true);
        assert_eq!(m.machine.rd(0xC_2D59), BIAS);
    }

    #[test]
    fn a_synthetic_image_loads_registers_words_and_runs_its_routine() {
        // no firmware bytes: a three instruction routine `R8 = 1.0; DM(0xC4000) = R8; rts`
        let mut m = FarfisaFw::new(&image::tests::synthetic_image());
        assert_eq!(m.machine.rd(0xC_27E0), 0x3F80_0000, "image word");
        assert_eq!(m.machine.i[0], 0xC_00EA, "image register");
        assert_eq!(m.step(), [1.0, 0.0]);
        assert_eq!(m.machine.i[0], PRESET.0, "the call presets I0");
    }
}
