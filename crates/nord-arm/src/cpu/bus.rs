//! The memory interface the interpreter runs against.

/// Guest bus. Device hooks, auto-mapping and fetch hooks live behind this trait.
pub trait Bus {
    fn read8(&mut self, addr: u32) -> u8;
    fn read16(&mut self, addr: u32) -> u16;
    fn read32(&mut self, addr: u32) -> u32;
    fn write8(&mut self, addr: u32, v: u8);
    fn write16(&mut self, addr: u32, v: u16);
    fn write32(&mut self, addr: u32, v: u32);
    fn fetch16(&mut self, addr: u32) -> u16 {
        self.read16(addr)
    }
    fn fetch32(&mut self, addr: u32) -> u32 {
        self.read32(addr)
    }
}

#[cfg(test)]
pub use self::flat::FlatBus;

#[cfg(test)]
mod flat {
    use super::Bus;
    use crate::mem::Mem;

    /// Plain memory bus for unit tests (no hooks).
    #[derive(Default)]
    pub struct FlatBus(pub Mem);

    impl Bus for FlatBus {
        fn read8(&mut self, a: u32) -> u8 {
            self.0.read8(a)
        }
        fn read16(&mut self, a: u32) -> u16 {
            self.0.read16(a)
        }
        fn read32(&mut self, a: u32) -> u32 {
            self.0.read32(a)
        }
        fn write8(&mut self, a: u32, v: u8) {
            self.0.write8(a, v)
        }
        fn write16(&mut self, a: u32, v: u16) {
            self.0.write16(a, v)
        }
        fn write32(&mut self, a: u32, v: u32) {
            self.0.write32(a, v)
        }
    }

    #[test]
    fn flat_bus_forwards_to_mem() {
        let mut b = FlatBus::default();
        b.write32(0x100, 0x0102_0304);
        assert_eq!(b.read8(0x100), 4);
        assert_eq!(b.fetch16(0x102), 0x0102);
        assert_eq!(b.fetch32(0x100), 0x0102_0304);
    }
}
