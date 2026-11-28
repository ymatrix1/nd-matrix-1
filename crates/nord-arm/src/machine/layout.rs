//! Firmware addresses of one OS version.
//!
//! The OS is recompiled for every release, so each ARM0 function and data cell moves a little
//! (v1.68 vs v1.66: `docs/research/os-1-68-vs-1-66.md`). The harness addresses live here, one
//! `Layout` per known version, picked from the `bank0_CODE` image length by [`Layout::detect`].
//! A new version is added by running `python tools/fw_addr_map.py OLD NEW <addresses>` (it
//! matches instruction windows with the branch offsets masked) and filling one more constant.
//!
//! Provenance: V166 values are the harness addresses used since `tools/arm_emu.py` (ASSUMED /
//! EMPIRICAL as tagged in `boot.rs`; the boot reaches the main loop). V168 values are EMPIRICAL:
//! derived by `tools/fw_addr_map.py` from the V166 ones; the boot and the null-call and message
//! counts of `arm-lle-bench` on the v1.68 image are the check (see the research note).

/// Harness addresses of one OS version (all in the `bank0_CODE` image or its BSS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// OS version this layout was derived for (`166` = v1.66).
    pub version: u32,
    /// `bank0_CODE.bin` length in bytes, the key of [`Layout::detect`].
    pub image_len: usize,
    /// DSP flow-control function, stubbed to return 1.
    pub flow_control: u32,
    /// Pointer cell of the DSP port object.
    pub port_object_ptr: u32,
    /// Table of the five DSP comm objects.
    pub comm_table: u32,
    /// `operator new` (bump allocator hook).
    pub operator_new: u32,
    /// The MMU-enable `mcr` skipped by the harness.
    pub mmu_enable_mcr: u32,
    /// DSP download function.
    pub dsp_download: u32,
    /// SPI block download function.
    pub spi_download_block: u32,
    /// Logger.
    pub logger: u32,
    /// Board-config query.
    pub board_config_query: u32,
    /// Display event insert.
    pub event_insert: u32,
    /// Text measure.
    pub text_measure: u32,
    /// SPI transfer (also the SPI N-byte read of the DSP return path).
    pub spi_transfer: u32,
    /// SPI count check.
    pub spi_count_check: u32,
    /// Static initialiser entry points run before the application thunk.
    pub init_array: [u32; 4],
    /// Application entry thunk.
    pub app_thunk: u32,
    /// First instruction of the main loop body.
    pub main_loop: u32,
    /// AINTC handler table.
    pub irq_table: u32,
    /// SPI read of 4 and of 2 bytes (DSP return path).
    pub spi_read4: u32,
    pub spi_read2: u32,
    /// Panel detection routine `[start, end)`.
    pub detect_range: (u32, u32),
    /// Panel wait loop `[start, end]`.
    pub wait_loop: (u32, u32),
    /// Board attach routine.
    pub board_attach: u32,
}

impl Layout {
    /// OS v1.66 (the version analysed first; the harness default).
    pub const V166: Layout = Layout {
        version: 166,
        image_len: 5_499_504,
        flow_control: 0xC006_CA48,
        port_object_ptr: 0xC055_B150,
        comm_table: 0xC053_F8A4,
        operator_new: 0xC025_D56C,
        mmu_enable_mcr: 0xC002_4624,
        dsp_download: 0xC024_1C58,
        spi_download_block: 0xC024_0548,
        logger: 0xC008_0314,
        board_config_query: 0xC024_BF70,
        event_insert: 0xC001_B67C,
        text_measure: 0xC001_B9D0,
        spi_transfer: 0xC006_6E64,
        spi_count_check: 0xC022_81F0,
        init_array: [0xC000_02A4, 0xC024_D1A0, 0xC024_DC64, 0xC025_DBBC],
        app_thunk: 0xC025_D524,
        main_loop: 0xC00A_0248,
        irq_table: 0xC055_F674,
        spi_read4: 0xC001_19A4,
        spi_read2: 0xC001_17E8,
        detect_range: (0xC024_D648, 0xC024_D710),
        wait_loop: (0xC007_5D38, 0xC007_5D50),
        board_attach: 0xC024_DB74,
    };

    /// OS v1.68 (EMPIRICAL, see the module doc).
    pub const V168: Layout = Layout {
        version: 168,
        image_len: 5_500_432,
        flow_control: 0xC006_CA94,
        port_object_ptr: 0xC055_B4F0,
        comm_table: 0xC053_FC44,
        operator_new: 0xC025_D8D4,
        mmu_enable_mcr: 0xC002_4694,
        dsp_download: 0xC024_1FC0,
        spi_download_block: 0xC024_08B0,
        logger: 0xC008_0368,
        board_config_query: 0xC024_C2D8,
        event_insert: 0xC001_B6EC,
        text_measure: 0xC001_BA40,
        spi_transfer: 0xC006_6EB0,
        spi_count_check: 0xC022_8570,
        init_array: [0xC000_02A4, 0xC024_D508, 0xC024_DFCC, 0xC025_DF24],
        app_thunk: 0xC025_D88C,
        main_loop: 0xC00A_029C,
        irq_table: 0xC055_FA14,
        spi_read4: 0xC001_1A10,
        spi_read2: 0xC001_1854,
        detect_range: (0xC024_D9B0, 0xC024_DA78),
        wait_loop: (0xC007_5D8C, 0xC007_5DA4),
        board_attach: 0xC024_DEDC,
    };

    /// Every known layout.
    pub const KNOWN: [Layout; 2] = [Self::V166, Self::V168];

    /// The layout whose image length equals `code.len()`, if any.
    pub fn detect(code: &[u8]) -> Option<Layout> {
        Self::KNOWN.into_iter().find(|l| l.image_len == code.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_picks_the_layout_by_image_length() {
        assert_eq!(Layout::detect(&vec![0; 5_499_504]).unwrap().version, 166);
        assert_eq!(Layout::detect(&vec![0; 5_500_432]).unwrap().version, 168);
        assert!(Layout::detect(&[0; 16]).is_none());
    }

    #[test]
    fn known_layouts_have_distinct_lengths_and_versions() {
        let k = Layout::KNOWN;
        assert_ne!(k[0].image_len, k[1].image_len);
        assert_ne!(k[0].version, k[1].version);
    }

    #[test]
    fn every_address_is_inside_the_image_or_its_bss() {
        for l in Layout::KNOWN {
            for a in [
                l.flow_control,
                l.app_thunk,
                l.main_loop,
                l.irq_table,
                l.comm_table,
            ] {
                assert!((0xC000_0000..0xC060_0000).contains(&a), "{a:#x}");
            }
            assert!(l.detect_range.0 < l.detect_range.1);
            assert!(l.wait_loop.0 < l.wait_loop.1);
        }
    }
}
