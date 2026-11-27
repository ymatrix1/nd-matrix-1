//! Idle-probe instrumentation of the system bus (`ns4-lle2` follow-up 1).
//!
//! While [`SysBus::probing`] is set, every load and store reports to [`SysBus::impure`] when it
//! could make the probe invalid: a device model would run, the page is unmapped, or a store
//! changes memory. The old design wrapped the bus in a separate `Probe` type; keeping it one type
//! lets the probe run on the predecoded handlers (which are specialised for `SysBus`).

use super::sysbus::SysBus;
use crate::mem::{HOOK_R, HOOK_W};

impl SysBus {
    /// A load of `a` while probing.
    #[cold]
    #[inline(never)]
    pub(super) fn probe_read(&mut self, a: u32) {
        self.probe_watch(a, HOOK_R);
    }

    /// A store of `v` (already truncated to its width `bytes`) to `a` while probing.
    #[cold]
    #[inline(never)]
    pub(super) fn probe_write(&mut self, a: u32, v: u32, bytes: u32) {
        self.probe_watch(a, HOOK_W);
        if !self.impure {
            let old = match bytes {
                1 => self.mem.read8(a) as u32,
                2 => self.mem.read16(a) as u32,
                _ => self.mem.read32(a),
            };
            if old != v {
                self.impure = true;
            }
        }
    }

    fn probe_watch(&mut self, a: u32, flag: u32) {
        let f = self.mem.flags(a) & flag;
        if (f != 0 && self.has_side_effect(a, flag == HOOK_W)) || !self.mem.is_mapped(a) {
            self.impure = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::Bus;

    #[test]
    fn probing_flags_devices_unmapped_pages_and_changing_stores_only() {
        let mut b = SysBus::new();
        b.install_devices();
        b.mem.map(0x8000_0000, 0x1000);
        b.mem.write32(0x8000_0000, 7);
        b.probing = true;
        b.read32(0x8000_0000);
        b.write32(0x8000_0000, 7); // same value
        b.write8(0x8000_0000, 7);
        assert!(!b.impure);
        b.write32(0x8000_0000, 8);
        assert!(b.impure);
        b.impure = false;
        b.read32(0xC123_0000); // unmapped
        assert!(b.impure);
        b.impure = false;
        b.read32(0x6000_0000); // device window
        assert!(b.impure);
        b.impure = false;
        b.probing = false;
        b.write32(0x8000_0000, 9); // not probing: no report
        assert!(!b.impure);
    }
}
