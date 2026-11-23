//! The SHARC DSP images inside the user's OS update file, read at run time (D128).
//!
//! `bank0_CODE` of the firmware container carries five DSP programs back to back (FX, PNO, ORG,
//! SY1SY2, SMP, `docs/research/dsp-image-hunt.md` 5.12 and 5.13). The ARM code hands them to the
//! DSP driver as `(pointer, length)` word pairs in a literal pool; the pairs chain (`pointer +
//! length` of one image is the pointer of the next). [`find_chain`] finds that chain without fixed
//! offsets, as `tools/ns4_dsp_images.py` does. [`ldr`] reads the loader block stream of an image
//! into the flat SRAM image of one core.
//!
//! Nothing here is stored: callers read the user's file at setup time and drop the bytes (D22).

pub mod ldr;

use crate::firmware::FirmwareContainer;
use std::collections::HashMap;
use std::path::Path;

/// Load address of `bank0_CODE`.
///
/// VERIFIED: pointers in the pool are `0xC0000000 + offset`, `docs/research/dsp-image-hunt.md`
/// 5.12; `find_chain` finds five images of 164,880, 288,804, 296,656, 121,844 and 44,596 bytes in
/// the v1.66 update file (`tools/ns4_dsp_images.py` gives the same table).
const LOAD_BASE: u32 = 0xC000_0000;
/// Image length bounds of a pool entry (`tools/ns4_dsp_images.py`). ASSUMED: wide enough for the
/// five images of v1.44 to v1.66 (44,596 to 296,656 B); a larger image would end the chain early
/// and `image_of` would report it.
const MIN_LEN: u32 = 0x400;
const MAX_LEN: u32 = 0x10_0000;
/// Word distance between a pointer and its length in the pool.
const WINDOW: usize = 2;
/// Download index of the PNO image (order FX, PNO, ORG, SY1SY2, SMP).
///
/// VERIFIED for index 1: the Farfisa routine `0x122B85` decodes from it and runs bit exact against
/// `sharc-chain` (`farfisa_fw::tests`); the names of the other indices stay ASSUMED
/// (`docs/research/dsp-image-hunt.md` 5.12 and 5.13).
pub const PNO: usize = 1;

fn text_like(head: &[u8]) -> bool {
    !head.is_empty() && head.iter().filter(|b| (32..127).contains(*b)).count() * 4 >= head.len() * 3
}

/// Longest chain of `(offset, length)` pairs of `data` with `offset + length == next offset`.
///
/// Port of `find_chain` of `tools/ns4_dsp_images.py` (same tie breaks: longest chain first, then
/// the smallest total pointer to length word distance).
pub fn find_chain(data: &[u8]) -> Vec<(usize, usize)> {
    let words: Vec<u32> = data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect();
    let n = words.len();
    let mut ptr_at: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, &w) in words.iter().enumerate() {
        if w >= LOAD_BASE && ((w - LOAD_BASE) as usize) < data.len() {
            ptr_at.entry((w - LOAD_BASE) as usize).or_default().push(i);
        }
    }
    // offset -> length -> smallest word distance
    let mut pairs: HashMap<usize, HashMap<usize, usize>> = HashMap::new();
    for (&off, locs) in &ptr_at {
        for &i in locs {
            for j in i.saturating_sub(WINDOW)..(i + WINDOW + 1).min(n) {
                let len = words.get(j).copied().unwrap_or(0);
                if j != i && (MIN_LEN..=MAX_LEN).contains(&len) && off + len as usize <= data.len()
                {
                    let d = pairs.entry(off).or_default();
                    let e = d.entry(len as usize).or_insert(WINDOW + 1);
                    *e = (*e).min(i.abs_diff(j));
                }
            }
        }
    }
    let mut best: Vec<(usize, usize)> = Vec::new();
    let mut best_key = (0usize, i64::MIN);
    let mut starts: Vec<usize> = pairs.keys().copied().collect();
    starts.sort_unstable();
    let mut path = Vec::new();
    for s in starts {
        walk(&pairs, s, &mut path, 0, &mut best, &mut best_key);
    }
    while best
        .last()
        .is_some_and(|&(o, _)| text_like(&data[o..(o + 32).min(data.len())]))
    {
        best.pop();
    }
    best
}

