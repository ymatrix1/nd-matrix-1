//! OWNER: W1-VGEN. Full-state snapshots of a CPU and of a machine with a readable field diff:
//! the comparator of every differential test.
//!
//! Spec (design note 8, "observable state compared everywhere"): the comparison covers the
//! registers, the flag nibble, `q`, `thumb`, the interrupt masks, the mode and its change count,
//! `spsr`, `cp15_ctrl`, `insns` and the fault, and for a machine the bus pc, the memory hash, the
//! device counters, the messages (path, comm, words, pc), the auto-mapped pages, the tick clock,
//! the I2C, panel and UART2 models, the hook-visible counters (`heap_top`, null calls, `wfi`) and
//! the [`Stop`] reason. Each field has a test that mutates it and expects the diff to name it,
//! and `Snapshot::diff` destructures the struct without `..`, so a field added to [`Snapshot`]
//! and not compared is a compile error: the comparator cannot silently ignore one. `live.rs`
//! checks the other half: that [`snapshot`] reads each field from the live machine.
//!
//! Memory is compared by `Mem::state_hash` (zero and unmapped pages hash alike: mapping is not
//! state); [`mem_diff`] names the differing words when the hashes differ.

mod boards;
mod cpu;
#[cfg(test)]
mod live;

pub(crate) use boards::{I2cSnap, PanelSnap, UartSnap};
pub(crate) use cpu::CpuSnap;

use crate::cpu::Fault;
use crate::machine::grouper::Message;
use crate::machine::{Machine, Stop};
use crate::mem::Mem;
use std::fmt::Debug;

/// One line for a pair of sequences: `None` when equal, else the lengths or the first element
/// that differs (a message list can hold thousands of entries).
fn diff_seq<T: PartialEq + Debug>(name: &str, a: &[T], b: &[T]) -> Option<String> {
    if let Some(i) = (0..a.len().min(b.len())).find(|i| a[*i] != b[*i]) {
        return Some(format!("{name}[{i}]: {:x?} != {:x?}", a[i], b[i]));
    }
    (a.len() != b.len()).then(|| format!("{name}: {} entries != {} entries", a.len(), b.len()))
}

/// CPU plus the observable state of the system bus and of the machine around it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub cpu: CpuSnap,
    pub bus_pc: u32,
    pub mem_hash: u64,
    pub device_reads: u64,
    pub poll_hits: u64,
    pub poll_sites: usize,
    pub messages: Vec<Message>,
    pub auto_mapped: Vec<u32>,
    pub auto_mapped_count: u64,
    pub tick_t: u64,
    pub tick_step: u64,
    pub gpio_writes: u64,
    pub dropped_messages: u64,
    pub grouper_pending: usize,
    pub probing: bool,
    pub impure: bool,
    pub null_calls: Vec<(u32, u32)>,
    pub null_call_count: u64,
    pub wfi: u64,
    pub irq_while_masked: u64,
    pub heap_top: u32,
    pub panel: Option<PanelSnap>,
    pub i2c: Option<I2cSnap>,
    pub uart2: Option<UartSnap>,
    /// The reason the run stopped; `None` for a snapshot not taken at the end of a run
    /// ([`snapshot`]), `Some` for [`snapshot_run`].
    pub stop: Option<Stop>,
}

/// Snapshot of `m` (`fault` is the reason the run stopped, when it was one; `stop` is `None`).
pub(crate) fn snapshot(m: &Machine, fault: Option<Fault>) -> Snapshot {
    Snapshot {
        cpu: CpuSnap::of(&m.cpu, fault),
        bus_pc: m.bus.pc,
        mem_hash: m.bus.mem.state_hash(),
        device_reads: m.bus.device_reads,
        poll_hits: m.bus.poll_hits,
        poll_sites: m.bus.poll_sites(),
        messages: m.bus.messages.clone(),
        auto_mapped: m.bus.mem.auto_mapped.clone(),
        auto_mapped_count: m.bus.mem.auto_mapped_count,
        tick_t: m.bus.tick_t,
        tick_step: m.bus.tick_step,
        gpio_writes: m.bus.gpio_writes,
        dropped_messages: m.bus.dropped_messages,
        grouper_pending: m.bus.grouper.pending_words(),
        probing: m.bus.probing,
        impure: m.bus.impure,
        null_calls: m.null_calls.clone(),
        null_call_count: m.null_call_count,
        wfi: m.wfi,
        irq_while_masked: m.irq_while_masked,
        heap_top: m.heap_top,
        panel: m.bus.panel.as_ref().map(PanelSnap::of),
        i2c: m.bus.i2c.as_ref().map(I2cSnap::of),
        uart2: m.bus.uart2.as_ref().map(UartSnap::of),
        stop: None,
    }
}

