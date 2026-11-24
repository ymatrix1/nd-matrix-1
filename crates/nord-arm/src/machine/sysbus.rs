//! System bus: guest memory plus the device models that watch it (no callbacks, no allocation per
//! access). Ports `tools/arm_boot.py` (`install_tick`, `install_gpio_echo`), `tools/arm_emu.py`
//! (`enable_poll_breaker`, the fake DSP ports) with the same constants.

use super::grouper::{Grouper, Message};
use super::i2c::{I2cBus, I2C_BASE, I2C_END};
use super::panel::{KeyMatrix, CS2_BASE, CS2_END, PANEL_STATUS};
use super::uart::Uart2;
use crate::cpu::Bus;
use crate::mem::{Mem, HOOK_R, HOOK_W};
use std::collections::HashMap;

/// ASSUMED: address of the 64 bit RTOS tick read by the `ldrd` delay loop at `0xC0080684`
/// (`tools/arm_boot.py` TICK_ADDRESS); confirm by tracing that loop on a boot.
pub const TICK_ADDRESS: u32 = 0x8000_1DC0;
/// ASSUMED: TI AM1808 GPIO block, bank stride and count (`tools/arm_boot.py`).
pub const GPIO_BASE: u32 = 0x01E2_6000;
const GPIO_BANK_STRIDE: u32 = 0x28;
const GPIO_BANKS: u32 = 5;
/// Fake DSP port block (harness-only addresses, `tools/arm_emu.py`).
pub const FAKE_BASE: u32 = 0xA000_0000;
pub const PORT2_BASE: u32 = 0xA001_0000;
pub const PORT1_BASE: u32 = 0xA001_1000;
pub const COMM_COUNT: u32 = 5;
const PORT_END: u32 = PORT1_BASE + 0x10 * COMM_COUNT;
/// ASSUMED: AM1705 peripheral windows polled by firmware wait loops (`tools/arm_xref.py`).
pub const DEVICE_WINDOWS: [(u32, u32); 3] = [
    (0x01C0_0000, 0x01F0_0000),
    (0x6000_0000, 0x6A00_0000),
    (0xB000_0000, 0xB001_0000),
];
/// ASSUMED: values tried, in order, on a polled device word until a firmware wait loop exits.
pub const POLL_VALUES: [u32; 8] = [
    0xFFFF_FFFF,
    0,
    3,
    1,
    0x3F,
    0x10001,
    0x8000_0000,
    0x0F0F_0F0F,
];
const POLL_LIMIT: u32 = 40;

/// Most messages the bus keeps between two drains; later ones are counted in
/// [`SysBus::dropped_messages`], not stored (the caller drains every tick).
pub const MESSAGE_CAP: usize = 1 << 18;

/// Memory plus device state.
#[derive(Clone)]
pub struct SysBus {
    pub mem: Mem,
    /// Address of the executing instruction (set by the run loop before each step).
    pub pc: u32,
    /// Increment of the 64 bit tick per read; `0` once the real timer interrupt runs.
    pub tick_step: u64,
    pub tick_t: u64,
    pub gpio_writes: u64,
    poll_counts: HashMap<(u32, u32), u32>,
    pub poll_hits: u64,
    /// Grouping state of the port stores (survives drains).
    pub grouper: Grouper,
    /// Completed ARM to DSP messages since the last drain.
    pub messages: Vec<Message>,
    pub dropped_messages: u64,
    /// Reads that reached a device window (profiling).
    pub device_reads: u64,
    /// Optional board models; `None` leaves the page to the poll breaker.
    pub panel: Option<KeyMatrix>,
    pub i2c: Option<I2cBus>,
    pub uart2: Option<Uart2>,
}

impl Default for SysBus {
    fn default() -> Self {
        Self::new()
    }
}

impl SysBus {
    pub fn new() -> Self {
        Self {
            mem: Mem::new(),
            pc: 0,
            tick_step: 5000,
            tick_t: 0,
            gpio_writes: 0,
            poll_counts: HashMap::new(),
            poll_hits: 0,
            grouper: Grouper::default(),
            messages: Vec::new(),
            dropped_messages: 0,
            device_reads: 0,
            panel: None,
            i2c: None,
            uart2: None,
        }
    }

    /// Flag the pages of every device model (tick, poll windows, GPIO, DSP ports).
    pub fn install_devices(&mut self) {
        self.mem.add_flag(TICK_ADDRESS, TICK_ADDRESS + 7, HOOK_R);
        for (lo, hi) in DEVICE_WINDOWS {
            self.mem.add_flag(lo, hi - 1, HOOK_R);
        }
        self.mem.add_flag(GPIO_BASE, GPIO_BASE + 0xFF, HOOK_W);
        self.mem.add_flag(PORT2_BASE, PORT_END - 1, HOOK_W);
        self.mem.add_flag(I2C_BASE, I2C_END - 1, HOOK_W);
        self.mem.add_flag(CS2_BASE, CS2_END - 1, HOOK_W);
    }

