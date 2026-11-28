//! DM image text of a firmware DSP machine, built from the user's OS update file (D128).
//!
//! The text is the format of `Machine::load_image` (`R,name,hex` start register lines and
//! `M,addr,hex hex ...` runs of non zero words). It is made of three parts:
//!
//! | Part | Source |
//! |---|---|
//! | DM words of the kept ranges | the core SRAM image of the loader stream ([`super::ldr`]) |
//! | install words, start registers | the delta text of the caller (our own capture of the ARM install) |
//! | words the ARM sends from its constants | the delta `A` lines: `bank0_CODE` at a byte offset |
//!
//! Delta lines (`#` and blank lines are ignored):
//!
//! * `R,name,hex`: a start register, copied to the text;
//! * `M,addr,hex hex ...`: inline words from `addr` (a `0` clears a loader word);
//! * `A,addr,count,offset`: `count` words from `addr`, read from `bank0_CODE` at byte `offset`
//!   (all hex). The line names where the user's own file holds the words; it carries none.

use super::ldr;
use std::collections::BTreeMap;

fn hex(s: &str, line: &str) -> Result<u32, String> {
    u32::from_str_radix(s.trim(), 16).map_err(|_| format!("bad delta {line:?}"))
}

/// `bank0_CODE` byte offsets `[from, to)` that an OS version holds `delta` bytes later than the
/// offsets of the delta `A` lines (written for v1.66).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shift {
    pub from: u32,
    pub to: u32,
    pub delta: i32,
}

/// Offset shifts of OS v1.68 against v1.66 for the rodata the `A` lines read: derived by finding
/// each of the 53 `A` ranges of the Bass delta byte for byte in the v1.68 image, unique in every
/// case (`docs/research/os-1-68-vs-1-66.md` s4). The constants themselves are unchanged, so the
/// image fingerprint of the Bass image is the same; that guard is what accepts this table.
///
/// VERIFIED: `+0x368` for `0x270000..0x2B6000` (51 ranges), `+0x3A0` from `0x2D8644` on (2 ranges:
/// `0x442E0C`, `0x444187`).
pub const OS_V168: &[Shift] = &[
    Shift {
        from: 0x27_0000,
        to: 0x2B_6000,
        delta: 0x368,
    },
    Shift {
        from: 0x2D_8644,
        to: 0x60_0000,
        delta: 0x3A0,
    },
];

/// Every offset table to try, in order: the offsets as written (v1.66), then each later version.
pub const KNOWN_SHIFTS: [&[Shift]; 2] = [&[], OS_V168];

fn shifted(off: u32, shifts: &[Shift]) -> Result<u32, String> {
    if shifts.is_empty() {
        return Ok(off);
    }
    let s = shifts
        .iter()
        .find(|s| (s.from..s.to).contains(&off))
        .ok_or_else(|| format!("offset {off:#x} is outside the known layout of this OS version"))?;
    Ok((i64::from(off) + i64::from(s.delta)) as u32)
}

/// DM image text of the `keep` ranges `(base, words)` of the flat core `sram` image, after the
/// install `delta` (words from `code`, the `bank0_CODE` bytes, for its `A` lines), and the number
/// of non zero words.
pub fn dm_image_text(
    sram: &[u8],
    code: &[u8],
    keep: &[(u32, u32)],
    delta: &str,
) -> Result<(String, usize), String> {
    dm_image_text_shifted(sram, code, keep, delta, &[])
}

