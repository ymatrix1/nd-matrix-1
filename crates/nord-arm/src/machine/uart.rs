//! UART2 (TI 16550 receive side) that carries MIDI into the firmware (`tools/arm_keydown.py`).

use std::collections::VecDeque;

/// EMPIRICAL: UART2 base and AINTC interrupt 61 (ISR `0x80000434`, `tools/arm_keydown.py`);
/// MIDI notes sent through them reach the DSP ports in `arm-lle-bench`.
pub const UART2_BASE: u32 = 0x01D0_D000;
pub const UART2_IRQ: u32 = 61;
/// ASSUMED: TI 16550 register offsets and status values (UART TRM), matching the ISR's reads.
const LSR: u32 = 0x14;
const IIR: u32 = 0x08;
const RBR: u32 = 0x00;
/// ASSUMED: TI 16550 `LSR` `0x60` idle / `0x61` data ready; `IIR` `0x01` idle / `0x04` rx data.
const LSR_IDLE: u32 = 0x60;
const LSR_READY: u32 = 0x61;
const IIR_IDLE: u32 = 0x01;
const IIR_RX: u32 = 0x04;
/// EMPIRICAL: the board-present attach routine and the offset of the two "present" bytes in
/// its configuration object (`docs/research/keydown-paths.md` s1); without them no key reaches
/// the DSP ports in `arm-lle-bench`.
pub const BOARD_ATTACH: u32 = 0xC024_DB74;
pub const BOARD_PRESENT_OFFSET: u32 = 0x12;

/// Byte queue behind `RBR`, `LSR`, `IIR`.
#[derive(Clone, Default, Debug)]
pub struct Uart2 {
    pub queue: VecDeque<u8>,
    pub reads: u64,
}

impl Uart2 {
    /// Register read at `addr`; `None` when `addr` is not one of the three registers.
    pub fn read(&mut self, addr: u32) -> Option<u32> {
        match addr.wrapping_sub(UART2_BASE) {
            LSR => Some(if self.queue.is_empty() {
                LSR_IDLE
            } else {
                LSR_READY
            }),
            RBR => {
                self.reads += 1;
                Some(self.queue.pop_front().unwrap_or(0) as u32)
            }
            IIR => Some(if self.queue.is_empty() {
                IIR_IDLE
            } else {
                IIR_RX
            }),
            _ => None,
        }
    }

    pub fn push(&mut self, data: &[u8]) {
        self.queue.extend(data);
    }
}

pub fn note_on(note: u8, velocity: u8, channel: u8) -> [u8; 3] {
    [0x90 | (channel & 15), note & 0x7F, velocity & 0x7F]
}

pub fn note_off(note: u8, channel: u8) -> [u8; 3] {
    [0x80 | (channel & 15), note & 0x7F, 0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_follow_the_queue() {
        let mut u = Uart2::default();
        assert_eq!(u.read(UART2_BASE + LSR), Some(0x60));
        assert_eq!(u.read(UART2_BASE + IIR), Some(1));
        u.push(&note_on(60, 100, 0));
        assert_eq!(u.read(UART2_BASE + LSR), Some(0x61));
        assert_eq!(u.read(UART2_BASE + IIR), Some(4));
        assert_eq!(u.read(UART2_BASE), Some(0x90));
        assert_eq!(u.read(UART2_BASE), Some(60));
        assert_eq!(u.read(UART2_BASE), Some(100));
        assert_eq!(u.read(UART2_BASE), Some(0));
        assert_eq!(u.reads, 4);
        assert_eq!(u.read(UART2_BASE + 4), None);
    }

    #[test]
    fn midi_helpers_mask_their_fields() {
        assert_eq!(note_on(0xFF, 0xFF, 0x1F), [0x9F, 0x7F, 0x7F]);
        assert_eq!(note_off(60, 2), [0x82, 60, 0]);
    }
}