    #[cold]
    fn on_read(&mut self, addr: u32) {
        if (TICK_ADDRESS..TICK_ADDRESS + 8).contains(&addr) && self.tick_step != 0 {
            self.tick_t = self.tick_t.wrapping_add(self.tick_step);
            let t = self.tick_t;
            self.mem.write_bytes(TICK_ADDRESS, &t.to_le_bytes());
        }
        if DEVICE_WINDOWS
            .iter()
            .any(|(lo, hi)| (*lo..*hi).contains(&addr))
        {
            self.device_reads += 1;
            let key = (self.pc, addr & !3);
            let n = self.poll_counts.entry(key).or_insert(0);
            *n += 1;
            if *n > POLL_LIMIT {
                let v = POLL_VALUES[((*n - POLL_LIMIT) / 8) as usize % POLL_VALUES.len()];
                self.poll_hits += 1;
                if self.mem.is_mapped(addr) {
                    self.mem.write32(addr & !3, v);
                }
            }
        }
        self.board_read(addr);
    }

    /// Board models that overwrite the polled word (they run after the poll breaker and win).
    fn board_read(&mut self, addr: u32) {
        let base = addr & !3;
        if let Some(k) = &self.panel {
            if (CS2_BASE..CS2_END).contains(&addr) && PANEL_STATUS.contains(&base) {
                let b = k.block(self.pc, base);
                self.mem.write32(base, u32::from_le_bytes(b));
            }
        }
        if let Some(bus) = &mut self.i2c {
            if (I2C_BASE..I2C_END).contains(&addr) {
                if let Some(v) = bus.read(base - I2C_BASE) {
                    self.mem.write32(base, v);
                }
            }
        }
        if let Some(u) = &mut self.uart2 {
            if let Some(v) = u.read(addr) {
                self.mem.write32(base, v);
            }
        }
    }

    #[cold]
    fn on_write(&mut self, addr: u32, value: u32) {
        if (PORT2_BASE..PORT_END).contains(&addr) {
            if let Some(m) = self.grouper.push(addr & !0xF, value, self.pc) {
                if self.messages.len() < MESSAGE_CAP {
                    self.messages.push(m);
                } else {
                    self.dropped_messages += 1;
                }
            }
        }
        if let Some(bus) = &mut self.i2c {
            if (I2C_BASE..I2C_END).contains(&addr) {
                bus.write((addr - I2C_BASE) & !3, value);
            }
        }
        if let Some(k) = &mut self.panel {
            if (CS2_BASE..CS2_END).contains(&addr) {
                k.write(addr, value);
            }
        }
        let rel = addr.wrapping_sub(GPIO_BASE).wrapping_sub(0x10);
        if (GPIO_BASE + 0x10..GPIO_BASE + 0x100).contains(&addr) {
            let (bank, reg) = (rel / GPIO_BANK_STRIDE, rel % GPIO_BANK_STRIDE);
            if bank < GPIO_BANKS && (reg == 0x8 || reg == 0xC) {
                self.gpio_writes += 1;
                let level = if reg == 0x8 { 0xFFFF_FFFFu32 } else { 0 };
                for b in 0..GPIO_BANKS {
                    self.mem
                        .write32(GPIO_BASE + 0x10 + GPIO_BANK_STRIDE * b + 0x10, level);
                }
            }
        }
    }

    /// True when a read (`write` false) or write of `addr` would run a device model right now.
    /// The tick is only a device while `tick_step` is non-zero.
    pub fn has_side_effect(&self, addr: u32, write: bool) -> bool {
        if write {
            (PORT2_BASE..PORT_END).contains(&addr)
                || (I2C_BASE..I2C_END).contains(&addr)
                || (CS2_BASE..CS2_END).contains(&addr)
                || (GPIO_BASE + 0x10..GPIO_BASE + 0x100).contains(&addr)
        } else {
            ((TICK_ADDRESS..TICK_ADDRESS + 8).contains(&addr) && self.tick_step != 0)
                || DEVICE_WINDOWS
                    .iter()
                    .any(|(lo, hi)| (*lo..*hi).contains(&addr))
        }
    }

    /// Distinct `(pc, word)` poll sites that crossed the poll limit (for tests and reports).
    pub fn poll_sites(&self) -> usize {
        self.poll_counts
            .values()
            .filter(|n| **n > POLL_LIMIT)
            .count()
    }
}

