//! The B3 Bass image, built at setup time from the user's own OS update file (D128).
//!
//! The firmware PNO image (`bank0_CODE`, download index 1, core 1) holds the generator `0x121BBE`,
//! the processor `0x122296` and the initial DM of the machine. Four parts make the machine of
//! [`super::Machine`]:
//!
//! | Part | Source |
//! |---|---|
//! | instruction rows of the two routines | the core 1 listing, [`crate::organ::sharc_rows`] |
//! | DM words of the 25 probed clusters | core 1 loader data at byte `4 * address`, [`crate::dsp_image`] |
//! | words the ARM sends from its constants | `bank0_CODE` of the same file, named by `A` lines of [`DELTA`] |
//! | boot and model 5 install words, start registers | [`DELTA`], our own capture of the ARM (1,898 words) |
//!
//! Nothing of the firmware is stored: the file is read when the stage is switched on and dropped.
//! Without it the stage does not exist and the layer plays the stage-off model (D22).
//!
//! VERIFIED: the image built from the v1.66 update file equals the former tracked
//! `bass-fw-image.csv` (6,881 DM words, 12 registers) and `prog_gen.rs` / `prog_proc.rs` (555 and
//! 898 rows) exactly, 0 differing words, 0 differing rows. The fingerprints of that content are
//! pinned in `the_real_update_file_builds_the_verified_image_and_rows` (run it with
//! `NS4_WAVE_TABLES` set).

use super::Row;
use crate::dsp_image::dm_text::{dm_image_text_shifted, Shift, KNOWN_SHIFTS};
use crate::dsp_image::{self, cache::ImageCache, ldr};
use crate::organ::sharc_rows::routine_rows;
use crate::render_opts_wave::WAVE_TABLES_ENV;
use std::path::Path;
use std::sync::Arc;

/// Generator `0x121BBE..0x121E28` and processor `0x122296..0x12261D` of PNO core 1 (short word
/// addresses).
///
/// VERIFIED: the bounds of the two routines = `docs/research/organ-farfisa-bass.md` s15
/// (`GEN` / `PROC` of `tools/organ_bass_fw_gen.py`), checked by
/// `the_real_update_file_builds_the_verified_image_and_rows` (555 and 898 rows decoded from these
/// ranges equal the former `prog_gen.rs` and `prog_proc.rs`, 0 differing).
const GEN: (u32, u32) = (0x12_1BBE, 0x12_1E28);
const PROC: (u32, u32) = (0x12_2296, 0x12_261D);
/// DM clusters the routines read or write `(base, words)`.
///
/// VERIFIED: the non zero words of the former oracle image merged within 0x40 words
/// (`ranges()` of `tools/organ_bass_fw_gen.py`); the 6,881 non zero words built from them equal
/// the former `bass-fw-image.csv`, same test, 0 differing.
const KEEP: [(u32, u32); 25] = [
    (0x9_5000, 0x54),
    (0x9_5100, 0xA),
    (0x9_5172, 0x7F),
    (0x9_5471, 0x1FD),
    (0x9_594E, 0x1),
    (0x9_59AB, 0x9),
    (0x9_5A10, 0x9),
    (0x9_5A75, 0x9),
    (0x9_5ADA, 0x5),
    (0x9_5B43, 0x1),
    (0x9_5BA3, 0x5),
    (0x9_5C0B, 0x1),
    (0x9_5C6B, 0x1DA),
    (0x9_5FC6, 0x36),
    (0x9_65DC, 0x1EA),
    (0xB_3000, 0x2E8),
    (0xB_33B8, 0xE8E),
    (0xB_4452, 0x2EA),
    (0xB_4858, 0xC),
    (0xB_4C84, 0x37C),
    (0xC_00DC, 0x1),
    (0xC_0175, 0x110),
    (0xC_2B00, 0x52),
    (0xC_2C1A, 0xD8),
    (0xC_2D3E, 0xC2),
];
/// Start registers, boot and install words the loader data lacks (`R`, `M` and `A` lines, see
/// [`crate::dsp_image::dm_text`]; `tools/organ_bass_fw_gen.py image`).
///
/// VERIFIED: diff of the former `bass-fw-image.csv` (an oracle dump after the boot log and the 337
/// install words of model 5) against the loader data (4,732 words: 2,834 equal to `bank0_CODE`
/// words, found by `tools/organ_bass_fw_gen.py convert`, and 1,898 inline, 7 of them clears),
/// checked by the same test and by the bit exact golden in `b3_bass_fw::tests` (needs the file).
const DELTA: &str = include_str!("../../../../../docs/research/data/bass-fw-install-delta.csv");
/// Layout guard: rows of the v1.66 routines and non zero DM words of the image. Another OS
/// version that moved a routine or the ARM constants fails here instead of running a wrong program.
///
/// VERIFIED: 555 + 898 rows and 6,881 words of the v1.66 update file; other versions are measured
/// in `docs/research/firmware-derived-data-inventory.md` (ASSUMED to differ where not listed).
const ROWS: (usize, usize) = (555, 898);
const WORDS: usize = 6_881;
/// FNV-1a 64 fingerprint of the v1.66 DM image text: the `A` lines read `bank0_CODE` at fixed
/// byte offsets, so a build of another OS version with the same counts is still rejected.
///
/// VERIFIED: taken from the image equal to the former `bass-fw-image.csv` (a fingerprint, no data).
const IMAGE_FNV: u64 = 0x9aa8_22f7_1830_282f;

