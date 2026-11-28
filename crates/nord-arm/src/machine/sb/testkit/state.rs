//! OWNER: W1-VGEN. Biased random CPU and machine states.
//!
//! Spec (design note 8, L0): "random state x random word per family (shift amounts 0, 1, 2, 30,
//! 31, 32, 33, 63, 64, 255, 256, every P/U/W combination, `rn == rd`, unaligned, page-end
//! addresses, all 16 NZCV)". [`random_cpu`] is the scaffold distribution (kept bit for bit: other
//! work packages' tests may depend on its stream); [`edge_cpu`] adds the shift amounts and the
//! page-end addresses; [`random_cpu_flags`] pins the flag nibble so a sweep can visit all 16.

use super::Rng;
use crate::cpu::Cpu;
use crate::machine::{Machine, SysBus};

/// Shift and rotate amounts that expose every boundary of the barrel shifter (design note 8, L0).
pub(crate) const SHIFT_AMOUNTS: [u32; 11] = [0, 1, 2, 30, 31, 32, 33, 63, 64, 255, 256];

/// Data page the generated programs use through `r8`.
pub(crate) const DATA_A: u32 = 0x9000_0000;
/// Second data page (`r9` of `testgen::machine_with`).
pub(crate) const DATA_B: u32 = 0x9000_1000;
/// Stack of the generated programs (`r13`).
pub(crate) const STACK: u32 = 0x9000_2000;

/// A CPU in ARM state at `pc` with biased registers, random flags and `q`, SVC mode.
pub(crate) fn random_cpu(rng: &mut Rng, pc: u32) -> Cpu {
    let mut cpu = Cpu::new();
    for r in cpu.r.iter_mut().take(15) {
        *r = rng.biased();
    }
    cpu.r[15] = pc;
    cpu.set_cpsr(rng.next() & 0xF800_0000 | 0x13);
    cpu.spsr = rng.next();
    cpu
}

/// [`random_cpu`] with the flag nibble `nzcv` (`n << 3 | z << 2 | c << 1 | v`) instead of random
/// flags, so a sweep over `0..16` visits every flag state.
pub(crate) fn random_cpu_flags(rng: &mut Rng, pc: u32, nzcv: u8) -> Cpu {
    let mut cpu = random_cpu(rng, pc);
    cpu.n = nzcv & 8 != 0;
    cpu.z = nzcv & 4 != 0;
    cpu.c = nzcv & 2 != 0;
    cpu.v = nzcv & 1 != 0;
    cpu
}

/// A CPU like [`random_cpu`] whose registers come from [`Rng::edge`]: the shift amounts of
/// [`SHIFT_AMOUNTS`], sign and carry boundaries, addresses at the end of a page and unaligned
/// ones, so register-specified shifts, address arithmetic and write-back rules see their edges.
pub(crate) fn edge_cpu(rng: &mut Rng, pc: u32) -> Cpu {
    let mut cpu = random_cpu(rng, pc);
    for r in cpu.r.iter_mut().take(15) {
        *r = rng.edge();
    }
    cpu
}

/// What [`random_machine`] sets up around the program.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MachineSpec {
    /// Address of the first program word and the initial `pc`.
    pub code_at: u32,
    /// Run ARM code from the predecode cache (tier 1 and above) or on the plain interpreter.
    pub predecode: bool,
    /// Fill the three data pages (`DATA_A`, `DATA_B`, `STACK`) with biased words, so loads read
    /// something other than zero.
    pub fill_data: bool,
    /// Attach the panel, I2C and UART2 board models to the bus.
    pub boards: bool,
}

impl Default for MachineSpec {
    fn default() -> Self {
        Self {
            code_at: 0xC000_0000,
            predecode: true,
            fill_data: true,
            boards: false,
        }
    }
}

/// A machine with `words` at `spec.code_at`, [`random_cpu`] registers and flags, `r8`, `r9` and
/// `r13` pointing at the data pages and the stack, `r10` at the code (self-modifying stores),
/// `lr` at the code and `pc` at its first word; optionally filled data pages and board models.
pub(crate) fn random_machine(rng: &mut Rng, words: &[u32], spec: &MachineSpec) -> Machine {
    let mut m = Machine::new();
    m.predecode = spec.predecode;
    m.cpu = random_cpu(rng, spec.code_at);
    m.bus.mem.write_bytes(
        spec.code_at,
        &words
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect::<Vec<u8>>(),
    );
    (m.cpu.r[8], m.cpu.r[9], m.cpu.r[10], m.cpu.r[13]) = (DATA_A, DATA_B, spec.code_at, STACK);
    m.cpu.r[14] = spec.code_at;
    if spec.fill_data {
        for page in [DATA_A, DATA_B, STACK] {
            for i in 0..1024 {
                m.bus.mem.write32(page + 4 * i, rng.biased());
            }
        }
    }
    if spec.boards {
        attach_boards(&mut m.bus);
    }
    m
}

