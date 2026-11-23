//! Loader block stream of a SHARC DSP image (FX, PNO or ORG) and the flat SRAM image of a core.
//!
//! Port of `tools/ns4_dsp_ldr.py` (`docs/research/dsp-image-hunt.md` 5.17 and 5.18). All fields
//! are little endian. The image is a sequence of blocks with a 16 byte header `tag, address,
//! count, aux`:
//!
//! | tag low 16 | kind | payload |
//! |---|---|---|
//! | `0x5003` | core section header (`address` = entry value, `aux` = bytes in the section) | none |
//! | `0x0003` | data block: `count` bytes loaded at `address` | `count` bytes |
//! | `0x0103` | zero fill: `count` zero bytes at `address` | none |
//! | `0x8003` | final block of a core section | none |
//!
//! VERIFIED: tag byte 3 is the XOR of the other 15 header bytes on every block of the v1.66 FX,
//! PNO and ORG images (`python tools/ns4_dsp_ldr.py IMAGE`: 0 header checksum failures). Internal
//! RAM blocks have top address byte `0x28`; the flat image is indexed by `address & 0x7FFFFF`, so
//! byte offset `2 * pc` holds the instruction parcel at short-word address `pc` and byte offset
//! `4 * a` the 32-bit DM word at address `a`.

const HEADER: usize = 16;
const SECTION: u32 = 0x5003;
const DATA: u32 = 0x0003;
const ZERO: u32 = 0x0103;
const FINAL: u32 = 0x8003;
/// Top address byte of internal SRAM blocks.
const INTERNAL_TOP: u32 = 0x28;
/// Bytes of the flat SRAM image (`address & 0x7FFFFF` stays below `0x3A0000` in v1.66).
pub const SRAM_BYTES: usize = 0x40_0000;

#[derive(Debug, Clone, Copy)]
struct Block {
    offset: usize,
    kind: u32,
    address: u32,
    count: usize,
}

fn word(img: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([img[at], img[at + 1], img[at + 2], img[at + 3]])
}

/// Blocks of the stream; `Err` when a header is foreign, its checksum is wrong or a payload runs
/// past the image.
fn parse_blocks(img: &[u8]) -> Result<Vec<Block>, String> {
    let mut blocks = Vec::new();
    let mut o = 0;
    while o + HEADER <= img.len() {
        let tag = word(img, o);
        let kind = tag & 0xFFFF;
        if ![SECTION, DATA, ZERO, FINAL].contains(&kind) {
            return Err(format!("unknown loader block tag {tag:#x} at {o:#x}"));
        }
        let x = img[o..o + HEADER]
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != 3)
            .fold(0u8, |a, (_, &b)| a ^ b);
        if u32::from(x) != tag >> 24 {
            return Err(format!("loader block header checksum failed at {o:#x}"));
        }
        let count = word(img, o + 8) as usize;
        let next = o + HEADER + if kind == DATA { count } else { 0 };
        if next > img.len() {
            return Err(format!("loader block at {o:#x} runs past the image"));
        }
        blocks.push(Block {
            offset: o,
            kind,
            address: word(img, o + 4),
            count,
        });
        o = next;
    }
    if o != img.len() || blocks.is_empty() {
        return Err("not a loader block stream".into());
    }
    Ok(blocks)
}

/// Flat SRAM image (`SRAM_BYTES`) of core section `core` (0 = Core1) of `img`.
pub fn core_sram(img: &[u8], core: usize) -> Result<Vec<u8>, String> {
    let blocks = parse_blocks(img)?;
    let mut cores: Vec<Vec<Block>> = Vec::new();
    for b in blocks {
        if b.kind == SECTION {
            cores.push(Vec::new());
        }
        if let Some(c) = cores.last_mut() {
            c.push(b);
        }
    }
    let section = cores.get(core).ok_or_else(|| {
        format!(
            "the image holds {} core sections, not {}",
            cores.len(),
            core + 1
        )
    })?;
    let mut mem = vec![0u8; SRAM_BYTES];
    for b in section {
        if b.kind == DATA && b.address >> 24 == INTERNAL_TOP {
            let lo = (b.address & 0x7F_FFFF) as usize;
            let src = img
                .get(b.offset + HEADER..b.offset + HEADER + b.count)
                .ok_or("payload beyond the image")?;
            mem.get_mut(lo..lo + b.count)
                .ok_or_else(|| format!("block at {:#x} is outside the SRAM image", b.address))?
                .copy_from_slice(src);
        }
    }
    Ok(mem)
}

/// 32-bit little endian word `a` of a flat SRAM image (a DM word address; 0 outside the image).
pub fn dm_word(sram: &[u8], a: u32) -> u32 {
    let at = 4 * a as usize;
    sram.get(at..at + 4)
        .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn header(kind: u32, address: u32, count: u32, aux: u32) -> Vec<u8> {
        let mut h = Vec::new();
        for w in [kind, address, count, aux] {
            h.extend_from_slice(&w.to_le_bytes());
        }
        let x = h
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != 3)
            .fold(0u8, |a, (_, &b)| a ^ b);
        h[3] = x;
        h
    }

    /// Two core sections: core 1 holds `words` at DM word address 4 and a zero block, core 2 one
    /// block of `0xEE` bytes at the same address.
    pub(crate) fn image(words: &[u32]) -> Vec<u8> {
        let mut img = header(SECTION, 0x1000, 0, 0);
        let payload: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        img.extend(header(DATA, 0x2800_0000 + 16, payload.len() as u32, 0));
        img.extend(&payload);
        img.extend(header(ZERO, 0x2800_0100, 64, 0));
        img.extend(header(FINAL, 0x1000, 0, 0));
        img.extend(header(SECTION, 0x2000, 0, 0));
        img.extend(header(DATA, 0x2880_0000 + 16, 4, 0));
        img.extend([0xEE; 4]);
        img.extend(header(FINAL, 0x2000, 0, 0));
        img
    }

    #[test]
    fn core_one_gets_its_data_blocks_at_the_masked_address() {
        let sram = core_sram(&image(&[0x1122_3344, 7]), 0).unwrap();
        assert_eq!(sram.len(), SRAM_BYTES);
        assert_eq!(dm_word(&sram, 4), 0x1122_3344);
        assert_eq!(dm_word(&sram, 5), 7);
        assert_eq!(dm_word(&sram, 6), 0);
    }

    #[test]
    fn core_two_is_its_own_section_and_a_third_does_not_exist() {
        let img = image(&[1]);
        assert_eq!(dm_word(&core_sram(&img, 1).unwrap(), 4), 0xEEEE_EEEE);
        assert!(core_sram(&img, 2).unwrap_err().contains("2 core sections"));
    }

    #[test]
    fn a_bad_header_checksum_a_foreign_tag_and_a_short_payload_are_errors() {
        let mut img = image(&[1]);
        img[8] ^= 1; // count of the section header: the header XOR no longer matches
        assert!(core_sram(&img, 0).unwrap_err().contains("checksum"));
        let foreign = header(0x1234, 0, 0, 0);
        assert!(core_sram(&foreign, 0).unwrap_err().contains("unknown"));
        let mut cut = image(&[1, 2, 3]);
        cut.truncate(16 + 16 + 4);
        assert!(core_sram(&cut, 0).is_err());
        assert!(core_sram(&[], 0).is_err());
    }

    #[test]
    fn dm_word_outside_the_image_reads_zero() {
        assert_eq!(dm_word(&[1, 2, 3, 4], 1), 0);
        assert_eq!(dm_word(&[1, 2, 3, 4], 0), 0x0403_0201);
    }
}
