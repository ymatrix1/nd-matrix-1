//! Firmware load, boot stubs and the run to the main loop.
//!
//! Port of `tools/arm_emu.py` (`ArmEmu.__post_init__`, `install_dsp_ports`),
//! `tools/arm_boot.py::install_boot_stubs`, `tools/arm_boot_full.py` (`install_ui_stubs`,
//! `run_init_array`, `prepare`) and `tools/arm_steady.py` (`run_until`, `boot_to_main_loop`).
//! Addresses are inherited from those scripts; each carries its own tag below.

use super::layout::Layout;
use super::sysbus::{COMM_COUNT, FAKE_BASE, PORT1_BASE, PORT2_BASE};
use super::{Hook, Machine, Stop, LOW_MEMORY, SENTINEL, STACK_TOP};
use crate::cpu::Bus;

/// ASSUMED: ARM load address of `bank0_CODE.bin` (`tools/arm_emu.py` LOAD_BASE); the boot in
/// `arm-lle-bench` reaches the main loop from it, which would fail for a wrong base.
pub const LOAD_BASE: u32 = 0xC000_0000;
/// ASSUMED: `bank0_SRAM.bin` image address (`tools/arm_emu.py`), same check as `LOAD_BASE`.
pub const SRAM_BASE: u32 = 0x8000_0000;
/// ASSUMED: 16 MB mapped over the code image (`tools/arm_emu.py`).
const RAM_SIZE: u32 = 0x0100_0000;
/// ASSUMED: size of the fake DSP object/port region (`tools/arm_emu.py`).
const FAKE_SIZE: u32 = 0x2_0000;
/// ASSUMED: the comm objects pad path 2 messages with a zero word for chips 0..=2 (FX, PNO, ORG).
pub const PAD: [u8; 5] = [1, 1, 1, 0, 0];
// The OS-version dependent addresses (flow control, comm table, `operator new`, the stubbed
// functions, init array, application thunk, main loop, AINTC table) are in `layout.rs`.
/// ASSUMED: high exception vector page mapped by the harness.
const HIGH_PAGE: u32 = 0xFFFF_0000;
/// EMPIRICAL (D130, `tools/arm_tick_probe.py timers`): system interrupt 22 is the 24 kHz timer.
pub const TIMER_IRQ: u32 = 22;
/// ASSUMED: base of the bump allocator heap (`tools/arm_emu.py`); unused address space.
const HEAP_BASE: u32 = 0x9000_0000;
/// ASSUMED: 200,000 ARM instructions per 1 ms virtual tick (`tools/arm_steady.py`); see
/// `docs/research/arm0-rust-runtime.md`.
pub const CHUNK: u64 = 200_000;

impl Machine {
    /// Map the harness regions and load the firmware (`ArmEmu.__post_init__`).
    pub fn load_firmware(&mut self, code: &[u8], sram: &[u8]) {
        let m = &mut self.bus.mem;
        m.map(LOAD_BASE, RAM_SIZE);
        m.write_bytes(LOAD_BASE, code);
        m.map(SRAM_BASE, 0x10_0000);
        m.write_bytes(SRAM_BASE, sram);
        m.map(STACK_TOP - 0x10_0000, 0x20_0000);
        m.map(FAKE_BASE, FAKE_SIZE);
        self.layout = Layout::detect(code).unwrap_or(self.layout);
        self.install_dsp_ports();
    }

    /// Build the fake DSP comm objects (`ArmEmu.install_dsp_ports`); call again after anything
    /// that clears BSS (the static initialisers do).
    pub fn install_dsp_ports(&mut self) {
        for c in 0..COMM_COUNT {
            self.write32(FAKE_BASE + 0x40 + 4 * c, PORT2_BASE + 0x10 * c);
            self.bus.mem.write8(FAKE_BASE + 0x90 + c, PAD[c as usize]);
            let obj = FAKE_BASE + 0x1000 + 0x100 * c;
            self.write32(obj + 0x10, PORT1_BASE + 0x10 * c);
            self.write32(obj + 0x20, FAKE_BASE + 0x2000 + 0x40 * c);
            self.write32(self.layout.comm_table + 4 * (c + 1), obj);
        }
        self.write32(self.layout.port_object_ptr, FAKE_BASE);
        self.add_hook(self.layout.flow_control, Hook::Return(1));
    }