/// [`snapshot`] at the end of a run: the fault is taken from `why` and `why` is compared too.
pub(crate) fn snapshot_run(m: &Machine, why: &Stop) -> Snapshot {
    let fault = if let Stop::Fault(f) = why {
        Some(*f)
    } else {
        None
    };
    Snapshot {
        stop: Some(*why),
        ..snapshot(m, fault)
    }
}

impl Snapshot {
    /// One entry per differing field joined by `; `; `None` when equal. Sequences name the first
    /// differing element, registers print in hex, the flags as `NZCV` letters.
    pub fn diff(&self, o: &Self) -> Option<String> {
        let mut out = Vec::new();
        macro_rules! compare {
            (scalars: $($f:ident),*; seqs: $($g:ident),*) => {
                // No `..`: a field missing from both lists is a compile error.
                let Snapshot { cpu: _, $($f: _,)* $($g: _,)* } = self;
                if let Some(d) = self.cpu.diff(&o.cpu) {
                    out.push(d);
                }
                $( if self.$f != o.$f {
                    out.push(format!("{}: {:x?} != {:x?}", stringify!($f), self.$f, o.$f));
                } )*
                $( if let Some(d) = diff_seq(stringify!($g), &self.$g, &o.$g) {
                    out.push(d);
                } )*
            };
        }
        compare!(
            scalars: bus_pc, mem_hash, device_reads, poll_hits, poll_sites, auto_mapped_count,
                tick_t, tick_step, gpio_writes, dropped_messages, grouper_pending, probing,
                impure, null_call_count, wfi, irq_while_masked, heap_top, panel, i2c, uart2, stop;
            seqs: messages, auto_mapped, null_calls
        );
        (!out.is_empty()).then(|| out.join("; "))
    }
}

