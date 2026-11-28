//! OWNER: W1-VGEN (scaffold written by S0). Full-state snapshots of a CPU and of a machine with
//! a readable field diff: the comparator of every differential test.
//!
//! Spec (design note 8, WP W1-VGEN): the comparison covers the registers, the flag nibble, `q`,
//! `thumb`, the interrupt masks, the mode and its change count, `spsr`, `cp15_ctrl`, `insns`
//! and the fault, and for a machine the bus pc, the memory hash, the device counters, the
//! messages, the auto-mapped pages and the tick clock. Each field has a test that mutates it
//! and expects the diff to name it, so a comparator that silently ignores a field cannot pass.

use crate::cpu::{Cpu, Fault};
use crate::machine::grouper::Message;
use crate::machine::Machine;

/// The CPU state every tier must agree on, with a readable diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CpuSnap {
    pub r: [u32; 16],
    pub nzcv: u8,
    pub q: bool,
    pub thumb: bool,
    pub irq_off: bool,
    pub fiq_off: bool,
    pub mode: u32,
    pub mode_changes: u64,
    pub spsr: u32,
    pub cp15_ctrl: u32,
    pub insns: u64,
    pub fault: Option<Fault>,
}

impl CpuSnap {
    pub fn of(cpu: &Cpu, fault: Option<Fault>) -> Self {
        Self {
            r: cpu.r,
            nzcv: cpu.nzcv() as u8,
            q: cpu.q,
            thumb: cpu.thumb,
            irq_off: cpu.irq_off,
            fiq_off: cpu.fiq_off,
            mode: cpu.mode,
            mode_changes: cpu.mode_changes,
            spsr: cpu.spsr,
            cp15_ctrl: cpu.cp15_ctrl,
            insns: cpu.insns,
            fault,
        }
    }

    /// One line per differing field; `None` when equal.
    pub fn diff(&self, o: &Self) -> Option<String> {
        let mut out = Vec::new();
        for i in 0..16 {
            if self.r[i] != o.r[i] {
                out.push(format!("r{i}: {:#x} != {:#x}", self.r[i], o.r[i]));
            }
        }
        macro_rules! field {
            ($($f:ident),*) => {$(
                if self.$f != o.$f {
                    out.push(format!("{}: {:x?} != {:x?}", stringify!($f), self.$f, o.$f));
                }
            )*};
        }
        field!(
            nzcv,
            q,
            thumb,
            irq_off,
            fiq_off,
            mode,
            mode_changes,
            spsr,
            cp15_ctrl,
            insns,
            fault
        );
        (!out.is_empty()).then(|| out.join("; "))
    }
}

/// CPU plus the observable state of the system bus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub cpu: CpuSnap,
    pub bus_pc: u32,
    pub mem_hash: u64,
    pub device_reads: u64,
    pub poll_hits: u64,
    pub messages: Vec<Message>,
    pub auto_mapped: Vec<u32>,
    pub auto_mapped_count: u64,
    pub tick_t: u64,
    pub gpio_writes: u64,
    pub dropped_messages: u64,
    pub null_call_count: u64,
    pub wfi: u64,
}

/// Snapshot of `m` (`fault` is the reason the run stopped, when it was one).
pub(crate) fn snapshot(m: &Machine, fault: Option<Fault>) -> Snapshot {
    Snapshot {
        cpu: CpuSnap::of(&m.cpu, fault),
        bus_pc: m.bus.pc,
        mem_hash: m.bus.mem.state_hash(),
        device_reads: m.bus.device_reads,
        poll_hits: m.bus.poll_hits,
        messages: m.bus.messages.clone(),
        auto_mapped: m.bus.mem.auto_mapped.clone(),
        auto_mapped_count: m.bus.mem.auto_mapped_count,
        tick_t: m.bus.tick_t,
        gpio_writes: m.bus.gpio_writes,
        dropped_messages: m.bus.dropped_messages,
        null_call_count: m.null_call_count,
        wfi: m.wfi,
    }
}

