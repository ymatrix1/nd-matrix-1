//! Finds the font table and the bar font in a `bank0_CODE` image of any OS version.
//!
//! The addresses of the two (`FONT_TABLE`, `BAR_FONT`) are those of OS v1.66; a later OS is a
//! recompile and holds the same objects a few hundred bytes later (v1.68: `+0x368` and `+0x398`,
//! `docs/research/os-1-68-vs-1-66.md`). The v1.66 address is tried first, then the image is
//! scanned for the layout, so the next update needs no code change.

use super::{parse_font, u32_at, Font, BAR_FONT, FONT_COUNT, FONT_TABLE, LOAD_BASE};

/// VERIFIED (v1.66 and v1.68 images, `docs/research/os-1-68-vs-1-66.md`): the bar font object is
/// 9 rows tall, covers the characters `0x20..=0x44` and has the mask word `0x1FF`.
const BAR_SIGNATURE: (u32, u8, u8, u32) = (9, 0x20, 0x44, 0x1FF);

fn in_image(code: &[u8], addr: u32) -> bool {
    addr >= LOAD_BASE && ((addr - LOAD_BASE) as usize) < code.len()
}

/// Cheap shape test of a font object header at `addr`: height 1..=32, `last >= first`, a table
/// pointer inside the image.
fn plausible_object(code: &[u8], addr: u32) -> bool {
    if !in_image(code, addr) {
        return false;
    }
    let off = (addr - LOAD_BASE) as usize;
    let (Ok(height), Ok(table), Some(range)) = (
        u32_at(code, off),
        u32_at(code, off + 8),
        code.get(off + 4..off + 6),
    ) else {
        return false;
    };
    (1..=32).contains(&height) && range[1] >= range[0] && in_image(code, table)
}

/// The 17 fonts of the table at `addr`, `Err` when any entry is not a font object.
fn table_at(code: &[u8], addr: u32) -> Result<Vec<Font>, String> {
    let base = addr.checked_sub(LOAD_BASE).ok_or("below the load base")? as usize;
    (0..FONT_COUNT)
        .map(|i| parse_font(code, u32_at(code, base + 4 * i)?))
        .collect()
}

/// The UI fonts: the table at the v1.66 address, else the first 4-aligned table of 17 font
/// objects found by scanning the image.
pub(super) fn find_table(code: &[u8]) -> Result<Vec<Font>, String> {
    let first_try = table_at(code, FONT_TABLE);
    if first_try.is_ok() {
        return first_try;
    }
    let last = code.len().saturating_sub(4 * FONT_COUNT);
    for off in (0..last).step_by(4) {
        // every entry must be a plausible object: test them all, then decode
        let all_plausible = (0..FONT_COUNT)
            .all(|i| u32_at(code, off + 4 * i).is_ok_and(|a| plausible_object(code, a)));
        if all_plausible {
            if let Ok(fonts) = table_at(code, LOAD_BASE + off as u32) {
                return Ok(fonts);
            }
        }
    }
    first_try
}

/// Whether the object at `addr` has the bar font signature.
fn is_bar(code: &[u8], addr: u32) -> bool {
    if !plausible_object(code, addr) {
        return false;
    }
    let off = (addr - LOAD_BASE) as usize;
    let (Ok(h), Ok(mask), Some(r)) = (
        u32_at(code, off),
        u32_at(code, off + 12),
        code.get(off + 4..off + 6),
    ) else {
        return false;
    };
    (h, r[0], r[1], mask) == BAR_SIGNATURE
}

/// The drawbar bar font: at the v1.66 address when it has the signature there, else the only
/// object of the image that has it (`None` when there is none or more than one).
pub(super) fn find_bar(code: &[u8]) -> Option<Font> {
    let addr = if is_bar(code, BAR_FONT) {
        BAR_FONT
    } else {
        let mut hits = (0..code.len().saturating_sub(16))
            .step_by(4)
            .map(|o| LOAD_BASE + o as u32)
            .filter(|&a| is_bar(code, a));
        let (one, two) = (hits.next()?, hits.next());
        if two.is_some() {
            return None;
        }
        one
    };
    parse_font(code, addr).ok()
}

