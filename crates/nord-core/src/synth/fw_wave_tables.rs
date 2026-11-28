//! The 46 WAVE tables, read at run time from the user's own OS image.
//!
//! Evidence `docs/research/synth-stage-models-13.md` s2; registry census
//! `tools/synth_wave_registry.py` (`docs/research/data/synth-wave-registry.csv`, addresses and
//! counts only). The table samples are firmware rodata and are never committed (D22): the user
//! passes a `bank0_CODE` image or the OS update file (`--synth-wave-tables PATH`).
//!
//! Registry: 28-byte records at `0xC02FD4B0` (`name, rows_end, 8, rows, 512, data, count`); record
//! `k` is the WAVE table id `k`. The DSP layout (VERIFIED run: the rebuilt record 0 equals the ARM
//! boot copy, 1970 of 1970 samples and 512 of 512 row words) re-lays the unique bands in ascending
//! source order with a stride of `count + 12` and appends the first 12 samples of each band
//! cyclically, so the FIR never reads past a cycle end.

use super::fw_super2::Super24Tables;
use crate::firmware::FirmwareContainer;

const LOAD_BASE: u32 = 0xC000_0000;
/// VERIFIED (v1.66 `bank0_CODE`, `tools/synth_wave_registry.py`): where the registry sits in OS
/// v1.66. OS v1.68 has it at `0xC02FD850` (+0x3A0, `docs/research/os-1-68-vs-1-66.md`). Only a
/// first guess now: `find_registry` scans for the layout when the guess is wrong.
const REGISTRY_ADDR: u32 = 0xC02F_D4B0;
const RECORD_BYTES: usize = 28;
const ROWS_WORDS: usize = 0x200;
const BANDS: usize = 128;
/// VERIFIED (run): cyclic samples appended to every band.
const GUARD: usize = 12;
/// ASSUMED beyond record 0 (VERIFIED there): the last band's guard is two samples short.
const TAIL_TRIM: usize = 2;
/// Number of WAVE tables (table ids `0..46`, `docs/research/data/synth-wave-categories.csv`).
pub const WAVE_TABLES: usize = 46;