impl Snapshot {
    /// One line per differing field; `None` when equal.
    pub fn diff(&self, o: &Self) -> Option<String> {
        let mut out = Vec::new();
        if let Some(d) = self.cpu.diff(&o.cpu) {
            out.push(d);
        }
        macro_rules! field {
            ($($f:ident),*) => {$(
                if self.$f != o.$f {
                    out.push(format!("{}: {:x?} != {:x?}", stringify!($f), self.$f, o.$f));
                }
            )*};
        }
        field!(
            bus_pc,
            mem_hash,
            device_reads,
            poll_hits,
            messages,
            auto_mapped,
            auto_mapped_count,
            tick_t,
            gpio_writes,
            dropped_messages,
            null_call_count,
            wfi
        );
        (!out.is_empty()).then(|| out.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A change of one field, applied to a copy to prove the comparator sees it.
    type Mutation<T> = Box<dyn Fn(&mut T)>;

    #[test]
    fn the_cpu_comparator_reports_every_field_it_claims_to_compare() {
        let base = CpuSnap::of(&Cpu::new(), None);
        assert_eq!(base.diff(&base.clone()), None);
        let mutations: Vec<(&str, Mutation<CpuSnap>)> = vec![
            ("r7", Box::new(|s| s.r[7] ^= 1)),
            ("r15", Box::new(|s| s.r[15] ^= 4)),
            ("nzcv", Box::new(|s| s.nzcv ^= 2)),
            ("q", Box::new(|s| s.q = !s.q)),
            ("thumb", Box::new(|s| s.thumb = !s.thumb)),
            ("irq_off", Box::new(|s| s.irq_off = !s.irq_off)),
            ("fiq_off", Box::new(|s| s.fiq_off = !s.fiq_off)),
            ("mode", Box::new(|s| s.mode ^= 1)),
            ("mode_changes", Box::new(|s| s.mode_changes += 1)),
            ("spsr", Box::new(|s| s.spsr ^= 1)),
            ("cp15_ctrl", Box::new(|s| s.cp15_ctrl ^= 1)),
            ("insns", Box::new(|s| s.insns += 1)),
            ("fault", Box::new(|s| s.fault = Some(Fault::Wfi))),
        ];
        for (name, mutate) in mutations {
            let mut m = base.clone();
            mutate(&mut m);
            let d = base
                .diff(&m)
                .unwrap_or_else(|| panic!("{name} not compared"));
            assert!(d.contains(name), "{name}: {d}");
        }
    }

    #[test]
    fn the_machine_comparator_reports_every_bus_field_it_claims_to_compare() {
        let m = Machine::new();
        let base = snapshot(&m, None);
        assert_eq!(base.diff(&snapshot(&m.clone(), None)), None);
        let mutations: Vec<(&str, Mutation<Snapshot>)> = vec![
            ("bus_pc", Box::new(|s| s.bus_pc ^= 4)),
            ("mem_hash", Box::new(|s| s.mem_hash ^= 1)),
            ("device_reads", Box::new(|s| s.device_reads += 1)),
            ("poll_hits", Box::new(|s| s.poll_hits += 1)),
            (
                "messages",
                Box::new(|s| {
                    s.messages.push(Message {
                        path: 1,
                        comm: 0,
                        word1: 0,
                        word2: 0,
                        pc: 0,
                    })
                }),
            ),
            ("auto_mapped", Box::new(|s| s.auto_mapped.push(0x1000))),
            ("auto_mapped_count", Box::new(|s| s.auto_mapped_count += 1)),
            ("tick_t", Box::new(|s| s.tick_t += 1)),
            ("gpio_writes", Box::new(|s| s.gpio_writes += 1)),
            ("dropped_messages", Box::new(|s| s.dropped_messages += 1)),
            ("null_call_count", Box::new(|s| s.null_call_count += 1)),
            ("wfi", Box::new(|s| s.wfi += 1)),
            ("insns", Box::new(|s| s.cpu.insns += 1)),
        ];
        for (name, mutate) in mutations {
            let mut s = base.clone();
            mutate(&mut s);
            let d = base
                .diff(&s)
                .unwrap_or_else(|| panic!("{name} not compared"));
            assert!(d.contains(name), "{name}: {d}");
        }
    }

    #[test]
    fn the_snapshot_of_a_machine_follows_its_memory_and_registers() {
        let mut m = Machine::new();
        let before = snapshot(&m, None);
        m.bus.mem.write32(0x5000, 1);
        m.cpu.r[3] = 9;
        let d = before.diff(&snapshot(&m, None)).unwrap();
        assert!(
            d.contains("r3: 0x0 != 0x9") && d.contains("mem_hash"),
            "{d}"
        );
    }
}
