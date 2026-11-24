//! I2C0 controller and the four 24xx EEPROMs of the host board (`tools/arm_i2c.py`).
//!
//! Register-level model of the TI I2C block at `0x01C22000`: master transmit and receive with or
//! without a stop, the `ICSTR` ready flags and the 16 bit address write of random reads.
//! ASSUMED: AM1808 TRM register layout (corroborated by the driver's polling masks); EMPIRICAL:
//! four 64 KB devices at slave addresses `0x50..0x53` (the firmware reads a program back through
//! them in `arm-lle-bench`).

/// Entries kept in `I2cBus::log`.
pub const LOG_CAP: usize = 4096;
/// ASSUMED: TI AM1808 I2C0 base and register offsets (TRM); the firmware driver's accesses match.
pub const I2C_BASE: u32 = 0x01C2_2000;
pub const I2C_END: u32 = I2C_BASE + 0x1000;
pub const I2C_STR: u32 = 0x08;
pub const I2C_CNT: u32 = 0x14;
pub const I2C_DRR: u32 = 0x18;
pub const I2C_SAR: u32 = 0x1C;
pub const I2C_DXR: u32 = 0x20;
pub const I2C_MDR: u32 = 0x24;
const STR_NACK: u32 = 0x02;
const STR_ARDY: u32 = 0x04;
const STR_RRDY: u32 = 0x08;
const STR_XRDY: u32 = 0x10;
const STR_BB: u32 = 0x1000;
const MDR_STT: u32 = 0x2000;
const MDR_STP: u32 = 0x0800;
const MDR_MST: u32 = 0x0400;
const MDR_TRX: u32 = 0x0200;
pub const EEPROM_COUNT: usize = 4;
pub const EEPROM_SIZE: usize = 0x1_0000;
const EEPROM_BASE_ADDRESS: u32 = 0x50;
/// ASSUMED: 24LC512-style 128 byte write page.
const PAGE: u32 = 128;

#[derive(Clone)]
pub struct Eeprom {
    pub data: Vec<u8>,
    pointer: u32,
}

impl Eeprom {
    fn new() -> Self {
        Self {
            data: vec![0xFF; EEPROM_SIZE],
            pointer: 0,
        }
    }

    fn write_byte(&mut self, v: u8) {
        self.data[self.pointer as usize] = v;
        let page = self.pointer & !(PAGE - 1);
        self.pointer = page | ((self.pointer + 1) & (PAGE - 1));
    }

    fn read_byte(&mut self) -> u8 {
        let v = self.data[self.pointer as usize];
        self.pointer = (self.pointer + 1) & 0xFFFF;
        v
    }
}

#[derive(Clone)]
pub struct I2cBus {
    pub eeproms: Vec<Eeprom>,
    status: u32,
    sar: u32,
    cnt: u32,
    mdr: u32,
    drr: u32,
    busy: bool,
    tx: u32,
    rx_left: u32,
    addr_high: u32,
    slave: Option<usize>,
    /// Started transactions: `(slave, 1 = write | 0 = read, count)`.
    pub log: Vec<(u32, u8, u32)>,
}

impl Default for I2cBus {
    fn default() -> Self {
        Self::new()
    }
}

impl I2cBus {
    pub fn new() -> Self {
        Self {
            eeproms: (0..EEPROM_COUNT).map(|_| Eeprom::new()).collect(),
            status: STR_XRDY,
            sar: 0,
            cnt: 0,
            mdr: 0,
            drr: 0xFF,
            busy: false,
            tx: 0,
            rx_left: 0,
            addr_high: 0,
            slave: None,
            log: Vec::new(),
        }
    }

    /// Load the 256 KB image (four 64 KB devices back to back).
    pub fn load_image(&mut self, blob: &[u8]) {
        for (i, e) in self.eeproms.iter_mut().enumerate() {
            let part = blob.get(i * EEPROM_SIZE..).unwrap_or(&[]);
            let n = part.len().min(EEPROM_SIZE);
            e.data[..n].copy_from_slice(&part[..n]);
        }
    }

    /// Log one transaction (capped at `LOG_CAP`).
    fn note(&mut self, read: u8) {
        if self.log.len() < LOG_CAP {
            self.log.push((self.sar, read, self.cnt));
        }
    }

    fn release(&mut self) {
        self.busy = false;
        self.mdr &= !(MDR_MST | MDR_STP | MDR_STT);
        self.status &= !STR_BB;
    }

    fn finish(&mut self) {
        self.status |= STR_ARDY;
        self.status &= !(STR_XRDY | STR_RRDY);
        if self.mdr & MDR_STP != 0 {
            self.release();
        }
    }