/// Attach the three board models (front panel, I2C with its EEPROMs, UART2) to `bus`.
pub(crate) fn attach_boards(bus: &mut SysBus) {
    bus.panel = Some(crate::machine::panel::KeyMatrix::default());
    bus.i2c = Some(crate::machine::i2c::I2cBus::new());
    bus.uart2 = Some(crate::machine::uart::Uart2::default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::Stop;

    #[test]
    fn a_random_cpu_is_arm_state_at_pc_in_svc_mode() {
        let mut rng = Rng::new(1);
        for _ in 0..50 {
            let c = random_cpu(&mut rng, 0x1000);
            assert_eq!((c.r[15], c.thumb, c.mode), (0x1000, false, 0x13));
            assert_eq!(c.cpsr() & 0x07FF_FFFF, 0x13, "interrupts enabled, SVC");
        }
    }

    #[test]
    fn the_flag_sweep_visits_every_flag_state_and_leaves_the_rest_random() {
        let mut rng = Rng::new(2);
        for f in 0..16u8 {
            let c = random_cpu_flags(&mut rng, 0x2000, f);
            assert_eq!(c.nzcv() as u8, f);
            assert_eq!((c.r[15], c.mode), (0x2000, 0x13));
        }
        let a = random_cpu_flags(&mut Rng::new(5), 0, 0);
        let b = random_cpu_flags(&mut Rng::new(5), 0, 15);
        assert_eq!(a.r, b.r, "same stream, only the flags differ");
        assert_ne!(a.nzcv(), b.nzcv());
    }

    #[test]
    fn the_edge_cpu_reaches_every_shift_amount_and_page_end_in_its_registers() {
        let mut rng = Rng::new(3);
        let mut seen = std::collections::BTreeSet::new();
        let (mut page_end, mut unaligned) = (false, false);
        for _ in 0..400 {
            for v in edge_cpu(&mut rng, 0x1000).r.iter().take(15) {
                seen.insert(*v);
                page_end |= v & 0xFFF == 0xFFC;
                unaligned |= v & 3 == 3 && v & 0xFFF > 0xF00;
            }
        }
        for amount in SHIFT_AMOUNTS {
            assert!(seen.contains(&amount), "shift amount {amount} never drawn");
        }
        assert!(page_end && unaligned, "page ends and unaligned addresses");
    }

    #[test]
    fn a_random_machine_is_reproducible_and_has_the_documented_layout() {
        let words = [0xE280_0001u32, 0xEAFF_FFFE];
        let spec = MachineSpec::default();
        let (a, b) = (
            random_machine(&mut Rng::new(4), &words, &spec),
            random_machine(&mut Rng::new(4), &words, &spec),
        );
        assert_eq!(a.state_hash(), b.state_hash());
        assert_eq!(a.cpu.r, b.cpu.r);
        let mut a = a;
        assert_eq!(
            (
                a.cpu.r[15],
                a.cpu.r[8],
                a.cpu.r[9],
                a.cpu.r[10],
                a.cpu.r[13]
            ),
            (spec.code_at, DATA_A, DATA_B, spec.code_at, STACK)
        );
        assert_eq!(a.bus.mem.read32(spec.code_at + 4), 0xEAFF_FFFE);
        assert!(
            (0..64).any(|i| a.bus.mem.read32(DATA_A + 4 * i) != 0),
            "data pages are filled"
        );
        let plain = random_machine(
            &mut Rng::new(4),
            &words,
            &MachineSpec {
                fill_data: false,
                predecode: false,
                ..spec
            },
        );
        assert!(!plain.predecode && plain.bus.panel.is_none());
        assert_ne!(plain.state_hash(), b.state_hash());
        assert_eq!(
            (a.run(2, 0, None).1, a.cpu.r[15]),
            (Stop::Budget, spec.code_at + 4)
        );
    }

    #[test]
    fn the_boards_attach_on_request() {
        let m = random_machine(
            &mut Rng::new(1),
            &[0xE1A0_0000],
            &MachineSpec {
                boards: true,
                ..MachineSpec::default()
            },
        );
        assert!(m.bus.panel.is_some() && m.bus.i2c.is_some() && m.bus.uart2.is_some());
    }
}
