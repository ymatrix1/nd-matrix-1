//! The Farfisa processor image, built at setup time from the user's own OS update file (D128).
//!
//! The firmware PNO image (`bank0_CODE`, download index 1, core 1) holds the routine `0x122B85`
//! and the initial DM of the machine. Three parts make the machine of [`super::FarfisaFw`]:
//!
//! | Part | Source |
//! |---|---|
//! | instruction rows of `0x122B85..0x122EF5` | the core 1 listing, [`super::rows`] |
//! | DM words of the five probed ranges | core 1 loader data at byte `4 * address`, [`crate::dsp_image`] |
//! | model 2 install words, start registers | [`DELTA`], our own capture of the ARM install (48 words) |
//!
//! Nothing of the firmware is stored: the file is read when the stage is switched on and dropped.
//! Without it the stage does not exist and the layer plays its non-firmware Farfisa model (D22).
//!
//! VERIFIED: the image built from the v1.66 update file equals the former tracked
//! `farfisa-fw-image.csv` (802 DM words, 10 registers) and `prog_proc.rs` (957 rows) exactly,
//! 0 differing words, 0 differing rows. The fingerprints of that content are pinned in
//! `the_real_update_file_builds_the_verified_image_and_rows` (run it with `NS4_WAVE_TABLES` set).

use super::rows::routine_rows;
use crate::dsp_image::{self, ldr};
use crate::organ::b3_bass_fw::Row;
use crate::render_opts_wave::WAVE_TABLES_ENV;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Routine `0x122B85..0x122EF5` of PNO core 1 (short word addresses).
///
/// VERIFIED: bounds of the processor = `docs/research/organ-farfisa-bass.md` s16 (`PROC_SPEC` of
/// `tools/organ_farfisa_fw.py`), checked by `the_real_update_file_builds_the_verified_image_and_rows`
/// (the 957 rows decoded from this range equal the former `prog_proc.rs`, 0 differing).
pub(super) const ENTRY: u32 = 0x12_2B85;
pub(super) const END: u32 = 0x12_2EF5;
/// DM ranges the routine reads or writes `(base, words)`.
///
/// VERIFIED: probe of the processor, `KEEP` of `tools/organ_farfisa_fw.py`, s16; the 802 non zero
/// words built from them equal the former `farfisa-fw-image.csv`, same test, 0 differing.
const KEEP: [(u32, u32); 5] = [
    (0x9_5FFC, 0x178),
    (0x9_65DC, 0x10),
    (0xB_472C, 8),
    (0xC_27E0, 0x640),
    (0xC_4000, 8),
];
/// Install words and start registers the ARM and the oracle boot add to the loader data
/// (`R,name,hex` and `M,addr,hex ...` lines; `tools/organ_farfisa_fw.py image`).
///
/// VERIFIED: 46 words, 2 clears and 10 registers = diff of the former `farfisa-fw-image.csv` (an
/// oracle dump after the boot log and the 526 install words of model 2) against the loader data,
/// checked by the same test and by the bit exact goldens in `farfisa_fw::tests` (needs the file).
const DELTA: &str = include_str!("../../../../../docs/research/data/farfisa-fw-install-delta.csv");
/// Layout guard: rows of the v1.66 routine and non zero DM words of the image. Another OS
/// version that moved the routine fails here instead of running a wrong program.
///
/// VERIFIED: 957 rows and 802 words of the v1.66 update file (the PNO image is byte identical in
/// v1.60 to v1.66, `dsp-image-hunt.md` 5.13); other versions are not measured (ASSUMED to differ).
const ROWS: usize = 957;
const WORDS: usize = 802;

/// A built image: DM image text (`R,` and `M,` lines of `Machine::load_image`) and the rows.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::organ) struct FarfisaImage {
    pub(in crate::organ) csv: String,
    pub(in crate::organ) rows: Vec<Row>,
}

/// DM image lines of `sram` plus the install `delta`: registers first, then runs of non zero
/// words (consecutive addresses merged), the format of `tools/organ_bass_fw_gen.py mem_runs`.
fn image_text(sram: &[u8], delta: &str) -> Result<(String, usize), String> {
    let mut words: BTreeMap<u32, u32> = BTreeMap::new();
    for (base, n) in KEEP {
        for a in base..base + n {
            words.insert(a, ldr::dm_word(sram, a));
        }
    }
    let mut lines: Vec<String> = Vec::new();
    for line in delta
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let f: Vec<&str> = line.split(',').collect();
        match f.as_slice() {
            ["R", ..] => lines.push(line.to_string()),
            ["M", addr, vals] => {
                let a = u32::from_str_radix(addr, 16).map_err(|_| format!("bad delta {line:?}"))?;
                for (k, v) in vals.split(' ').enumerate() {
                    let v =
                        u32::from_str_radix(v, 16).map_err(|_| format!("bad delta {line:?}"))?;
                    words.insert(a + k as u32, v);
                }
            }
            _ => return Err(format!("bad delta line {line:?}")),
        }
    }
    let nonzero: Vec<(u32, u32)> = words.into_iter().filter(|&(_, v)| v != 0).collect();
    let count = nonzero.len();
    let mut run: Vec<u32> = Vec::new();
    let mut start = 0u32;
    for (a, v) in nonzero {
        if !run.is_empty() && a != start + run.len() as u32 {
            lines.push(run_line(start, &run));
            run.clear();
        }
        if run.is_empty() {
            start = a;
        }
        run.push(v);
    }
    if !run.is_empty() {
        lines.push(run_line(start, &run));
    }
    Ok((lines.join("\n") + "\n", count))
}

