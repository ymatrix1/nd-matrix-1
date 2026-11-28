//! Front-panel presence and key matrix on EMIFA CS2 (`tools/arm_panel.py`, `tools/arm_keys.py`).
//!
//! The panel status ports `0x60000010` / `0x60000020` answer `0x80` ("present") inside the boot
//! detection routine and the key matrix afterwards (idle `0xFF`, a held key clears its bit). The
//! strobe store at `0x60000015` selects the column. Every other CS2 store is a display frame byte
//! and is only counted here (the OLED controller model is `ns4-lle6`).

use super::layout::Layout;
use std::collections::BTreeSet;

/// ASSUMED: front-panel chip select 2 window, strobe and status words (`tools/arm_panel.py`).
pub const CS2_BASE: u32 = 0x6000_0000;
pub const CS2_END: u32 = CS2_BASE + 0x1000;
pub const STROBE_ADDRESS: u32 = 0x6000_0015;
pub const PANEL_STATUS: [u32; 2] = [0x6000_0010, 0x6000_0020];
/// ASSUMED: a present key-matrix port reads `0x80` inside the detection routine.
pub const PRESENT: u8 = 0x80;

/// Key matrix: `(strobe, port, bit)` triples held low.
#[derive(Clone, Debug)]
pub struct KeyMatrix {
    /// EMPIRICAL: panel detection routine `[start, end)` (`tools/arm_panel.py`, from the
    /// `Layout`); the boot of `arm-lle-bench` needs "present" answers inside it.
    pub detect_range: (u32, u32),
    pub strobe: u8,
    pub held: BTreeSet<(u8, u8, u8)>,
    /// Stores to CS2 other than the strobe (display bytes).
    pub display_writes: u64,
}

impl Default for KeyMatrix {
    /// Detection range of OS v1.66; `Machine::attach_board` sets the one of the loaded OS.
    fn default() -> Self {
        Self {
            detect_range: Layout::V166.detect_range,
            strobe: 0,
            held: BTreeSet::new(),
            display_writes: 0,
        }
    }
}

impl KeyMatrix {
    pub fn press(&mut self, strobe: u8, port: u8, bit: u8) {
        self.held.insert((strobe, port, bit));
    }

    pub fn release(&mut self, strobe: u8, port: u8, bit: u8) {
        self.held.remove(&(strobe, port, bit));
    }

    /// Byte on `port` for the current strobe: all ones, held keys of that column cleared.
    pub fn port_byte(&self, port: u8) -> u8 {
        let mut v = 0xFFu8;
        for (s, p, b) in &self.held {
            if *s == self.strobe && *p == port {
                v &= !(1 << b);
            }
        }
        v
    }

    /// The four bytes of the status port at `base` for a read executed at `pc`
    /// (`arm_panel.panel_block`).
    pub fn block(&self, pc: u32, base: u32) -> [u8; 4] {
        if (self.detect_range.0..self.detect_range.1).contains(&pc) {
            return [PRESENT, 0, 0, 0];
        }
        let port = if base == PANEL_STATUS[0] { 0 } else { 1 };
        [self.port_byte(port), 0xFF, 0xFF, 0xFF]
    }

    /// A store into CS2 (`value` already masked to the store size).
    pub fn write(&mut self, address: u32, value: u32) {
        if address == STROBE_ADDRESS {
            self.strobe = value as u8;
        } else {
            self.display_writes += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_answers_present_only_inside_the_routine() {
        let k = KeyMatrix::default();
        assert_eq!(k.block(0xC024_D660, PANEL_STATUS[0])[0], PRESENT);
        assert_eq!(
            k.block(0xC000_0000, PANEL_STATUS[0]),
            [0xFF, 0xFF, 0xFF, 0xFF]
        );
    }

    #[test]
    fn held_key_clears_its_bit_only_on_its_strobe_and_port() {
        let mut k = KeyMatrix::default();
        k.press(0x02, 1, 3);
        k.write(STROBE_ADDRESS, 0x02);
        assert_eq!(k.port_byte(1), 0xF7);
        assert_eq!(k.port_byte(0), 0xFF);
        k.write(STROBE_ADDRESS, 0x04);
        assert_eq!(k.port_byte(1), 0xFF);
        k.release(0x02, 1, 3);
        k.write(STROBE_ADDRESS, 0x02);
        assert_eq!(k.port_byte(1), 0xFF);
        k.write(0x6000_0020, 5);
        assert_eq!(k.display_writes, 1);
    }
}
