//! OWNER: S0 (frozen). The bus view the superblock executor needs on top of [`Bus`]: the pc of
//! the accessing instruction (I5) and the "code went stale" probe (I6).

use crate::cpu::Bus;
use crate::machine::SysBus;

/// A [`Bus`] the trace executor can run on. `'static` because the interpreter's handler table is
/// generic over the bus type (`Table::<B>::ARM`).
pub trait SbBus: Bus + 'static {
    /// Record the guest address of the instruction about to access memory (`SysBus::pc`: device
    /// models, the poll breaker, probing and auto-mapping key on it).
    fn set_pc(&mut self, pc: u32);
    /// True when a store made decoded code stale (`Mem::stale_code` is not empty): the trace must
    /// end before its next uop (I6).
    fn stale(&self) -> bool;
}

impl SbBus for SysBus {
    #[inline(always)]
    fn set_pc(&mut self, pc: u32) {
        self.pc = pc;
    }

    #[inline(always)]
    fn stale(&self) -> bool {
        !self.mem.stale_code.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_bus_reports_the_pc_and_a_store_to_decoded_code() {
        let mut b = SysBus::new();
        b.set_pc(0xC000_0010);
        assert_eq!(b.pc, 0xC000_0010);
        b.mem.write32(0x5000, 1);
        b.mem.mark_code(0x5000);
        assert!(!b.stale());
        b.write32(0x5004, 2);
        assert!(b.stale());
        b.mem.stale_code.clear();
        assert!(!b.stale());
    }
}
