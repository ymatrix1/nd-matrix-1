//! The firmware's OLED bitmap fonts, decoded at run time from the user's own `bank0_CODE` image.
//!
//! D128: the font bitmaps are firmware data (class C), so nothing is stored in the repository. The
//! GUI reads them from the Nord OS update file the user set (`NS4_WAVE_TABLES` / saved setting) and
//! falls back to its own painter when there is none. Layout of the objects and the 18 font
//! validation: `docs/research/arm-oled-render.md` section 3.

mod install;

pub use install::{font, install_from_env, install_from_path, installed};

/// Load address of `bank0_CODE`.
///
/// VERIFIED: `bank0_CODE` load base `0xC0000000`, `docs/research/arm-oled-render.md` header; the
/// pointer fields of the font objects are absolute addresses inside it.
pub const LOAD_BASE: u32 = 0xC000_0000;
/// Address of the array of UI font pointers.
///
/// VERIFIED: `FONT_TABLE = 0xC029E5AC`, 17 entries, `tools/arm_oled.py` `font_table`
/// (`python tools/arm_panel.py fonts target/fw_unpacked/bank0_CODE.bin`), 17 of 17 decode.
pub const FONT_TABLE: u32 = 0xC029_E5AC;
/// Number of entries of [`FONT_TABLE`] (VERIFIED, same command).
pub const FONT_COUNT: usize = 17;
/// Address of the drawbar bar font (9 px tall; its characters `0`..`8` are the brown / black
/// drawbars at heights 1..9, `9`..`A` the white ones; `88AA09009` = `8 8 8 8 0 0 0 0 0`).
///
/// VERIFIED: font object at `0xC02C3BA8`, 9 rows, glyph columns decoded by
/// `tools/arm_oled.parse_font`; the string `88AA09009` is what the home screen gave its drawbar
/// text widget (`tools/arm_screens.py press ... --key 0x01,0,3`). ASSUMED: the colour assignment
/// of the characters (brown / black hollow, white solid) from the glyph shapes.
pub const BAR_FONT: u32 = 0xC02C_3BA8;
/// Index of the bar font in [`font`] (after the [`FONT_COUNT`] UI fonts).
pub const BAR_FONT_INDEX: usize = FONT_COUNT;
/// Display width in pixels (VERIFIED: SSD1309 init `A8 3F`, 128 columns, `arm-oled-render.md` s2.3).
pub const WIDTH: usize = 128;
/// Display height in pixels (same source).
pub const HEIGHT: usize = 64;

/// One firmware font: column bitmaps for the characters `first..=last`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Font {
    /// Glyph height in pixels (rows).
    pub height: u32,
    /// First character code.
    pub first: u8,
    /// Last character code.
    pub last: u8,
    /// Per character the column words: bit `i` = row `i` from the top, `1` = lit.
    pub glyphs: Vec<Vec<u32>>,
}

impl Font {
    /// Whether the font has a glyph for `c`.
    pub fn has(&self, c: u32) -> bool {
        c >= self.first as u32 && c <= self.last as u32
    }

    /// Column words of `c`; a character outside the range draws the first glyph (the space).
    pub fn columns(&self, c: u32) -> &[u32] {
        let i = if self.has(c) {
            (c - self.first as u32) as usize
        } else {
            0
        };
        self.glyphs.get(i).map_or(&[], Vec::as_slice)
    }

    /// Advance of `c` in pixels (the gap column is part of the glyph).
    pub fn advance(&self, c: u32) -> usize {
        self.columns(c).len()
    }

    /// Pixel width of the first line of `text`.
    ///
    /// VERIFIED: sum of the advances minus 1 when the last glyph's final column is empty, equal to
    /// `TEXT_MEASURE` `0xC001B9D0` on all 18 fonts (`arm_panel.py oracle`, "all equal").
    pub fn text_width(&self, text: &str) -> usize {
        let chars: Vec<u32> = text
            .chars()
            .take_while(|c| *c != '\0' && *c != '\n')
            .map(|c| c as u32)
            .collect();
        let Some(&last) = chars.last() else { return 0 };
        let total: usize = chars
            .iter()
            .filter(|c| self.has(**c))
            .map(|c| self.advance(*c))
            .sum();
        let trailing_gap = self.has(last) && self.columns(last).last() == Some(&0);
        total - usize::from(trailing_gap)
    }

