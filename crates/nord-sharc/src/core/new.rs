//! Construction of a [`Core`]: reset state of every register file, stack and peripheral hook.
use super::*;

impl Core {
    /// A core with `sram` as its internal memory (padded or cut to [`SRAM_BYTES`]).
    pub fn new(sram: &[u8]) -> Self {
        let mut mem = vec![0u8; SRAM_BYTES].into_boxed_slice();
        let n = sram.len().min(SRAM_BYTES);
        mem[..n].copy_from_slice(&sram[..n]);
        let mut core = Core {
            pc: 0,
            r: [0; 16],
            s: [0; 16],
            i: [0; 16],
            m: [0; 16],
            l: [0; 16],
            b: [0; 16],
            mode1: 0,
            mode2: 0,
            ustat: [0; 4],
            mr: [0; 2],
            lenient: false,
            fast_float: false,
            sharc_plus: false,
            skipped: 0,
            first_skipped: None,
            skipped_pcs: Vec::new(),
            watch: u32::MAX,
            watch_len: 1,
            watch_hits: Vec::new(),
            watch_addrs: Vec::new(),
            watch_limit: WATCH_LIMIT,
            rwatch: u32::MAX,
            trace: None,
            trace_dm: (0, 0),
            trace_log: Vec::new(),
            rwatch_len: 1,
            rwatch_hits: Vec::new(),
            rwatch_on: false,
            imask: 0,
            mmask: sysregs::MMASK_RESET,
            flags: Flags::default(),
            flags_y: Flags::default(),
            mr_y: [0; 2],
            fx: float_ops::ExtFile::default(),
            fx_y: float_ops::ExtFile::default(),
            steps: 0,
            site_hits: Vec::new(),
            sram: mem,
            iop: vec![0u32; IOP_WORDS].into_boxed_slice(),
            device: DeviceMap::default(),
            preset: DeviceMap::default(),
            device_reads: 0,
            device_writes: 0,
            call_stack: Vec::with_capacity(64),
            loops: Vec::with_capacity(8),
            pending: None,
            irq: interrupts::IrqState::new(),
            timer: timer::TimerState::default(),
            sys: sysregs::SysState::new(),
            banks: banks::Banks::default(),
            cb_wrap: std::cell::Cell::new(0),
            isa_code: Vec::new(),
            periph: None,
            decode_cache: None,
            code_gen: 1,
            sram_code_nw_lo: u32::MAX,
            sram_code_nw_hi: 0,
            aot_hot: None,
            aot_lo: 0,
            aot_span: 0,
            aot_bits: Box::default(),
            #[cfg(feature = "jit")]
            jit: None,
        };
        core.sites_from_env();
        core
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_core_is_reset_and_pads_a_short_image() {
        let c = Core::new(&[1, 2, 3]);
        assert_eq!((c.pc, c.steps, c.mode1), (0, 0, 0));
        assert_eq!(c.parcel_at(0), 0x0201);
        assert_eq!(c.parcel_at(1), 0x0003);
        assert_eq!(c.parcel_at(100), 0);
        assert!(c.site_hits().is_empty() || std::env::var_os(super::site_hits::ENV).is_some());
    }
}
