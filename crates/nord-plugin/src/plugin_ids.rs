//! Plugin identity stability contract (D16, permissive VST3 binding).
//!
//! Hosts key saved projects on the CLAP ID, the VST3 class ID and the VST3 parameter ID (a hash of
//! the string parameter ID). Changing any of them orphans existing projects, so this module pins
//! them as literals. The string parameter IDs themselves are pinned by `param-ids.snapshot`
//! (see `param_snapshot`). The VST3 wire hash is nih-plug's `hash_param_id`, which is unchanged
//! between the `vst3-sys` and `vst3` wrappers: `src/wrapper/util.rs` is byte-identical at nih-plug
//! de42101 and at PR 263 head 95034d5.

#[cfg(test)]
mod tests {
    use crate::NdMatrix1Plugin;
    use nih_plug::prelude::*;

    /// nih-plug's `hash_param_id`: `h = h * 31 + byte` (wrapping u32), top bit cleared.
    /// VERIFIED: copy of nih-plug `src/wrapper/util.rs::hash_param_id` at rev 95034d5, checked by
    /// `hash_matches_pinned_values` (values computed independently with a Python one-liner).
    fn vst3_param_hash(id: &str) -> u32 {
        let mut hash: u32 = 0;
        for byte in id.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(byte as u32);
        }
        hash & !(1 << 31)
    }

    #[test]
    fn clap_id_is_pinned() {
        assert_eq!(
            <NdMatrix1Plugin as ClapPlugin>::CLAP_ID,
            "org.ndmatrix.nd-matrix-1"
        );
    }

    #[test]
    fn vst3_class_id_is_pinned() {
        assert_eq!(
            <NdMatrix1Plugin as Vst3Plugin>::VST3_CLASS_ID,
            [
                0x4e, 0x44, 0x4d, 0x61, 0x74, 0x72, 0x69, 0x78, 0x31, 0x45, 0x6d, 0x75, 0x6c, 0x61,
                0x74, 0x6f
            ]
        );
        assert_eq!(
            <NdMatrix1Plugin as Vst3Plugin>::VST3_CLASS_ID,
            *b"NDMatrix1Emulato"
        );
        #[cfg(not(target_os = "windows"))]
        assert_eq!(
            <NdMatrix1Plugin as Vst3Plugin>::PLATFORM_VST3_CLASS_ID,
            *b"NDMatrix1Emulato"
        );
        // Windows COM GUID layout: nih-plug's `swap_vst3_uid_byte_order` reverses bytes 0..4 and
        // swaps 4..6 and 6..8 (identical at nih-plug de42101 and 95034d5), so "NDMa" "tr" "ix"
        // becomes "aMDN" "rt" "xi"; bytes 8..16 are unchanged.
        #[cfg(target_os = "windows")]
        assert_eq!(
            <NdMatrix1Plugin as Vst3Plugin>::PLATFORM_VST3_CLASS_ID,
            *b"aMDNrtxi1Emulato"
        );
    }

    #[test]
    fn hash_matches_pinned_values() {
        for (id, hash) in [
            ("drawbar_1_16", 1_646_930_659_u32),
            ("aftertouch", 1_561_666_275),
            ("control_pedal", 1_223_771_160),
            ("sys_transpose", 278_090_823),
        ] {
            assert_eq!(vst3_param_hash(id), hash, "{id}");
        }
    }

    /// Every shipped string ID hashes to a distinct VST3 parameter ID (no host-visible aliasing).
    #[test]
    fn snapshot_ids_hash_without_collision() {
        let text = include_str!("../param-ids.snapshot");
        let ids: Vec<&str> = text
            .lines()
            .filter_map(|l| l.split('|').next())
            .filter(|s| !s.is_empty())
            .collect();
        let hashes: std::collections::BTreeSet<u32> =
            ids.iter().map(|id| vst3_param_hash(id)).collect();
        assert_eq!(ids.len(), hashes.len());
        assert!(ids.len() >= 815);
    }
}