type Pairs = HashMap<usize, HashMap<usize, usize>>;

fn walk(
    pairs: &Pairs,
    off: usize,
    path: &mut Vec<(usize, usize)>,
    dist: i64,
    best: &mut Vec<(usize, usize)>,
    best_key: &mut (usize, i64),
) {
    let Some(next) = pairs.get(&off) else { return };
    let mut lens: Vec<(usize, usize)> = next.iter().map(|(&l, &d)| (l, d)).collect();
    lens.sort_unstable();
    for (len, d) in lens {
        path.push((off, len));
        let nd = dist + d as i64;
        let key = (path.len(), -nd);
        if key > *best_key {
            *best_key = key;
            best.clone_from(path);
        }
        walk(pairs, off + len, path, nd, best, best_key);
        path.pop();
    }
}

/// The DSP image with download index `index` (`0..5`) of an unpacked `bank0_CODE`.
pub fn image_of(code: &[u8], index: usize) -> Result<&[u8], String> {
    let chain = find_chain(code);
    if chain.len() != 5 {
        return Err(format!(
            "expected five DSP images in bank0_CODE, found a chain of {}",
            chain.len()
        ));
    }
    let (off, len) = chain[index];
    Ok(&code[off..off + len])
}

/// `bank0_CODE` of the user's OS update file (`.exe`, `.dmg`, a `.zip` holding one, or the raw
/// container), read and decompressed at run time.
pub fn bank0_code(path: &Path) -> Result<Vec<u8>, String> {
    let raw = crate::os_update::read_capped(path)?;
    let container = crate::os_update::extract_firmware_container(&raw)
        .map_err(|why| format!("{}: {why}", path.display()))?;
    let fw = FirmwareContainer::parse(&container)?;
    fw.extract_segment(0, "CODE", &container)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A code image with a pool of `sizes.len()` chained images and a text tail.
    pub(crate) fn code_with_images(sizes: &[usize], fill: &[u8]) -> (Vec<u8>, Vec<(usize, usize)>) {
        let pool = 0x100usize;
        let mut off = 0x2000usize;
        let mut chain = Vec::new();
        for &s in sizes {
            chain.push((off, s));
            off += s;
        }
        let mut code = vec![0u8; off + 0x400];
        for (k, &(o, l)) in chain.iter().enumerate() {
            let at = pool + 8 * k;
            code[at..at + 4].copy_from_slice(&(LOAD_BASE + o as u32).to_le_bytes());
            code[at + 4..at + 8].copy_from_slice(&(l as u32).to_le_bytes());
            for (j, b) in code[o..o + l].iter_mut().enumerate() {
                *b = fill[j % fill.len()];
            }
        }
        for (j, b) in code[off..].iter_mut().enumerate() {
            *b = b"Layer Level Adjust"[j % 18];
        }
        (code, chain)
    }

    #[test]
    fn the_chain_of_pointer_length_pairs_is_found_and_the_text_tail_dropped() {
        let sizes = [0x800, 0x900, 0xA00, 0x500, 0x600];
        let (code, chain) = code_with_images(&sizes, &[1, 2, 3, 0xF0]);
        // the sixth pool entry points at the string table that follows the last image
        let mut code = code;
        let end = chain[4].0 + chain[4].1;
        let at = 0x100 + 40;
        code[at..at + 4].copy_from_slice(&(LOAD_BASE + end as u32).to_le_bytes());
        code[at + 4..at + 8].copy_from_slice(&0x400u32.to_le_bytes());
        assert_eq!(find_chain(&code), chain);
        let img = image_of(&code, PNO).unwrap();
        assert_eq!(img.len(), 0x900);
    }

    #[test]
    fn fewer_than_five_images_is_an_error() {
        let (code, _) = code_with_images(&[0x800, 0x900], &[1, 2, 3, 0xF0]);
        assert!(image_of(&code, 0).unwrap_err().contains("five DSP images"));
        assert!(find_chain(&[0u8; 64]).is_empty());
    }

    #[test]
    fn a_missing_update_file_names_the_path() {
        let e = bank0_code(Path::new("no/such/update.exe")).unwrap_err();
        assert!(e.contains("no/such/update.exe"), "{e}");
    }
}