fn run_line(start: u32, run: &[u32]) -> String {
    let words: Vec<String> = run.iter().map(|w| format!("{w:x}")).collect();
    format!("M,{start:x},{}", words.join(" "))
}

impl FarfisaImage {
    /// The image of a flat core 1 SRAM image and the install `delta`; `guard` applies the layout
    /// check of the v1.66 routine (tests with synthetic images switch it off).
    pub(super) fn from_sram(sram: &[u8], delta: &str, guard: bool) -> Result<Self, String> {
        let rows = routine_rows(sram, ENTRY, END)?;
        if !crate::organ::b3_bass_fw::rows_valid(&rows) {
            return Err("the Farfisa routine does not decode into valid rows".into());
        }
        let (csv, words) = image_text(sram, delta)?;
        if guard && (rows.len() != ROWS || words != WORDS) {
            return Err(format!(
                "unsupported Nord OS version: the Farfisa routine has {} rows and {} DM words, \
                 expected {ROWS} and {WORDS} (OS v1.60 to v1.66)",
                rows.len(),
                words
            ));
        }
        Ok(Self { csv, rows })
    }

    /// The image of the user's OS update file `path` (`.exe`, `.dmg`, `.zip` or the container).
    pub(super) fn from_file(path: &Path) -> Result<Self, String> {
        let code = dsp_image::bank0_code(path)?;
        let pno = dsp_image::image_of(&code, dsp_image::PNO)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let sram = ldr::core_sram(pno, 0).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_sram(&sram, DELTA, true).map_err(|e| format!("{}: {e}", path.display()))
    }
}

type Cached = Option<(PathBuf, Result<Arc<FarfisaImage>, String>)>;
static CACHE: Mutex<Cached> = Mutex::new(None);

/// The image named by `NS4_WAVE_TABLES` (the OS update file, also filled from the saved app
/// setting at start-up, D127 / D129): `Ok(None)` when it is unset, an error when it cannot be read.
/// Read once per path and kept (setup time only).
pub(in crate::organ) fn from_env() -> Result<Option<Arc<FarfisaImage>>, String> {
    from_setting(std::env::var(WAVE_TABLES_ENV).ok().as_deref())
}

/// [`from_env`] for an explicit value of the variable (`None` or blank: no file named).
fn from_setting(value: Option<&str>) -> Result<Option<Arc<FarfisaImage>>, String> {
    let Some(path) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let path = PathBuf::from(path);
    let mut cache = CACHE
        .lock()
        .map_err(|_| "image cache poisoned".to_string())?;
    if let Some((p, r)) = cache.as_ref() {
        if *p == path {
            return r.clone().map(Some);
        }
    }
    let r = FarfisaImage::from_file(&path).map(Arc::new);
    *cache = Some((path, r.clone()));
    r.map(Some)
}