    /// Calls `lit(x, y)` for every lit pixel of `text` with its top-left at `(x0, y0)`; returns the
    /// x after the text. Same geometry as the firmware blitter `0xC004064C` (VERIFIED, same
    /// command as [`Font::text_width`]).
    pub fn for_each_pixel(
        &self,
        text: &str,
        x0: i32,
        y0: i32,
        mut lit: impl FnMut(i32, i32),
    ) -> i32 {
        let mut x = x0;
        for c in text.chars().take_while(|c| *c != '\0' && *c != '\n') {
            for col in self.columns(c as u32) {
                for row in 0..self.height.min(32) {
                    if (col >> row) & 1 == 1 {
                        lit(x, y0 + row as i32);
                    }
                }
                x += 1;
            }
        }
        x
    }
}

fn u32_at(code: &[u8], off: usize) -> Result<u32, String> {
    code.get(off..off + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| format!("font data read past the image at {off:#x}"))
}

fn offset(addr: u32) -> Result<usize, String> {
    addr.checked_sub(LOAD_BASE)
        .map(|o| o as usize)
        .ok_or_else(|| format!("address {addr:#x} is below the load base"))
}

/// Decodes the font object at `addr` (image address, `0xC0......`) of `code`.
///
/// Object: `{height u32, first u8, last u8, pad u16, table u32, mask u32}`; `table` holds
/// `last - first + 1` entries `{ptr, width}`, a glyph is `width` u32 column words.
/// VERIFIED: `arm-oled-render.md` s3, `tools/arm_oled.py` `parse_font`.
pub fn parse_font(code: &[u8], addr: u32) -> Result<Font, String> {
    let off = offset(addr)?;
    let height = u32_at(code, off)?;
    let (first, last) = match code.get(off + 4..off + 6) {
        Some(b) => (b[0], b[1]),
        None => return Err("font object past the image".into()),
    };
    if !(1..=32).contains(&height) || last < first {
        return Err(format!("not a font object at {addr:#x}"));
    }
    let table = offset(u32_at(code, off + 8)?)?;
    let mut glyphs = Vec::with_capacity((last - first) as usize + 1);
    for i in 0..=(last - first) as usize {
        let ptr = offset(u32_at(code, table + 8 * i)?)?;
        let width = u32_at(code, table + 8 * i + 4)? as usize;
        if width > 8192 {
            return Err(format!(
                "glyph {i} of the font at {addr:#x} is {width} columns wide"
            ));
        }
        glyphs.push(
            (0..width)
                .map(|k| u32_at(code, ptr + 4 * k))
                .collect::<Result<_, _>>()?,
        );
    }
    Ok(Font {
        height,
        first,
        last,
        glyphs,
    })
}