    /// `install_boot_stubs` + `install_ui_stubs(display_stubs=True)` of the Python harness.
    pub fn install_boot_stubs(&mut self) {
        self.guard_null_calls();
        self.heap_top = HEAP_BASE;
        self.add_hook(self.layout.operator_new, Hook::BumpNew);
        self.add_hook(self.layout.mmu_enable_mcr, Hook::SkipInsn);
        self.add_hook(self.layout.dsp_download, Hook::Return(0));
        self.add_hook(self.layout.spi_download_block, Hook::Return(0));
        self.add_hook(self.layout.logger, Hook::Return(0));
        self.add_hook(self.layout.board_config_query, Hook::BoardConfig);
        self.add_hook(self.layout.event_insert, Hook::Return(0));
        self.add_hook(self.layout.spi_transfer, Hook::SpiTransfer);
        self.add_hook(self.layout.spi_count_check, Hook::ZeroR5);
        self.add_hook(self.layout.text_measure, Hook::Return(0));
        self.bus.mem.map(HIGH_PAGE, 0x1_0000);
    }

    /// Run the four static initialisers for real (`run_init_array`).
    pub fn run_init_array(&mut self) -> Result<(), Stop> {
        for f in self.layout.init_array {
            let (_, stop) = self.call(f, &[], 300_000_000);
            if stop != Stop::Until {
                return Err(stop);
            }
        }
        self.install_dsp_ports();
        Ok(())
    }

    /// `arm_boot_full.prepare`: firmware + stubs + static initialisers.
    pub fn prepare(&mut self, code: &[u8], sram: &[u8]) -> Result<(), Stop> {
        self.load_firmware(code, sram);
        self.install_boot_stubs();
        self.run_init_array()
    }

    /// `arm_steady.run_until`: run in chunks of `chunk` instructions, zeroing the null-guard page
    /// between chunks, until `stop_pc` is about to execute. Returns `(instructions budgeted, stop)`.
    pub fn run_chunked(&mut self, stop_pc: Option<u32>, max: u64, chunk: u64) -> (u64, Stop) {
        let mut done = 0u64;
        let mut stop = Stop::Budget;
        while done < max {
            let (_, s) = self.run_wfi(chunk.min(max - done), SENTINEL, stop_pc);
            stop = s;
            done += chunk;
            self.bus.mem.zero(0, LOW_MEMORY as usize);
            if !matches!(s, Stop::Budget) {
                break;
            }
        }
        (done.min(max), stop)
    }

    /// `arm_steady.boot_to_main_loop`.
    pub fn boot_to_main_loop(&mut self, max: u64) -> (u64, Stop) {
        self.cpu.r[13] = STACK_TOP - 0x1000;
        self.cpu.r[14] = SENTINEL;
        self.cpu.thumb = false;
        self.cpu.r[15] = self.layout.app_thunk;
        self.run_chunked(Some(self.layout.main_loop), max, CHUNK)
    }

    /// `arm_steady.enable_real_timer`: stop the +5000-per-read tick, the injected IRQ counts.
    pub fn enable_real_timer(&mut self) {
        self.bus.tick_step = 0;
    }

    /// `arm_steady.run_ticking` for one tick: `insns` instructions, then the timer interrupt.
    pub fn run_tick(&mut self, insns: u64) -> Stop {
        let (_, stop) = self.run_chunked(None, insns, insns);
        if matches!(stop, Stop::Budget) {
            self.inject_irq(TIMER_IRQ);
        }
        stop
    }