/// Whether the Farfisa firmware stage can be built: `Ok(true)` when `NS4_WAVE_TABLES` names an OS
/// update file that loads, `Ok(false)` when it is unset (the stage stays off and the layer plays
/// its non-firmware model, D22), `Err` with the reason when a file is named but cannot be used.
pub fn firmware_status() -> Result<bool, String> {
    from_env().map(|i| i.is_some())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Flat SRAM image with a tiny routine at `ENTRY` (`R8 = 1.0; DM(0xC4000) = R8; nop; rts`)
    /// and two DM words in the kept ranges. Generic encodings, no firmware bytes.
    pub(super) fn synthetic_sram() -> Vec<u8> {
        let mut sram = vec![0u8; ldr::SRAM_BYTES];
        let at = 2 * ENTRY as usize;
        let parcels = [
            0x0F08u16, 0x3F80, 0x0000, // R8 = 0x3f800000
            0x1108, 0x000C, 0x4000, // DM(0xc4000) = R8
            0x0000, 0x0000, 0x0000, // NOP
            0x0ABE, // RTS
        ];
        for (k, p) in parcels.iter().enumerate() {
            sram[at + 2 * k..at + 2 * k + 2].copy_from_slice(&p.to_le_bytes());
        }
        for (a, v) in [(0xC_27E0u32, 0x3F80_0000u32), (0x9_5FFC, 5)] {
            sram[4 * a as usize..4 * a as usize + 4].copy_from_slice(&v.to_le_bytes());
        }
        sram
    }

    /// The synthetic image with one start register.
    pub(in crate::organ) fn synthetic_image() -> FarfisaImage {
        FarfisaImage::from_sram(&synthetic_sram(), "R,i0,c00ea\n", false).unwrap()
    }

    /// The image of the user's OS update file named by `NS4_WAVE_TABLES`, or `None` with a note
    /// (tests that need the real routine skip by absence, never `#[ignore]`).
    pub(in crate::organ) fn real_image() -> Option<Arc<FarfisaImage>> {
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
    fn the_synthetic_routine_decodes_into_instruction_rows() {
        let img = synthetic_image();
        let ins = |n: i32| [0, n, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            img.rows,
            vec![
                ins(1),
                [10, 8, 0x3F80_0000, 0, 0, 0, 0, 0],
                ins(1),
                [3, 8, 0, 0xC_4000, 0, 0, 0, 0],
                ins(0),
            ]
        );
        assert_eq!(img.csv, "R,i0,c00ea\nM,95ffc,5\nM,c27e0,3f800000\n");
    }

    #[test]
    fn the_dm_text_merges_runs_applies_the_delta_and_drops_zero_words() {
        let sram = synthetic_sram();
        let delta = "# c\nR,i0,c00ea\nM,c27e1,4 5\nM,9,0\n";
        // the zero word at 0x9 is outside the kept ranges but still a (zero) word: dropped
        let (text, words) = image_text(&sram, delta).unwrap();
        assert_eq!(words, 4);
        assert_eq!(text, "R,i0,c00ea\nM,95ffc,5\nM,c27e0,3f800000 4 5\n");
    }

    #[test]
    fn a_delta_zero_clears_a_loader_word() {
        let (text, _) = image_text(&synthetic_sram(), "M,c27e0,0\n").unwrap();
        assert_eq!(text, "M,95ffc,5\n");
    }

    #[test]
    fn a_malformed_delta_is_an_error() {
        assert!(image_text(&synthetic_sram(), "X,1,2\n").is_err());
        assert!(image_text(&synthetic_sram(), "M,zz,1\n").is_err());
        assert!(image_text(&synthetic_sram(), "M,10,gg\n").is_err());
    }

    #[test]
    fn the_layout_guard_rejects_a_foreign_routine_and_the_unguarded_build_decodes_it() {
        let sram = synthetic_sram();
        let e = FarfisaImage::from_sram(&sram, "", true).unwrap_err();
        assert!(e.contains("unsupported Nord OS version"), "{e}");
        assert!(FarfisaImage::from_sram(&sram, "", false).is_ok());
    }

    #[test]
    fn an_unsupported_instruction_is_an_error_not_a_wrong_program() {
        // `BIT SET FLAGS 0x2` (18a: 1405 0000 0002) in place of the store
        let mut sram = synthetic_sram();
        let at = 2 * ENTRY as usize + 6;
        for (k, p) in [0x1405u16, 0x0000, 0x0002].iter().enumerate() {
            sram[at + 2 * k..at + 2 * k + 2].copy_from_slice(&p.to_le_bytes());
        }
        let e = FarfisaImage::from_sram(&sram, "", false).unwrap_err();
        assert!(e.contains("cannot parse"), "{e}");
    }

    #[test]
    fn a_missing_file_and_an_unset_variable_are_reported() {
        let e = FarfisaImage::from_file(Path::new("no/such/update.exe")).unwrap_err();
        assert!(e.contains("no/such/update.exe"), "{e}");
        assert_eq!(from_setting(None), Ok(None));
        assert_eq!(from_setting(Some("  ")), Ok(None));
        let e = from_setting(Some("no/such/other.exe")).unwrap_err();
        assert!(e.contains("no/such/other.exe"), "{e}");
    }

    /// VERIFIED: the image built from the v1.66 update file equals the former tracked
    /// `farfisa-fw-image.csv` and `prog_proc.rs`. These are the FNV-1a 64 fingerprints of that
    /// content, taken with a comparison test (113 DM text lines and 957 rows equal, 0 differing)
    /// before the files were removed. Needs the update file; skipped with a note without it.
    #[test]
    fn the_real_update_file_builds_the_verified_image_and_rows() {
        let Some(img) = real_image() else { return };
        let fp = crate::firmware_setting::fnv1a64;
        let rows: Vec<u8> = img
            .rows
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(img.rows.len(), ROWS);
        assert_eq!(img.csv.lines().count(), 113);
        assert_eq!(
            format!("{:016x}", fp(img.csv.as_bytes())),
            "27b741fb95b4fa3d"
        );
        assert_eq!(format!("{:016x}", fp(&rows)), "7645b555ad81bd82");
    }
}