    fn next_rx(&mut self) {
        match (self.rx_left > 0, self.slave) {
            (true, Some(s)) => {
                self.drr = self.eeproms[s].read_byte() as u32;
                self.status |= STR_RRDY;
            }
            _ => self.status &= !STR_RRDY,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        match offset {
            I2C_STR => self.status &= !(value & !STR_BB),
            I2C_CNT => self.cnt = value & 0xFFFF,
            I2C_SAR => self.sar = value & 0x3FF,
            I2C_DXR => self.transmit(value & 0xFF),
            I2C_MDR => self.set_mode(value),
            _ => {}
        }
    }

    fn set_mode(&mut self, value: u32) {
        let was_stop = value & MDR_STP != 0 && self.mdr & MDR_STP == 0;
        self.mdr = value & 0xFFFF;
        if value & MDR_STT != 0 {
            self.start();
        } else if was_stop && self.busy && self.rx_left == 0 && self.tx == 0 {
            self.release();
        }
    }

    fn start(&mut self) {
        self.busy = true;
        self.status |= STR_BB;
        self.status &= !(STR_ARDY | STR_NACK | STR_RRDY | STR_XRDY);
        let i = self.sar.wrapping_sub(EEPROM_BASE_ADDRESS) as usize;
        self.slave = (i < EEPROM_COUNT).then_some(i);
        self.tx = 0;
        if self.slave.is_none() {
            self.status |= STR_NACK | STR_ARDY;
            self.release();
            return;
        }
        if self.mdr & MDR_TRX != 0 {
            self.note(1);
            self.rx_left = 0;
            if self.cnt == 0 {
                self.finish();
            } else {
                self.status |= STR_XRDY;
            }
        } else {
            self.note(0);
            self.rx_left = self.cnt;
            self.next_rx();
            if self.rx_left == 0 {
                self.finish();
            }
        }
    }

    fn transmit(&mut self, byte: u32) {
        let Some(s) = self.slave else { return };
        if !self.busy || self.mdr & MDR_TRX == 0 {
            return;
        }
        match self.tx {
            0 => self.addr_high = byte,
            1 => self.eeproms[s].pointer = ((self.addr_high << 8) | byte) & 0xFFFF,
            _ => self.eeproms[s].write_byte(byte as u8),
        }
        self.tx += 1;
        if self.tx >= self.cnt {
            self.finish();
        }
    }

    /// Register read; `None` for registers without a model (plain memory).
    pub fn read(&mut self, offset: u32) -> Option<u32> {
        match offset {
            I2C_STR => Some(self.status),
            I2C_DRR => Some(self.receive()),
            I2C_MDR => Some(self.mdr),
            I2C_CNT => Some(self.cnt),
            _ => None,
        }
    }

    fn receive(&mut self) -> u32 {
        let v = self.drr;
        if self.rx_left > 0 {
            self.rx_left -= 1;
            self.status &= !STR_RRDY;
            if self.rx_left == 0 {
                self.finish();
            } else {
                self.next_rx();
            }
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_block(bus: &mut I2cBus, slave: u32, addr: u16, data: &[u8]) {
        bus.write(I2C_SAR, slave);
        bus.write(I2C_CNT, 2 + data.len() as u32);
        bus.write(I2C_MDR, MDR_STT | MDR_STP | MDR_MST | MDR_TRX);
        bus.write(I2C_DXR, (addr >> 8) as u32);
        bus.write(I2C_DXR, addr as u32 & 0xFF);
        for b in data {
            bus.write(I2C_DXR, *b as u32);
        }
    }

    #[test]
    fn write_then_random_read_round_trips_through_the_eeprom() {
        let mut bus = I2cBus::new();
        write_block(&mut bus, 0x51, 0x1234, &[0xDE, 0xAD]);
        assert_eq!(bus.eeproms[1].data[0x1234..0x1236], [0xDE, 0xAD]);
        assert_eq!(
            bus.read(I2C_STR).unwrap() & STR_BB,
            0,
            "stop releases the bus"
        );
        bus.write(I2C_SAR, 0x51);
        bus.write(I2C_CNT, 2);
        bus.write(I2C_MDR, MDR_STT | MDR_MST | MDR_TRX);
        bus.write(I2C_DXR, 0x12);
        bus.write(I2C_DXR, 0x34);
        bus.write(I2C_CNT, 2);
        bus.write(I2C_MDR, MDR_STT | MDR_STP | MDR_MST);
        assert_eq!(bus.read(I2C_DRR), Some(0xDE));
        assert_eq!(bus.read(I2C_DRR), Some(0xAD));
        assert_ne!(bus.read(I2C_STR).unwrap() & STR_ARDY, 0);
    }

    #[test]
    fn unknown_slave_nacks_and_page_writes_wrap() {
        let mut bus = I2cBus::new();
        bus.write(I2C_SAR, 0x60);
        bus.write(I2C_CNT, 1);
        bus.write(I2C_MDR, MDR_STT | MDR_STP | MDR_MST | MDR_TRX);
        assert_ne!(bus.read(I2C_STR).unwrap() & STR_NACK, 0);
        let data: Vec<u8> = (0..4).collect();
        write_block(&mut bus, 0x50, 0x007F, &data);
        assert_eq!(bus.eeproms[0].data[0x7F], 0);
        assert_eq!(bus.eeproms[0].data[0x00], 1);
    }

    #[test]
    fn image_loading_splits_into_four_devices() {
        let mut bus = I2cBus::new();
        let mut img = vec![0u8; EEPROM_COUNT * EEPROM_SIZE];
        for (i, b) in img.iter_mut().step_by(EEPROM_SIZE).enumerate() {
            *b = i as u8 + 1;
        }
        bus.load_image(&img);
        assert_eq!(bus.eeproms[3].data[0], 4);
        // a short image fills what it has and does not panic
        let mut short = I2cBus::new();
        short.load_image(&img[..EEPROM_SIZE + 5]);
        assert_eq!(short.eeproms[1].data[0], 2);
        assert_eq!(short.eeproms[2].data[0], 0xFF);
    }
}