/// [`dm_image_text`] with the byte offsets of the `A` lines moved by `shifts` (none: as written).
pub fn dm_image_text_shifted(
    sram: &[u8],
    code: &[u8],
    keep: &[(u32, u32)],
    delta: &str,
    shifts: &[Shift],
) -> Result<(String, usize), String> {
    let mut words: BTreeMap<u32, u32> = BTreeMap::new();
    for &(base, n) in keep {
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
                let a = hex(addr, line)?;
                for (k, v) in vals.split(' ').enumerate() {
                    words.insert(a + k as u32, hex(v, line)?);
                }
            }
            ["A", addr, count, offset] => {
                let (a, n, off) = (
                    hex(addr, line)?,
                    hex(count, line)? as usize,
                    shifted(hex(offset, line)?, shifts)?,
                );
                let bytes = code
                    .get(off as usize..off as usize + 4 * n)
                    .ok_or_else(|| format!("delta {line:?} reads outside bank0_CODE"))?;
                for (k, c) in bytes.as_chunks::<4>().0.iter().enumerate() {
                    words.insert(a + k as u32, u32::from_le_bytes(*c));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat SRAM image with two DM words in the range `0xC27E0..` and `0x95FFC`.
    fn sram() -> Vec<u8> {
        let mut sram = vec![0u8; ldr::SRAM_BYTES];
        for (a, v) in [(0xC_27E0u32, 0x3F80_0000u32), (0x9_5FFC, 5)] {
            sram[4 * a as usize..4 * a as usize + 4].copy_from_slice(&v.to_le_bytes());
        }
        sram
    }

    const KEEP: [(u32, u32); 2] = [(0x9_5FFC, 2), (0xC_27E0, 4)];

    #[test]
    fn the_text_merges_runs_applies_the_delta_and_drops_zero_words() {
        let delta = "# c\nR,i0,c00ea\nM,c27e1,4 5\nM,9,0\n";
        // the zero word at 0x9 is outside the kept ranges but still a (zero) word: dropped
        let (text, words) = dm_image_text(&sram(), &[], &KEEP, delta).unwrap();
        assert_eq!(words, 4);
        assert_eq!(text, "R,i0,c00ea\nM,95ffc,5\nM,c27e0,3f800000 4 5\n");
    }

    #[test]
    fn a_delta_zero_clears_a_loader_word() {
        let (text, _) = dm_image_text(&sram(), &[], &KEEP, "M,c27e0,0\n").unwrap();
        assert_eq!(text, "M,95ffc,5\n");
    }

    #[test]
    fn an_a_line_copies_words_from_the_code_bytes() {
        let mut code = vec![0u8; 0x20];
        code[0x10..0x18].copy_from_slice(&[1, 0, 0, 0, 0xFF, 0, 0, 0]);
        let (text, words) = dm_image_text(&sram(), &code, &KEEP, "A,c27e0,2,10\n").unwrap();
        assert_eq!(words, 3);
        assert_eq!(text, "M,95ffc,5\nM,c27e0,1 ff\n");
    }

    #[test]
    fn a_malformed_delta_or_a_read_outside_the_code_is_an_error() {
        for bad in ["X,1,2\n", "M,zz,1\n", "M,10,gg\n", "A,1,2\n", "A,1,1,100\n"] {
            assert!(
                dm_image_text(&sram(), &[0; 8], &KEEP, bad).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn shifts_move_the_a_line_offsets_and_unknown_offsets_are_an_error() {
        let mut code = vec![0u8; 0x40];
        code[0x30..0x34].copy_from_slice(&[7, 0, 0, 0]);
        let sh = [Shift {
            from: 0x10,
            to: 0x20,
            delta: 0x20,
        }];
        let (text, _) = dm_image_text_shifted(
            &sram(),
            &code,
            &KEEP,
            "A,c27e0,1,10
",
            &sh,
        )
        .unwrap();
        assert!(
            text.contains(
                "M,c27e0,7
"
            ),
            "{text}"
        );
        let e = dm_image_text_shifted(
            &sram(),
            &code,
            &KEEP,
            "A,c27e0,1,28
",
            &sh,
        )
        .unwrap_err();
        assert!(e.contains("outside the known layout"), "{e}");
        assert_eq!(shifted(0x2A_7000, &[]).unwrap(), 0x2A_7000);
    }

    #[test]
    fn the_v168_table_is_ordered_and_does_not_overlap() {
        for w in OS_V168.windows(2) {
            assert!(w[0].from < w[0].to && w[0].to <= w[1].from);
        }
        assert_eq!(shifted(0x2A_765C, OS_V168).unwrap(), 0x2A_79C4);
        assert_eq!(shifted(0x44_4187, OS_V168).unwrap(), 0x44_4527);
        assert_eq!(KNOWN_SHIFTS[0].len(), 0);
    }
}
