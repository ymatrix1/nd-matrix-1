//! The memory images of `exact_diff`: how two images are compared and how the first difference is
//! reported. OWNER: W1-VERIFY.
//!
//! Compared images: SRAM (which holds the short-word alias window), the IOP window, the dense
//! SDRAM delay-line window, the sparse device store and the ISA-mode word image. A handler can
//! store to every one of them through `dm_write`, so a wrong value or a wrong address there must
//! show; the read and write counters alone would not see it.
use super::ExactDiff;
use crate::core::addr_hash::DeviceMap;
use crate::core::sdram;
use std::fmt;

/// First difference of two images of equal meaning: a length mismatch, else the index and the two
/// values of the first differing element.
pub(super) fn slice_diff<T: Copy + PartialEq + fmt::LowerHex>(
    field: &'static str,
    a: &[T],
    b: &[T],
) -> Option<ExactDiff> {
    if a == b {
        return None;
    }
    if a.len() != b.len() {
        return Some(ExactDiff {
            field,
            a: format!("len {}", a.len()),
            b: format!("len {}", b.len()),
        });
    }
    let k = a.iter().zip(b).position(|(x, y)| x != y)?;
    Some(ExactDiff {
        field,
        a: format!("[{k:#x}] = {:#x}", a[k]),
        b: format!("[{k:#x}] = {:#x}", b[k]),
    })
}

/// First differing word of the SDRAM window. The window is allocated on its first write, so an
/// unallocated window and a window of zeros are the same state.
pub(super) fn sdram_diff(a: &sdram::Sdram, b: &sdram::Sdram) -> Option<ExactDiff> {
    if a.mem == b.mem {
        return None;
    }
    let at = |m: &[u32], k: usize| m.get(k).copied().unwrap_or(0);
    let len = a.mem.len().max(b.mem.len());
    let k = (0..len).find(|&k| at(&a.mem, k) != at(&b.mem, k))?;
    let addr = sdram::BASE + k as u32;
    Some(ExactDiff {
        field: "sdram",
        a: format!("[{addr:#x}] = {:#x}", at(&a.mem, k)),
        b: format!("[{addr:#x}] = {:#x}", at(&b.mem, k)),
    })
}

/// First differing entry of the sparse device store, by address. A stored zero and a missing entry
/// differ (every engine writes through `dm_write`, so the entries match exactly).
pub(super) fn device_diff(a: &DeviceMap, b: &DeviceMap) -> Option<ExactDiff> {
    if a == b {
        return None;
    }
    let mut keys: Vec<u32> = a.keys().chain(b.keys()).copied().collect();
    keys.sort_unstable();
    keys.dedup();
    let k = keys.into_iter().find(|k| a.get(k) != b.get(k))?;
    Some(ExactDiff {
        field: "device",
        a: format!("[{k:#x}] = {:?}", a.get(&k)),
        b: format!("[{k:#x}] = {:?}", b.get(&k)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::verify::compare::exact_diff;
    use crate::core::Core;

    #[test]
    fn the_report_names_the_first_differing_memory_byte() {
        let (mut a, b) = (Core::new(&[]), Core::new(&[]));
        a.sram[0x1234] = 0xAB;
        let d = exact_diff(&a, &b).unwrap();
        assert_eq!((d.field, d.a.as_str()), ("sram", "[0x1234] = 0xab"));
        assert!(d.to_string().starts_with("sram: [0x1234]"));
    }

    #[test]
    fn a_store_into_the_sdram_window_is_seen_by_value_and_an_empty_window_equals_zeros() {
        let (mut a, mut b) = (Core::new(&[]), Core::new(&[]));
        a.dm_write(sdram::BASE + 0x40, 0).unwrap();
        b.dm_write(sdram::BASE + 0x80, 0).unwrap();
        assert!(!a.sdram.mem.is_empty() && !b.sdram.mem.is_empty());
        let (c, d) = (Core::new(&[]), Core::new(&[]));
        let mut e = Core::new(&[]);
        e.sdram.write(sdram::BASE, 0); // allocated, all zeros
        assert_eq!(exact_diff(&c, &e), None, "unallocated equals zero-filled");
        assert_eq!(exact_diff(&e, &d), None);
        // the counters differ from a fresh core (the writes were counted); equalise them
        (a.sdram_writes, b.sdram_writes) = (0, 0);
        assert_eq!(exact_diff(&a, &b), None, "both windows hold only zeros");
        a.sdram.write(sdram::BASE + 0x40, 0xAA);
        let d = exact_diff(&a, &b).unwrap();
        assert_eq!(
            (d.field, d.a.as_str(), d.b.as_str()),
            ("sdram", "[0x8000040] = 0xaa", "[0x8000040] = 0x0")
        );
        assert_eq!(exact_diff(&b, &a).map(|d| d.field), Some("sdram"));
    }

    #[test]
    fn the_device_store_is_compared_by_address_and_value() {
        let (mut a, mut b) = (Core::new(&[]), Core::new(&[]));
        a.dm_write(0x3000_0010, 5).unwrap();
        b.dm_write(0x3000_0010, 5).unwrap();
        assert_eq!(exact_diff(&a, &b), None);
        b.dm_write(0x3000_0010, 6).unwrap();
        b.device_writes = a.device_writes; // the counters are not what differs
        let d = exact_diff(&a, &b).unwrap();
        assert_eq!(
            (d.field, d.a.as_str(), d.b.as_str()),
            ("device", "[0x30000010] = Some(5)", "[0x30000010] = Some(6)")
        );
        b.dm_write(0x3000_0010, 5).unwrap();
        b.dm_write(0x3000_0020, 0).unwrap(); // a stored zero is an entry a never made
        b.device_writes = a.device_writes;
        let d = exact_diff(&a, &b).unwrap();
        assert_eq!(
            (d.field, d.a.as_str(), d.b.as_str()),
            ("device", "[0x30000020] = None", "[0x30000020] = Some(0)")
        );
    }

    #[test]
    fn isa_words_and_iop_report_the_index_and_a_length_change() {
        let (mut a, b) = (Core::new(&[]), Core::new(&[]));
        a.isa_code.extend_from_slice(&[1, 2, 3]);
        let d = exact_diff(&a, &b).unwrap();
        assert_eq!(
            (d.field, d.a.as_str(), d.b.as_str()),
            ("isa_code", "len 3", "len 0")
        );
        let mut b = Core::new(&[]);
        b.isa_code.extend_from_slice(&[1, 9, 3]);
        let d = exact_diff(&a, &b).unwrap();
        assert_eq!(
            (d.field, d.a.as_str(), d.b.as_str()),
            ("isa_code", "[0x1] = 0x2", "[0x1] = 0x9")
        );
        a.iop[7] = 0xBEEF;
        assert_eq!(exact_diff(&a, &b).map(|d| d.field), Some("iop"));
    }
}