fn word(code: &[u8], addr: u32) -> Result<u32, String> {
    let off = addr
        .checked_sub(LOAD_BASE)
        .ok_or("address below the code base")? as usize;
    let b = code
        .get(off..off + 4)
        .ok_or("address beyond the code image")?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// DSP layout `(rows, words)` of record `k`; `boot` supplies the FIR and biquad tables.
fn build(
    code: &[u8],
    registry: u32,
    k: usize,
    boot: &Super24Tables,
) -> Result<Super24Tables, String> {
    let rec = registry + (RECORD_BYTES * k) as u32;
    let f = |j: u32| word(code, rec + 4 * j);
    let (end, eight, rows_ptr, n, data_ptr, count) = (f(1)?, f(2)?, f(3)?, f(4)?, f(5)?, f(6)?);
    if eight != 8 || n as usize != ROWS_WORDS || end != rows_ptr + 4 * ROWS_WORDS as u32 {
        return Err(format!("registry record {k} has a foreign layout"));
    }
    let rows_in: Vec<u32> = (0..ROWS_WORDS as u32)
        .map(|i| word(code, rows_ptr + 4 * i))
        .collect::<Result<_, _>>()?;
    let off = (data_ptr - LOAD_BASE) as usize;
    let raw = code
        .get(off..off + 2 * (count as usize + 1))
        .ok_or("table beyond the code image")?;
    let src: Vec<u16> = raw
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let mut bands: Vec<(u32, u32)> = (0..BANDS)
        .map(|b| (rows_in[4 * b], rows_in[4 * b + 1]))
        .collect();
    bands.sort_unstable();
    bands.dedup();
    let mut shorts: Vec<u16> = Vec::new();
    let mut starts = Vec::with_capacity(bands.len());
    for &(o, c) in &bands {
        let (o, c) = (o as usize, c as usize);
        let band = src.get(o..o + c).filter(|b| !b.is_empty());
        let band = band.ok_or_else(|| format!("record {k}: band beyond the table"))?;
        starts.push(shorts.len() as u32);
        shorts.extend((0..c + GUARD).map(|i| band[i % c]));
    }
    shorts.truncate(shorts.len().saturating_sub(TAIL_TRIM));
    let mut rows = rows_in.clone();
    for b in 0..BANDS {
        let key = (rows_in[4 * b], rows_in[4 * b + 1]);
        let at = bands.binary_search(&key).map_err(|_| "band lookup")?;
        rows[4 * b] = starts[at];
    }
    let wave = shorts
        .chunks(2)
        .map(|c| u32::from(c[0]) | (u32::from(*c.get(1).unwrap_or(&0)) << 16))
        .collect();
    Ok(Super24Tables {
        rows,
        wave,
        fir: boot.fir.clone(),
        lattice: boot.lattice,
    })
}

/// True when record `k` at `registry` has the registry layout (`end == rows + 4 * 512`, `8`, `512`)
/// and its pointers fall inside the image.
fn record_ok(code: &[u8], registry: u32, k: usize) -> bool {
    let rec = registry + (RECORD_BYTES * k) as u32;
    let f = |j: u32| word(code, rec + 4 * j).ok();
    let (Some(end), Some(eight), Some(rows), Some(n), Some(data)) = (f(1), f(2), f(3), f(4), f(5))
    else {
        return false;
    };
    let top = LOAD_BASE.saturating_add(code.len() as u32);
    eight == 8
        && n as usize == ROWS_WORDS
        && end == rows.wrapping_add(4 * ROWS_WORDS as u32)
        && (LOAD_BASE..top).contains(&rows)
        && (LOAD_BASE..top).contains(&data)
}

/// Address of record 0 of the registry: the v1.66 address when it still validates, else the first
/// 4-aligned address where all `WAVE_TABLES` records validate (runtime discovery, so the next OS
/// update needs no code change).
fn find_registry(code: &[u8]) -> Result<u32, String> {
    let all_ok = |at: u32| (0..WAVE_TABLES).all(|k| record_ok(code, at, k));
    if all_ok(REGISTRY_ADDR) {
        return Ok(REGISTRY_ADDR);
    }
    let last = code.len().saturating_sub(RECORD_BYTES * WAVE_TABLES);
    (0..last)
        .step_by(4)
        .map(|o| LOAD_BASE + o as u32)
        .find(|&at| record_ok(code, at, 0) && all_ok(at))
        .ok_or_else(|| "registry record 0 has a foreign layout".to_string())
}

/// All 46 tables from a `bank0_CODE` image.
pub fn tables_from_code(code: &[u8], boot: &Super24Tables) -> Result<Vec<Super24Tables>, String> {
    let registry = find_registry(code)?;
    (0..WAVE_TABLES)
        .map(|k| build(code, registry, k, boot))
        .collect()
}

/// All 46 tables from a file: the official OS update (`.exe`, `.dmg`, or a `.zip` holding one),
/// the extracted OS container, or a raw `bank0_CODE` image (D129). Read at run time, never stored.
pub fn tables_from_file(
    path: &std::path::Path,
    boot: &Super24Tables,
) -> Result<Vec<Super24Tables>, String> {
    let raw = crate::os_update::read_capped(path)?;
    let code = match crate::os_update::extract_firmware_container(&raw) {
        Ok(container) => {
            let fw = FirmwareContainer::parse(&container)?;
            fw.extract_segment(0, "CODE", &container)?
        }
        Err(why) => {
            // not an update file: accept a raw bank0_CODE image, else explain what was expected
            return tables_from_code(&raw, boot).map_err(|_| {
                format!(
                    "{}: {why}; it is not a bank0_CODE image either",
                    path.display()
                )
            });
        }
    };
    tables_from_code(&code, boot).map_err(|e| {
        format!(
            "{}: unsupported Nord OS version, WAVE registry not found ({e})",
            path.display()
        )
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn boot() -> Super24Tables {
        Super24Tables {
            rows: vec![],
            wave: vec![],
            fir: vec![1, 2, 3],
            lattice: [0.5; 5],
        }
    }

    /// A code image whose registry holds `WAVE_TABLES` copies of one two-band table.
    pub(crate) fn image() -> Vec<u8> {
        image_at(REGISTRY_ADDR)
    }

    /// Same image with the registry at `registry`.
    pub(crate) fn image_at(registry: u32) -> Vec<u8> {
        let mut img = vec![0u8; 0x40_0000];
        let put = |img: &mut Vec<u8>, addr: u32, w: u32| {
            let o = (addr - LOAD_BASE) as usize;
            img[o..o + 4].copy_from_slice(&w.to_le_bytes());
        };
        let (rows_at, data_at) = (0xC030_0000u32, 0xC031_0000u32);
        for (i, v) in [10u16, 20, 30, 40, 1, 2, 3, 4, 5, 6].iter().enumerate() {
            let o = (data_at - LOAD_BASE) as usize + 2 * i;
            img[o..o + 2].copy_from_slice(&v.to_le_bytes());
        }
        for b in 0..BANDS as u32 {
            let (o, c) = if b % 2 == 0 { (0, 4) } else { (4, 6) };
            for (j, v) in [o, c, 7, 1].iter().enumerate() {
                put(&mut img, rows_at + 16 * b + 4 * j as u32, *v);
            }
        }
        for k in 0..WAVE_TABLES as u32 {
            let rec = registry + 28 * k;
            let vals = [0, rows_at + 0x800, 8, rows_at, 0x200, data_at, 9];
            for (j, v) in vals.iter().enumerate() {
                put(&mut img, rec + 4 * j as u32, *v);
            }
        }
        img
    }

    #[test]
    fn the_layout_relays_bands_with_a_cyclic_guard_and_new_offsets() {
        let t = tables_from_code(&image(), &boot()).unwrap();
        assert_eq!(t.len(), WAVE_TABLES);
        let tab = &t[3];
        assert_eq!((tab.rows[0], tab.rows[1]), (0, 4));
        assert_eq!((tab.rows[4], tab.rows[5]), (16, 6));
        assert_eq!((tab.rows[2], tab.rows[3]), (7, 1), "spare words stay");
        let shorts: Vec<u16> = tab
            .wave
            .iter()
            .flat_map(|w| [*w as u16, (*w >> 16) as u16])
            .collect();
        assert_eq!(&shorts[..16], &[10, 20, 30, 40].repeat(4)[..]);
        assert_eq!(&shorts[16..22], &[1, 2, 3, 4, 5, 6]);
        assert_eq!(shorts.len(), 16 + 6 + GUARD - TAIL_TRIM);
        assert_eq!(tab.fir, vec![1, 2, 3]);
    }

    #[test]
    fn a_foreign_registry_or_a_short_image_is_an_error() {
        let mut img = image();
        let o = (REGISTRY_ADDR - LOAD_BASE) as usize + 8;
        img[o] = 9;
        assert!(tables_from_code(&img, &boot()).is_err());
        assert!(tables_from_code(&[0u8; 16], &boot()).is_err());
    }

    #[test]
    fn a_registry_at_another_address_is_found_by_its_layout() {
        let a = tables_from_code(&image(), &boot()).unwrap();
        for moved in [0xC02F_D850u32, 0xC02F_D4B4, 0xC033_0000] {
            let b = tables_from_code(&image_at(moved), &boot()).unwrap();
            assert_eq!(a.len(), b.len());
            assert_eq!(a[5].rows, b[5].rows);
            assert_eq!(a[5].wave, b[5].wave);
        }
    }

    #[test]
    fn a_missing_file_names_the_path() {
        let e = tables_from_file(std::path::Path::new("no/such/fw.bin"), &boot()).unwrap_err();
        assert!(e.contains("no/such/fw.bin"), "{e}");
    }
}