/// A built image: DM image text (`R,` and `M,` lines of `Machine::load_image`) and the rows.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::organ) struct BassImage {
    pub(in crate::organ) csv: String,
    pub(in crate::organ) gen: Vec<Row>,
    pub(in crate::organ) proc: Vec<Row>,
}

impl BassImage {
    /// The image of a flat core 1 SRAM image, the `bank0_CODE` bytes and the install `delta`;
    /// `guard` applies the layout check of the v1.66 routines (tests with synthetic images switch
    /// it off).
    pub(super) fn from_parts(
        sram: &[u8],
        code: &[u8],
        delta: &str,
        guard: bool,
    ) -> Result<Self, String> {
        let gen = routine_rows(sram, GEN.0, GEN.1)?;
        let proc = routine_rows(sram, PROC.0, PROC.1)?;
        if !super::rows_valid(&gen) || !super::rows_valid(&proc) {
            return Err("the Bass routines do not decode into valid rows".into());
        }
        // The `A` lines are written for v1.66; a later OS holds the same constants at moved
        // offsets. The image fingerprint decides which offset table is the right one.
        let tables: &[&[Shift]] = if guard {
            &KNOWN_SHIFTS
        } else {
            &KNOWN_SHIFTS[..1]
        };
        let mut built = None;
        let mut last_err = String::new();
        for shifts in tables {
            match dm_image_text_shifted(sram, code, &KEEP, delta, shifts) {
                Ok((csv, words)) => {
                    let ok = !guard
                        || (words == WORDS
                            && crate::firmware_setting::fnv1a64(csv.as_bytes()) == IMAGE_FNV);
                    built = Some((csv, words));
                    if ok {
                        break;
                    }
                }
                Err(e) => last_err = e,
            }
        }
        let Some((csv, words)) = built else {
            return Err(last_err);
        };
        if guard
            && ((gen.len(), proc.len()) != ROWS
                || words != WORDS
                || crate::firmware_setting::fnv1a64(csv.as_bytes()) != IMAGE_FNV)
        {
            return Err(format!(
                "unsupported Nord OS version: the Bass routines have {} and {} rows and the image \
                 {words} DM words, expected {} and {} rows and {WORDS} words (OS v1.66)",
                gen.len(),
                proc.len(),
                ROWS.0,
                ROWS.1
            ));
        }
        Ok(Self { csv, gen, proc })
    }

    /// The image of the user's OS update file `path` (`.exe`, `.dmg`, `.zip` or the container).
    pub(super) fn from_file(path: &Path) -> Result<Self, String> {
        let code = dsp_image::bank0_code(path)?;
        let pno = dsp_image::image_of(&code, dsp_image::PNO)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let sram = ldr::core_sram(pno, 0).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_parts(&sram, &code, DELTA, true).map_err(|e| format!("{}: {e}", path.display()))
    }
}