macro_rules! rd {
    ($self:ident, $a:ident, $f:ident) => {{
        if $self.mem.flags($a) & HOOK_R != 0 {
            $self.on_read($a);
        }
        $self.mem.$f($a)
    }};
}

macro_rules! wr {
    ($self:ident, $a:ident, $v:ident, $f:ident, $mask:expr) => {{
        if $self.mem.flags($a) & HOOK_W != 0 {
            $self.on_write($a, $v as u32 & $mask);
        }
        $self.mem.$f($a, $v)
    }};
}

impl Bus for SysBus {
    #[inline(always)]
    fn read8(&mut self, a: u32) -> u8 {
        rd!(self, a, read8)
    }
    #[inline(always)]
    fn read16(&mut self, a: u32) -> u16 {
        rd!(self, a, read16)
    }
    #[inline(always)]
    fn read32(&mut self, a: u32) -> u32 {
        rd!(self, a, read32)
    }
    #[inline(always)]
    fn write8(&mut self, a: u32, v: u8) {
        wr!(self, a, v, write8, 0xFF)
    }
    #[inline(always)]
    fn write16(&mut self, a: u32, v: u16) {
        wr!(self, a, v, write16, 0xFFFF)
    }
    #[inline(always)]
    fn write32(&mut self, a: u32, v: u32) {
        wr!(self, a, v, write32, 0xFFFF_FFFF)
    }
    #[inline(always)]
    fn fetch16(&mut self, a: u32) -> u16 {
        self.mem.read16(a)
    }
    #[inline(always)]
    fn fetch32(&mut self, a: u32) -> u32 {
        self.mem.read32(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus() -> SysBus {
        let mut b = SysBus::new();
        b.mem.map(0x8000_0000, 0x10_0000);
        b.install_devices();
        b
    }

    #[test]
    fn side_effect_query_matches_the_models() {
        let mut b = bus();
        assert!(b.has_side_effect(TICK_ADDRESS, false));
        b.tick_step = 0;
        assert!(!b.has_side_effect(TICK_ADDRESS, false));
        assert!(b.has_side_effect(0x6000_0004, false) && b.has_side_effect(PORT2_BASE, true));
        assert!(!b.has_side_effect(0x8000_0000, true) && !b.has_side_effect(0xC000_0000, false));
    }

    #[test]
    fn tick_advances_per_read_until_disabled() {
        let mut b = bus();
        assert_eq!(b.read32(TICK_ADDRESS), 5000);
        assert_eq!(b.read32(TICK_ADDRESS), 10_000);
        b.tick_step = 0;
        assert_eq!(b.read32(TICK_ADDRESS), 10_000);
    }

    #[test]
    fn poll_breaker_rotates_values_after_the_limit() {
        let mut b = bus();
        b.mem.map(0x6000_0000, 0x1000);
        b.pc = 0xC000_0100;
        let first: Vec<u32> = (0..41).map(|_| b.read32(0x6000_0010)).collect();
        assert!(first[..40].iter().all(|v| *v == 0));
        assert_eq!(first[40], POLL_VALUES[0]);
        assert_eq!(b.poll_sites(), 1);
        // 8 more hits rotate to the next value
        for _ in 0..7 {
            b.read32(0x6000_0010);
        }
        assert_eq!(b.read32(0x6000_0010), POLL_VALUES[1]);
    }

    #[test]
    fn dsp_port_stores_group_into_messages_with_pc_and_masked_value() {
        let mut b = bus();
        b.mem.map(PORT2_BASE, 0x2000);
        b.pc = 0xC000_0200;
        // comm 3 (path 1, two words): the low byte store masks to the port address
        b.write32(PORT1_BASE + 0x30, 0x1234_5678);
        b.write8(PORT1_BASE + 0x31, 0xFF);
        assert_eq!(
            b.messages,
            vec![Message {
                path: 1,
                comm: 3,
                word1: 0x1234_5678,
                word2: 0xFF,
                pc: 0xC000_0200
            }]
        );
    }

    #[test]
    fn gpio_set_and_clear_echo_into_every_input_register() {
        let mut b = bus();
        b.mem.map(GPIO_BASE, 0x1000);
        b.write32(GPIO_BASE + 0x10 + 0x8, 1);
        assert_eq!(
            b.mem.read32(GPIO_BASE + 0x10 + GPIO_BANK_STRIDE * 3 + 0x10),
            0xFFFF_FFFF
        );
        b.write32(GPIO_BASE + 0x10 + 0xC, 1);
        assert_eq!(b.mem.read32(GPIO_BASE + 0x10 + 0x10), 0);
        assert_eq!(b.gpio_writes, 2);
    }
}