/// The 17 UI fonts of `code` (index 0 = 13 px, index 3 = 8 px, see `arm-oled-render.md` s3).
pub fn font_table(code: &[u8]) -> Result<Vec<Font>, String> {
    let base = offset(FONT_TABLE)?;
    (0..FONT_COUNT)
        .map(|i| parse_font(code, u32_at(code, base + 4 * i)?))
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A code image with a font table of `FONT_COUNT` fonts: font `k` has height `8 + k`, the
    /// characters `0x41..=0x42` and glyphs `A` = `[0x7F, 0x09, 0x00]` (3 wide, trailing gap) and
    /// `B` = `[0x7F, 0x49]`.
    pub(crate) fn synthetic_code() -> Vec<u8> {
        let mut code = vec![0u8; 0x30_0000];
        let put = |code: &mut Vec<u8>, addr: u32, v: u32| {
            let o = (addr - LOAD_BASE) as usize;
            code[o..o + 4].copy_from_slice(&v.to_le_bytes());
        };
        let glyph_a = LOAD_BASE + 0x6000;
        let glyph_b = LOAD_BASE + 0x6010;
        for (k, w) in [0x7Fu32, 0x09, 0x00].into_iter().enumerate() {
            put(&mut code, glyph_a + 4 * k as u32, w);
        }
        for (k, w) in [0x7Fu32, 0x49].into_iter().enumerate() {
            put(&mut code, glyph_b + 4 * k as u32, w);
        }
        for k in 0..FONT_COUNT as u32 {
            let obj = LOAD_BASE + 0x4000 + 0x10 * k;
            let glyph_table = LOAD_BASE + 0x5000 + 0x20 * k;
            put(&mut code, FONT_TABLE + 4 * k, obj);
            put(&mut code, obj, 8 + k);
            let o = (obj - LOAD_BASE) as usize;
            code[o + 4] = 0x41;
            code[o + 5] = 0x42;
            put(&mut code, obj + 8, glyph_table);
            put(&mut code, glyph_table, glyph_a);
            put(&mut code, glyph_table + 4, 3);
            put(&mut code, glyph_table + 8, glyph_b);
            put(&mut code, glyph_table + 12, 2);
        }
        code
    }

    #[test]
    fn the_table_decodes_every_font_with_its_glyphs() {
        let fonts = font_table(&synthetic_code()).unwrap();
        assert_eq!(fonts.len(), FONT_COUNT);
        assert_eq!(fonts[0].height, 8);
        assert_eq!(fonts[3].height, 11);
        assert_eq!(
            fonts[0].glyphs,
            vec![vec![0x7F, 0x09, 0x00], vec![0x7F, 0x49]]
        );
    }

    #[test]
    fn width_drops_the_trailing_gap_and_skips_missing_glyphs() {
        let f = &font_table(&synthetic_code()).unwrap()[0];
        assert_eq!(f.text_width("A"), 2, "3 columns, the last is empty");
        assert_eq!(f.text_width("AB"), 5);
        assert_eq!(f.text_width("B"), 2);
        assert_eq!(f.text_width("A\nB"), 2, "first line only");
        assert_eq!(f.text_width(""), 0);
        assert_eq!(f.text_width("z"), 0, "no glyph, no advance");
    }

    #[test]
    fn pixels_follow_the_column_bits_and_advance_one_column_each() {
        let f = &font_table(&synthetic_code()).unwrap()[0];
        let mut lit = Vec::new();
        let end = f.for_each_pixel("B", 10, 20, |x, y| lit.push((x, y)));
        assert_eq!(end, 12);
        // column 0 = 0x7F rows 0..=6, column 1 = 0x49 rows 0, 3, 6
        assert_eq!(lit.len(), 7 + 3);
        assert!(lit.contains(&(10, 20)) && lit.contains(&(10, 26)) && !lit.contains(&(10, 27)));
        assert!(lit.contains(&(11, 23)) && !lit.contains(&(11, 21)));
    }

    #[test]
    fn a_character_outside_the_range_draws_the_space_glyph() {
        let f = &font_table(&synthetic_code()).unwrap()[0];
        assert_eq!(f.columns('z' as u32), f.columns('A' as u32));
    }

    #[test]
    fn damaged_images_are_errors_not_panics() {
        assert!(font_table(&[0u8; 16]).is_err());
        let mut code = synthetic_code();
        let o = (FONT_TABLE - LOAD_BASE) as usize;
        code[o..o + 4].copy_from_slice(&0x1000u32.to_le_bytes()); // below the load base
        assert!(font_table(&code).is_err());
        let mut code = synthetic_code();
        let obj = (LOAD_BASE + 0x4000 - LOAD_BASE) as usize;
        code[obj..obj + 4].copy_from_slice(&0u32.to_le_bytes()); // height 0
        assert!(parse_font(&code, LOAD_BASE + 0x4000).is_err());
    }
}