static CACHE: ImageCache<BassImage> = ImageCache::new();

/// The image named by `NS4_WAVE_TABLES` (the OS update file, also filled from the saved app
/// setting at start-up, D127 / D129): `Ok(None)` when it is unset, an error when it cannot be read.
/// Read once per path and kept (setup time only).
pub(in crate::organ) fn from_env() -> Result<Option<Arc<BassImage>>, String> {
    from_setting(std::env::var(WAVE_TABLES_ENV).ok().as_deref())
}

/// [`from_env`] for an explicit value of the variable (`None` or blank: no file named).
fn from_setting(value: Option<&str>) -> Result<Option<Arc<BassImage>>, String> {
    CACHE.get(value, BassImage::from_file)
}

/// Whether the Bass firmware stage can be built: `Ok(true)` when `NS4_WAVE_TABLES` names an OS
/// update file that loads, `Ok(false)` when it is unset (the stage stays off and the layer plays
/// the stage-off model, D22), `Err` with the reason when a file is named but cannot be used.
pub fn firmware_status() -> Result<bool, String> {
    from_env().map(|i| i.is_some())
}

#[cfg(test)]
pub(in crate::organ) mod tests {
    use super::*;
    use crate::dsp_image::dm_text::dm_image_text;

    /// Flat SRAM image with tiny routines at both entries: the generator is `rts`, the processor
    /// is `R8 = 1.0; DM(0xC00EA) = R8; nop; rts` (output word 0 of every call is 1.0). Generic
    /// encodings, no firmware bytes. Two DM words sit in the kept ranges.
    pub(in crate::organ) fn synthetic_sram() -> Vec<u8> {
        let mut sram = vec![0u8; ldr::SRAM_BYTES];
        let put = |sram: &mut Vec<u8>, at: usize, parcels: &[u16]| {
            for (k, p) in parcels.iter().enumerate() {
                sram[2 * at + 2 * k..2 * at + 2 * k + 2].copy_from_slice(&p.to_le_bytes());
            }
        };
        put(&mut sram, GEN.0 as usize, &[0x0ABE]);
        put(
            &mut sram,
            PROC.0 as usize,
            &[
                0x0F08, 0x3F80, 0x0000, // R8 = 0x3f800000
                0x1108, 0x000C, 0x00EA, // DM(0xc00ea) = R8
                0x0000, 0x0000, 0x0000, // NOP
                0x0ABE, // RTS
            ],
        );
        for (a, v) in [(0xB_33B8u32, 0x3F80_0000u32), (0x9_5000, 5)] {
            sram[4 * a as usize..4 * a as usize + 4].copy_from_slice(&v.to_le_bytes());
        }
        sram
    }

    /// The synthetic image with the start registers `i0` / `i1`.
    pub(in crate::organ) fn synthetic_image() -> BassImage {
        BassImage::from_parts(&synthetic_sram(), &[], "R,i0,c00ea\n", false).unwrap()
    }

