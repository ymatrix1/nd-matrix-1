//! ARMv5TE interpreter (ARM926EJ-S): ARM and Thumb state, CP15 stub, no exceptions.
//!
//! Interrupts are not modelled as exceptions: the harness runs the registered handler as a call
//! (`Machine::inject_irq`, as `tools/arm_steady.py::inject_irq` does). Processor modes are kept
//! in the CPSR but registers are not banked: `mode_changes` counts every `msr` that changes the
//! mode field so a run that depends on banking is visible.

pub mod alu;
mod arm;
mod arm_cached;
mod arm_dp;
mod arm_mem;
mod arm_mul;
pub mod bus;
pub mod predecode;
mod thumb;
mod thumb_mem;

pub use bus::Bus;

/// Why an instruction did not execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// Undefined or unsupported instruction at `pc` (the raw opcode, 16 bit values zero-extended).
    Undefined { pc: u32, insn: u32 },
    /// `swi` executed at `pc`.
    Swi { pc: u32, imm: u32 },
    /// Wait-for-interrupt (`mcr p15, 0, rd, c7, c0, 4`) executed; `pc` already points past it.
    Wfi,
}

/// CPU state.
#[derive(Clone)]
pub struct Cpu {
    /// `r0..r15`. While an instruction runs `r[15]` holds the operand value (`pc + 8` ARM,
    /// `pc + 4` Thumb); between instructions it is the address of the next instruction.
    pub r: [u32; 16],
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
    pub q: bool,
    pub thumb: bool,
    pub irq_off: bool,
    pub fiq_off: bool,
    pub mode: u32,
    pub spsr: u32,
    /// CP15 c1 control register (the only register firmware reads back).
    pub cp15_ctrl: u32,
    pub mode_changes: u64,
    pub insns: u64,
    pub(crate) jumped: bool,
    /// Fault of the last predecoded handler that returned `true` (see `arm_cached`).
    pub(crate) fault: Option<Fault>,
}

/// Supervisor mode, the reset mode.
pub const MODE_SVC: u32 = 0x13;
/// ASSUMED: ARM926EJ-S main ID register value `0x41069265` (r0p5, ARM926EJ-S TRM `DDI 0198E`);
/// the firmware's use of it is not checked against the real core revision.
pub const CP15_ID: u32 = 0x4106_9265;

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            r: [0; 16],
            n: false,
            z: false,
            c: false,
            v: false,
            q: false,
            thumb: false,
            irq_off: true,
            fiq_off: true,
            mode: MODE_SVC,
            spsr: 0,
            cp15_ctrl: 0,
            mode_changes: 0,
            insns: 0,
            jumped: false,
            fault: None,
        }
    }

    /// Flags as `n << 3 | z << 2 | c << 1 | v`, the index into [`predecode::COND_MASK`] bits.
    #[inline(always)]
    pub fn nzcv(&self) -> u32 {
        (self.n as u32) << 3 | (self.z as u32) << 2 | (self.c as u32) << 1 | self.v as u32
    }

    pub fn cpsr(&self) -> u32 {
        (self.n as u32) << 31
            | (self.z as u32) << 30
            | (self.c as u32) << 29
            | (self.v as u32) << 28
            | (self.q as u32) << 27
            | (self.irq_off as u32) << 7
            | (self.fiq_off as u32) << 6
            | (self.thumb as u32) << 5
            | self.mode
    }

    pub fn set_cpsr(&mut self, v: u32) {
        self.n = v >> 31 & 1 != 0;
        self.z = v >> 30 & 1 != 0;
        self.c = v >> 29 & 1 != 0;
        self.v = v >> 28 & 1 != 0;
        self.q = v >> 27 & 1 != 0;
        self.irq_off = v >> 7 & 1 != 0;
        self.fiq_off = v >> 6 & 1 != 0;
        self.thumb = v >> 5 & 1 != 0;
        self.mode = v & 0x1F;
    }

    /// Address of the next instruction (valid between instructions).
    pub fn pc(&self) -> u32 {
        self.r[15]
    }

    pub fn set_pc(&mut self, pc: u32) {
        self.r[15] = pc;
    }

    /// Branch: `interwork` selects Thumb state from bit 0 (BX, BLX, LDR/POP to pc).
    pub(crate) fn branch(&mut self, target: u32, interwork: bool) {
        if interwork {
            self.thumb = target & 1 != 0;
        }
        self.r[15] = if self.thumb { target & !1 } else { target & !3 };
        self.jumped = true;
    }

    pub(crate) fn set_nz(&mut self, v: u32) {
        self.n = v >> 31 != 0;
        self.z = v == 0;
    }

    /// Execute one instruction at `r[15]`.
    #[inline]
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> Result<(), Fault> {
        let pc = self.r[15];
        self.jumped = false;
        self.insns += 1;
        let res = if self.thumb {
            let insn = bus.fetch16(pc) as u32;
            self.r[15] = pc.wrapping_add(4);
            self.exec_thumb(bus, pc, insn)
        } else {
            let insn = bus.fetch32(pc);
            self.r[15] = pc.wrapping_add(8);
            self.exec_arm(bus, pc, insn)
        };
        if !self.jumped {
            self.r[15] = pc.wrapping_add(if self.thumb_after(pc) { 2 } else { 4 });
        }
        res
    }

    /// Instruction width after a non-branching instruction: unchanged state, so the width of the
    /// state the instruction started in. `msr`/`bx` always set `jumped`, so `self.thumb` is it.
    #[inline(always)]
    fn thumb_after(&self, _pc: u32) -> bool {
        self.thumb
    }
}

#[cfg(test)]
pub(crate) mod testkit {
    //! Tiny helpers for building synthetic ARM / Thumb programs in memory.
    use super::bus::FlatBus;
    use super::*;

    pub fn run_arm(code: &[u32], steps: usize) -> (Cpu, FlatBus) {
        let mut bus = FlatBus::default();
        for (i, w) in code.iter().enumerate() {
            bus.0.write32(0x1000 + 4 * i as u32, *w);
        }
        let mut cpu = Cpu::new();
        cpu.r[15] = 0x1000;
        for _ in 0..steps {
            cpu.step(&mut bus).unwrap();
        }
        (cpu, bus)
    }

    pub fn run_thumb(code: &[u16], steps: usize) -> (Cpu, FlatBus) {
        let mut bus = FlatBus::default();
        for (i, w) in code.iter().enumerate() {
            bus.0.write16(0x1000 + 2 * i as u32, *w);
        }
        let mut cpu = Cpu::new();
        cpu.thumb = true;
        cpu.r[15] = 0x1000;
        for _ in 0..steps {
            cpu.step(&mut bus).unwrap();
        }
        (cpu, bus)
    }

    #[test]
    fn cpsr_round_trip_and_reset_state() {
        let mut c = Cpu::new();
        assert_eq!(c.cpsr(), 0xC0 | MODE_SVC);
        c.set_cpsr(0xF800_0010);
        assert!(c.n && c.z && c.c && c.v && c.q && !c.thumb);
        assert_eq!(c.cpsr(), 0xF800_0010);
    }

    #[test]
    fn mov_add_sequence_runs() {
        // mov r0,#5 ; add r1,r0,#7 ; b . (infinite)
        let (c, _) = run_arm(&[0xE3A0_0005, 0xE280_1007], 2);
        assert_eq!((c.r[0], c.r[1], c.r[15]), (5, 12, 0x1008));
    }
}
