//! Full-board boot: panel, I2C EEPROMs, UART2 MIDI (`tools/arm_screens.py::Session`,
//! `tools/arm_keydown.py`, `tools/arm_panel.py::boot_with_panels`).

use super::boot::{APP_THUNK, CHUNK, MAIN_LOOP, TIMER_IRQ};
pub use super::grouper::Message;
use super::idle::IdleStats;
use super::panel::{KeyMatrix, WAIT_LOOP};
use super::uart::{Uart2, BOARD_ATTACH, UART2_IRQ};
use super::{Hook, Machine, Stop, SENTINEL, STACK_TOP};

impl Machine {
    /// `arm_screens.Session.__init__` models: key matrix, 4 EEPROMs (blank `0xFF`), UART2, and the
    /// board-present hook. `prepare` must have run with the UI stubs removed first, see
    /// [`Machine::prepare_system`].
    pub fn attach_board(&mut self) {
        self.bus.panel = Some(KeyMatrix::default());
        self.bus.i2c = Some(super::i2c::I2cBus::new());
        self.bus.uart2 = Some(Uart2::default());
        self.add_hook(BOARD_ATTACH, Hook::BoardPresent);
    }

    /// `prepare(display_stubs=False)` + board models: the display objects run for real.
    pub fn prepare_system(&mut self, code: &[u8], sram: &[u8]) -> Result<(), Stop> {
        self.load_firmware(code, sram);
        self.install_boot_stubs();
        // the panel link needs the display routines unstubbed (arm_boot_full.install_ui_stubs)
        self.hooks.remove(&0xC001_B67C);
        self.hooks.remove(&0xC001_B9D0);
        self.run_init_array()?;
        self.attach_board();
        Ok(())
    }

    /// Boot to the main loop, serving the wait loop with timer interrupts (idle skipping on).
    /// Returns `(instructions, interrupts injected, stop)`.
    pub fn boot_system(&mut self, max: u64, stats: &mut IdleStats) -> (u64, u64, Stop) {
        self.cpu.r[13] = STACK_TOP - 0x1000;
        self.cpu.r[14] = SENTINEL;
        self.cpu.thumb = false;
        self.cpu.r[15] = APP_THUNK;
        self.stop_pc = Some(MAIN_LOOP);
        let start = self.cpu.insns;
        let mut irqs = 0;
        let stop = loop {
            if self.cpu.insns - start >= max {
                break Stop::Budget;
            }
            let s = self.run_idle_skip(CHUNK, stats);
            self.bus.mem.zero(0, super::LOW_MEMORY as usize);
            if !matches!(s, Stop::Budget) {
                break s;
            }
            let pc = self.cpu.r[15];
            if (WAIT_LOOP.0..=WAIT_LOOP.1).contains(&pc) {
                self.inject_irq(TIMER_IRQ);
                irqs += 1;
            }
        };
        self.stop_pc = None;
        (self.cpu.insns - start, irqs, stop)
    }

    /// Queue MIDI bytes on UART2 and run its receive ISR (`Uart2.push`).
    pub fn send_midi(&mut self, data: &[u8]) {
        if let Some(u) = &mut self.bus.uart2 {
            u.push(data);
        }
        self.inject_irq(UART2_IRQ);
    }

    /// Move the messages completed since the last call into `out` (appended). Partial messages
    /// stay in the bus grouper and complete on a later call.
    pub fn drain_messages_into(&mut self, out: &mut Vec<Message>) {
        if let Some(r) = &mut self.dsp_return {
            r.sync(&self.bus.messages);
            r.reset_seen();
        }
        out.append(&mut self.bus.messages);
    }

    /// [`Machine::drain_messages_into`] into a fresh vector.
    pub fn drain_messages(&mut self) -> Vec<Message> {
        let mut v = Vec::new();
        self.drain_messages_into(&mut v);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::Bus;
    use crate::machine::sysbus::{PORT1_BASE, PORT2_BASE};

    #[test]
    fn a_message_split_across_two_drains_is_delivered_whole() {
        let mut m = Machine::new();
        m.bus.mem.map(PORT2_BASE, 0x2000);
        m.bus.mem.map(PORT1_BASE, 0x1000);
        m.install_dsp_return();
        m.bus.write32(PORT2_BASE + 0x10, 0x2800_0010); // comm 1: addr, pad, value
        m.bus.write32(PORT2_BASE + 0x10, 0);
        assert!(m.drain_messages().is_empty());
        m.bus.write32(PORT2_BASE + 0x10, 0xAB);
        let got = m.drain_messages();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].word1, got[0].word2), (0x2800_0010, 0xAB));
        // the DSP return shadow saw it exactly once
        let r = m.dsp_return.as_ref().unwrap();
        assert_eq!(r.shadow.get(&(1, 1, 4)), Some(&0xAB));
        // and the next message is aligned
        m.bus.write32(PORT1_BASE + 0x30, 5);
        m.bus.write32(PORT1_BASE + 0x30, 6);
        assert_eq!(m.drain_messages().len(), 1);
    }

    #[test]
    fn attach_board_registers_models_and_hook() {
        let mut m = Machine::new();
        m.attach_board();
        assert!(m.bus.panel.is_some() && m.bus.i2c.is_some() && m.bus.uart2.is_some());
        m.cpu.r[0] = 0x2000;
        m.cpu.r[15] = BOARD_ATTACH;
        m.bus.mem.write32(BOARD_ATTACH, 0xE1A0_0000);
        m.run(1, 0, None);
        assert_eq!(m.bus.mem.read8(0x2012), 1);
        assert_eq!(m.bus.mem.read8(0x2013), 1);
    }

    #[test]
    fn send_midi_queues_bytes_even_without_an_isr() {
        let mut m = Machine::new();
        m.attach_board();
        m.send_midi(&[0x90, 60, 100]);
        assert_eq!(m.bus.uart2.as_ref().unwrap().queue.len(), 3);
    }
}