/// Word-level differences of two memories as `(address, left, right)`, ascending, at most
/// `limit` of them. Unmapped and zero pages are equal, as in `Mem::state_hash`. Works on copies
/// (a read maps pages), so use it to explain a `mem_hash` mismatch, not in a hot loop.
pub(crate) fn mem_diff(a: &Mem, b: &Mem, limit: usize) -> Vec<(u32, u32, u32)> {
    let (mut a, mut b) = (a.clone(), b.clone());
    let mut pages = a.mapped_pages();
    pages.extend(b.mapped_pages());
    pages.sort_unstable();
    pages.dedup();
    let mut out = Vec::new();
    for base in pages {
        for w in 0..1024u32 {
            let at = base + 4 * w;
            let (x, y) = (a.read32(at), b.read32(at));
            if x != y {
                out.push((at, x, y));
                if out.len() == limit {
                    return out;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A change of one field, applied to a copy to prove the comparator sees it.
    type Mutation<T> = Box<dyn Fn(&mut T)>;

    #[test]
    fn the_machine_comparator_reports_every_bus_field_it_claims_to_compare() {
        let m = Machine::new();
        let base = snapshot(&m, None);
        assert_eq!(base.diff(&snapshot(&m.clone(), None)), None);
        let msg = || Message {
            path: 1,
            comm: 0,
            word1: 0,
            word2: 0,
            pc: 0,
        };
        let mutations: Vec<(&str, Mutation<Snapshot>)> = vec![
            ("bus_pc", Box::new(|s| s.bus_pc ^= 4)),
            ("mem_hash", Box::new(|s| s.mem_hash ^= 1)),
            ("device_reads", Box::new(|s| s.device_reads += 1)),
            ("poll_hits", Box::new(|s| s.poll_hits += 1)),
            ("poll_sites", Box::new(|s| s.poll_sites += 1)),
            ("messages", Box::new(move |s| s.messages.push(msg()))),
            ("auto_mapped", Box::new(|s| s.auto_mapped.push(0x1000))),
            ("auto_mapped_count", Box::new(|s| s.auto_mapped_count += 1)),
            ("tick_t", Box::new(|s| s.tick_t += 1)),
            ("tick_step", Box::new(|s| s.tick_step += 1)),
            ("gpio_writes", Box::new(|s| s.gpio_writes += 1)),
            ("dropped_messages", Box::new(|s| s.dropped_messages += 1)),
            ("grouper_pending", Box::new(|s| s.grouper_pending += 1)),
            ("probing", Box::new(|s| s.probing = !s.probing)),
            ("impure", Box::new(|s| s.impure = !s.impure)),
            ("null_calls", Box::new(|s| s.null_calls.push((1, 2)))),
            ("null_call_count", Box::new(|s| s.null_call_count += 1)),
            ("wfi", Box::new(|s| s.wfi += 1)),
            ("irq_while_masked", Box::new(|s| s.irq_while_masked += 1)),
            ("heap_top", Box::new(|s| s.heap_top += 16)),
            (
                "panel",
                Box::new(|s| {
                    s.panel = Some(PanelSnap {
                        strobe: 0,
                        held: vec![],
                        display_writes: 0,
                    })
                }),
            ),
            (
                "i2c",
                Box::new(|s| {
                    s.i2c = Some(I2cSnap {
                        log: vec![],
                        regs: [None; 4],
                        eeprom_hash: 0,
                    })
                }),
            ),
            (
                "uart2",
                Box::new(|s| {
                    s.uart2 = Some(UartSnap {
                        queue: vec![],
                        reads: 0,
                    })
                }),
            ),
            ("stop", Box::new(|s| s.stop = Some(Stop::Budget))),
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

    #[test]
    fn a_run_snapshot_carries_the_stop_reason_and_its_fault() {
        let m = Machine::new();
        let fault = Fault::Undefined { pc: 4, insn: 5 };
        let s = snapshot_run(&m, &Stop::Fault(fault));
        assert_eq!(
            (s.stop, s.cpu.fault),
            (Some(Stop::Fault(fault)), Some(fault))
        );
        let plain = snapshot_run(&m, &Stop::Budget);
        assert_eq!((plain.stop, plain.cpu.fault), (Some(Stop::Budget), None));
        let d = plain.diff(&snapshot(&m, None)).unwrap();
        assert!(d.contains("stop: Some(Budget) != None"), "{d}");
        let (a, b) = (
            snapshot_run(&m, &Stop::Until),
            snapshot_run(&m, &Stop::StopPc),
        );
        assert!(a.diff(&b).unwrap().contains("stop"));
    }

    #[test]
    fn sequences_name_the_first_differing_element_not_the_whole_list() {
        let m = Machine::new();
        let mut a = snapshot(&m, None);
        let mut b = a.clone();
        a.auto_mapped = (0..2000).map(|i| i << 12).collect();
        b.auto_mapped = a.auto_mapped.clone();
        b.auto_mapped[1500] ^= 0x1000;
        assert_eq!(a.diff(&b).unwrap(), "auto_mapped[1500]: 5dc000 != 5dd000");
        b.auto_mapped = a.auto_mapped[..1999].to_vec();
        assert_eq!(
            a.diff(&b).unwrap(),
            "auto_mapped: 2000 entries != 1999 entries"
        );
    }

    #[test]
    fn mem_diff_names_the_differing_words_and_ignores_mapping() {
        let (mut a, mut b) = (Mem::new(), Mem::new());
        a.write32(0x9000_0004, 7);
        b.write32(0x9000_0004, 7);
        b.map(0x5000_0000, 0x2000); // zero pages are not state
        assert_eq!(mem_diff(&a, &b, 8), vec![]);
        assert_eq!(a.state_hash(), b.state_hash());
        a.write32(0x9000_0008, 1);
        b.write32(0x9000_0FFC, 2);
        b.write32(0x9000_1000, 3);
        assert_eq!(
            mem_diff(&a, &b, 8),
            vec![
                (0x9000_0008, 1, 0),
                (0x9000_0FFC, 0, 2),
                (0x9000_1000, 0, 3)
            ]
        );
        assert_eq!(mem_diff(&a, &b, 2).len(), 2, "the limit is honoured");
        assert_ne!(a.state_hash(), b.state_hash());
    }
}