    /// Run the registered handler of system interrupt `irq` as a call (`arm_steady.inject_irq`).
    pub fn inject_irq(&mut self, irq: u32) -> u32 {
        if self.cpu.irq_off {
            self.irq_while_masked += 1;
        }
        let handler = self.bus.read32(self.layout.irq_table + 4 * irq);
        if handler != 0 {
            self.call_preserving(handler, &[], 2_000_000);
        }
        handler
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> Machine {
        let mut m = Machine::new();
        // code: one ARM `bx lr` at the load base
        let code = 0xE12F_FF1Eu32.to_le_bytes();
        m.load_firmware(&code, &[1, 2, 3, 4]);
        m
    }

    #[test]
    fn load_firmware_maps_regions_and_builds_fake_comm_objects() {
        let mut m = tiny();
        assert_eq!(m.read32(LOAD_BASE), 0xE12F_FF1E);
        assert_eq!(m.read32(SRAM_BASE), 0x0403_0201);
        assert_eq!(m.read32(Layout::V166.port_object_ptr), FAKE_BASE);
        let obj2 = m.read32(Layout::V166.comm_table + 4 * 3);
        assert_eq!(obj2, FAKE_BASE + 0x1000 + 0x200);
        assert_eq!(m.read32(obj2 + 0x10), PORT1_BASE + 0x20);
        assert_eq!(m.read32(FAKE_BASE + 0x40 + 8), PORT2_BASE + 0x20);
        assert_eq!(m.bus.mem.read8(FAKE_BASE + 0x90 + 3), 0);
    }

    #[test]
    fn flow_control_is_stubbed_and_boot_stubs_install_the_guard() {
        let mut m = tiny();
        m.install_boot_stubs();
        assert!(m.null_guard);
        assert_eq!(m.hooks[&Layout::V166.flow_control], Hook::Return(1));
        assert_eq!(m.hooks[&Layout::V166.spi_transfer], Hook::SpiTransfer);
        assert_eq!(m.heap_top, HEAP_BASE);
    }

    #[test]
    fn run_chunked_zeroes_low_memory_between_chunks_and_stops_at_pc() {
        let mut m = tiny();
        m.guard_null_calls();
        // 0xC0000004: str to low memory then loop: mov r0,#0x55 ; mov r1,#0 ; strb r0,[r1] ; b .
        for (i, w) in [0xE3A0_0055u32, 0xE3A0_1000, 0xE5C1_0000, 0xEAFF_FFFE]
            .iter()
            .enumerate()
        {
            m.bus.mem.write32(0xC000_0004 + 4 * i as u32, *w);
        }
        m.cpu.r[15] = 0xC000_0004;
        let (done, stop) = m.run_chunked(None, 10, 5);
        assert_eq!((done, stop), (10, Stop::Budget));
        assert_eq!(m.bus.mem.read8(0), 0);
        let mut m2 = tiny();
        m2.guard_null_calls();
        for (i, w) in [0xE3A0_0055u32, 0xE3A0_1000, 0xE5C1_0000, 0xEAFF_FFFE]
            .iter()
            .enumerate()
        {
            m2.bus.mem.write32(0xC000_0004 + 4 * i as u32, *w);
        }
        m2.cpu.r[15] = 0xC000_0004;
        let (_, stop) = m2.run_chunked(Some(0xC000_0010), 1000, 100);
        assert_eq!(stop, Stop::StopPc);
        // the chunk that hit the stop pc is followed by the same zeroing as any other chunk
        assert_eq!(m2.bus.mem.read8(0), 0);
    }

    #[test]
    fn inject_irq_calls_the_registered_handler_and_restores_state() {
        let mut m = tiny();
        // handler at 0xC0000100: add r4,r4,#1 ; bx lr   (r4 is callee-saved here: prove restore)
        m.bus.mem.write32(0xC000_0100, 0xE284_4001);
        m.bus.mem.write32(0xC000_0104, 0xE12F_FF1E);
        m.write32(Layout::V166.irq_table + 4 * TIMER_IRQ, 0xC000_0100);
        m.cpu.r = [0; 16];
        m.cpu.r[4] = 10;
        m.cpu.r[13] = 0x7000_8000;
        m.cpu.r[15] = 0xC000_0000;
        assert_eq!(m.inject_irq(TIMER_IRQ), 0xC000_0100);
        assert_eq!(
            (m.reg(4), m.reg(13), m.reg(15)),
            (10, 0x7000_8000, 0xC000_0000)
        );
        assert_eq!(m.cpu.insns, 2);
    }

    #[test]
    fn load_firmware_picks_the_layout_of_the_image_version() {
        let mut m = Machine::new();
        m.load_firmware(&vec![0u8; Layout::V168.image_len], &[]);
        assert_eq!(m.layout.version, 168);
        assert_eq!(m.read32(Layout::V168.port_object_ptr), FAKE_BASE);
        assert_eq!(m.hooks[&Layout::V168.flow_control], Hook::Return(1));
        // an unknown image keeps the layout in force (v1.66 by default)
        let mut m = Machine::new();
        m.load_firmware(&[0u8; 16], &[]);
        assert_eq!(m.layout.version, 166);
    }
}