    /// The image of the user's OS update file named by `NS4_WAVE_TABLES`, or `None` with a note
    /// (tests that need the real routines skip by absence, never `#[ignore]`).
    pub(in crate::organ) fn real_image() -> Option<Arc<BassImage>> {
        match from_env() {
            Ok(Some(i)) => Some(i),
            Ok(None) => {
                eprintln!("{WAVE_TABLES_ENV} unset: skipping a test that needs the OS update file");
                None
            }
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn the_synthetic_routines_decode_into_instruction_rows() {
        let img = synthetic_image();
        let ins = |n: i32| [0, n, 0, 0, 0, 0, 0, 0];
        assert!(img.gen.is_empty(), "the generator is an `rts`: no rows");
        assert_eq!(
            img.proc,
            vec![
                ins(1),
                [10, 8, 0x3F80_0000, 0, 0, 0, 0, 0],
                ins(1),
                [3, 8, 0, 0xC_00EA, 0, 0, 0, 0],
                ins(0),
            ]
        );
        assert_eq!(img.csv, "R,i0,c00ea\nM,95000,5\nM,b33b8,3f800000\n");
    }

    #[test]
    fn an_a_line_reads_words_from_the_code_and_the_delta_overrides_the_loader() {
        let mut code = vec![0u8; 0x40];
        code[0x20..0x28].copy_from_slice(&[9, 0, 0, 0, 7, 0, 0, 0]);
        let img =
            BassImage::from_parts(&synthetic_sram(), &code, "A,b33b9,2,20\nM,95000,0\n", false)
                .unwrap();
        assert_eq!(img.csv, "M,b33b8,3f800000 9 7\n");
    }

    #[test]
    fn the_layout_guard_rejects_a_foreign_image_and_the_unguarded_build_decodes_it() {
        let sram = synthetic_sram();
        let e = BassImage::from_parts(&sram, &[], "", true).unwrap_err();
        assert!(e.contains("unsupported Nord OS version"), "{e}");
        assert!(BassImage::from_parts(&sram, &[], "", false).is_ok());
    }

    #[test]
    fn an_unsupported_instruction_is_an_error_not_a_wrong_program() {
        // `BIT SET FLAGS 0x2` (18a: 1405 0000 0002) in place of the processor store
        let mut sram = synthetic_sram();
        let at = 2 * PROC.0 as usize + 6;
        for (k, p) in [0x1405u16, 0x0000, 0x0002].iter().enumerate() {
            sram[at + 2 * k..at + 2 * k + 2].copy_from_slice(&p.to_le_bytes());
        }
        let e = BassImage::from_parts(&sram, &[], "", false).unwrap_err();
        assert!(e.contains("cannot parse"), "{e}");
    }

    #[test]
    fn a_missing_file_and_an_unset_variable_are_reported() {
        let e = BassImage::from_file(Path::new("no/such/update.exe")).unwrap_err();
        assert!(e.contains("no/such/update.exe"), "{e}");
        assert_eq!(from_setting(None), Ok(None));
        assert_eq!(from_setting(Some("  ")), Ok(None));
        let e = from_setting(Some("no/such/other.exe")).unwrap_err();
        assert!(e.contains("no/such/other.exe"), "{e}");
    }

    #[test]
    fn the_embedded_delta_is_well_formed_and_the_keep_ranges_are_ordered() {
        // every line is an R, M or A line the loader accepts, with the real `bank0_CODE` absent
        // the A lines are the only ones allowed to fail
        let lines = DELTA
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty());
        assert!(lines
            .clone()
            .all(|l| matches!(l.as_bytes()[0], b'R' | b'M' | b'A')));
        assert!(lines.filter(|l| l.starts_with('A')).count() > 0);
        assert!(KEEP.windows(2).all(|w| w[0].0 + w[0].1 <= w[1].0));
        let e = dm_image_text(&synthetic_sram(), &[], &KEEP, DELTA).unwrap_err();
        assert!(e.contains("outside bank0_CODE"), "{e}");
    }

    /// VERIFIED: the image built from the v1.66 update file equals the former tracked
    /// `bass-fw-image.csv`, `prog_gen.rs` and `prog_proc.rs`. These are the FNV-1a 64 fingerprints
    /// of that content, taken with a comparison test (6,881 words and 555 / 898 rows equal, 0
    /// differing) before the files were removed. Needs the update file; skipped without it.
    #[test]
    fn the_real_update_file_builds_the_verified_image_and_rows() {
        let Some(img) = real_image() else { return };
        let fp = crate::firmware_setting::fnv1a64;
        let bytes = |rows: &[Row]| -> Vec<u8> {
            rows.iter()
                .flatten()
                .flat_map(|v| v.to_le_bytes())
                .collect()
        };
        assert_eq!((img.gen.len(), img.proc.len()), ROWS);
        assert_eq!(fp(img.csv.as_bytes()), IMAGE_FNV);
        assert_eq!(format!("{:016x}", fp(&bytes(&img.gen))), "adad5119d9710a40");
        assert_eq!(
            format!("{:016x}", fp(&bytes(&img.proc))),
            "466edb101536318f"
        );
    }
}
