//! OWNER: W1-VGEN. The board models of the system bus as the firmware can observe them: front
//! panel, I2C controller with its EEPROMs, and UART2. Part of the machine snapshot.

use crate::machine::i2c::{I2cBus, I2C_CNT, I2C_DRR, I2C_MDR, I2C_STR};
use crate::machine::panel::KeyMatrix;
use crate::machine::uart::Uart2;

/// Front-panel key matrix state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PanelSnap {
    pub strobe: u8,
    pub held: Vec<(u8, u8, u8)>,
    pub display_writes: u64,
}

/// I2C controller state as the firmware can observe it: the log of started transactions, the
/// readable registers (`STR`, `MDR`, `CNT`, and what `DRR` would return, read on a copy so the
/// receive side effect does not touch the machine) and a hash of the four EEPROMs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct I2cSnap {
    pub log: Vec<(u32, u8, u32)>,
    pub regs: [Option<u32>; 4],
    pub eeprom_hash: u64,
}

impl I2cSnap {
    pub fn of(bus: &I2cBus) -> Self {
        let mut probe = bus.clone();
        let regs = [I2C_STR, I2C_MDR, I2C_CNT, I2C_DRR].map(|r| probe.read(r));
        let mut h: u64 = 0xCBF2_9CE4_8422_2325;
        for byte in bus.eeproms.iter().flat_map(|e| e.data.iter()) {
            h = (h ^ *byte as u64).wrapping_mul(0x100_0000_01B3);
        }
        Self {
            log: bus.log.clone(),
            regs,
            eeprom_hash: h,
        }
    }
}

/// UART2 receive queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UartSnap {
    pub queue: Vec<u8>,
    pub reads: u64,
}

impl PanelSnap {
    pub fn of(p: &KeyMatrix) -> Self {
        Self {
            strobe: p.strobe,
            held: p.held.iter().copied().collect(),
            display_writes: p.display_writes,
        }
    }
}

impl UartSnap {
    pub fn of(u: &Uart2) -> Self {
        Self {
            queue: u.queue.iter().copied().collect(),
            reads: u.reads,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::i2c::{I2C_BASE, I2C_SAR};
    use crate::machine::sb::testkit::snapshot;
    use crate::machine::Machine;

    /// A change of one field, applied to a copy to prove the comparator sees it.
    type Mutation<T> = Box<dyn Fn(&mut T)>;

    #[test]
    fn the_board_models_are_compared_through_their_observable_state() {
        let mut m = Machine::new();
        crate::machine::sb::testkit::state::attach_boards(&mut m.bus);
        let base = snapshot(&m, None);
        assert!(base.panel.is_some() && base.i2c.is_some() && base.uart2.is_some());
        let live: Vec<(&str, Mutation<Machine>)> = vec![
            (
                "panel",
                Box::new(|m| m.bus.panel.as_mut().unwrap().strobe = 2),
            ),
            (
                "panel",
                Box::new(|m| m.bus.panel.as_mut().unwrap().press(1, 0, 3)),
            ),
            (
                "panel",
                Box::new(|m| m.bus.panel.as_mut().unwrap().display_writes = 5),
            ),
            (
                "uart2",
                Box::new(|m| m.bus.uart2.as_mut().unwrap().push(&[0x90])),
            ),
            (
                "uart2",
                Box::new(|m| m.bus.uart2.as_mut().unwrap().reads = 3),
            ),
            (
                "i2c",
                Box::new(|m| m.bus.i2c.as_mut().unwrap().eeproms[2].data[9] = 0),
            ),
            (
                "i2c",
                Box::new(|m| m.bus.i2c.as_mut().unwrap().log.push((1, 1, 1))),
            ),
            (
                "i2c",
                Box::new(|m| m.bus.i2c.as_mut().unwrap().write(I2C_CNT, 3)),
            ),
            (
                "i2c",
                Box::new(|m| {
                    let b = m.bus.i2c.as_mut().unwrap();
                    b.write(I2C_SAR, 0x50);
                    b.write(I2C_CNT, 1);
                    b.write(I2C_MDR, 0x2000 | 0x0400); // start a read
                }),
            ),
        ];
        for (name, mutate) in live {
            let mut m2 = m.clone();
            mutate(&mut m2);
            let d = base
                .diff(&snapshot(&m2, None))
                .unwrap_or_else(|| panic!("{name} state not compared"));
            assert!(d.contains(name), "{name}: {d}");
        }
        assert_eq!(I2C_BASE, 0x01C2_2000);
        assert_eq!(base.diff(&snapshot(&m.clone(), None)), None);
    }
}