#[cfg(test)]
mod tests {
    use super::super::tests::synthetic_code;
    use super::*;

    /// Moves the whole synthetic font table to `to` (the objects stay).
    fn moved_table(to: u32) -> Vec<u8> {
        let mut code = synthetic_code();
        let (from, n) = ((FONT_TABLE - LOAD_BASE) as usize, 4 * FONT_COUNT);
        let saved: Vec<u8> = code[from..from + n].to_vec();
        code[from..from + n].fill(0);
        let at = (to - LOAD_BASE) as usize;
        code[at..at + n].copy_from_slice(&saved);
        code
    }

    /// Writes a bar font object (the signature, the glyph table of the synthetic fonts) at `addr`.
    fn with_bar(mut code: Vec<u8>, addr: u32) -> Vec<u8> {
        let o = (addr - LOAD_BASE) as usize;
        code[o..o + 4].copy_from_slice(&9u32.to_le_bytes());
        code[o + 4] = 0x20;
        code[o + 5] = 0x44;
        // glyph table for 0x20..=0x44: every entry points at the glyph area with width 0
        let table = 0x7000usize;
        for k in 0..=(0x44 - 0x20) {
            let at = table + 8 * k;
            code[at..at + 4].copy_from_slice(&(LOAD_BASE + 0x6000).to_le_bytes());
        }
        code[o + 8..o + 12].copy_from_slice(&(LOAD_BASE + table as u32).to_le_bytes());
        code[o + 12..o + 16].copy_from_slice(&0x1FFu32.to_le_bytes());
        code
    }

    #[test]
    fn a_table_at_the_v166_address_is_taken_as_is() {
        assert_eq!(find_table(&synthetic_code()).unwrap().len(), FONT_COUNT);
    }

    #[test]
    fn a_moved_table_is_found_by_scanning() {
        let a = find_table(&synthetic_code()).unwrap();
        let b = find_table(&moved_table(FONT_TABLE + 0x368)).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn no_table_is_an_error() {
        assert!(find_table(&[0u8; 0x100]).is_err());
        let mut code = vec![0u8; 0x1000];
        code[0..4].copy_from_slice(&0x1000u32.to_le_bytes());
        assert!(find_table(&code).is_err());
    }

    #[test]
    fn the_bar_font_is_found_at_the_old_address_or_by_its_signature() {
        let code = synthetic_code();
        assert!(find_bar(&code).is_none(), "no object has the signature");
        let at_old = with_bar(code.clone(), BAR_FONT);
        assert!(is_bar(&at_old, BAR_FONT));
        assert_eq!(find_bar(&at_old).unwrap().height, 9);
        let elsewhere = with_bar(code, BAR_FONT + 0x398);
        assert!(!is_bar(&elsewhere, BAR_FONT));
        assert!(is_bar(&elsewhere, BAR_FONT + 0x398));
        assert_eq!(find_bar(&elsewhere).unwrap().last, 0x44);
    }

    #[test]
    fn two_bar_candidates_are_not_guessed() {
        let code = with_bar(
            with_bar(synthetic_code(), BAR_FONT + 0x100),
            BAR_FONT + 0x200,
        );
        assert!(find_bar(&code).is_none());
    }

    /// VERIFIED (run, `NS4_WAVE_TABLES` = the OS v1.66 and the v1.68 update file): 17 UI fonts of
    /// heights 13, 13, 10, 8 first and the bar font of height 9. Skips without the file.
    #[test]
    fn the_real_update_file_gives_the_fonts_in_any_version() {
        let Ok(path) = std::env::var(crate::render_opts_wave::WAVE_TABLES_ENV) else {
            eprintln!("NS4_WAVE_TABLES unset: skipping a test that needs the OS update file");
            return;
        };
        let code = crate::dsp_image::bank0_code(std::path::Path::new(path.trim())).unwrap();
        let fonts = find_table(&code).unwrap();
        let heights: Vec<u32> = fonts.iter().map(|f| f.height).collect();
        assert_eq!(&heights[..4], &[13, 13, 10, 8]);
        assert_eq!(heights.len(), FONT_COUNT);
        assert_eq!(find_bar(&code).unwrap().height, 9);
    }
}
