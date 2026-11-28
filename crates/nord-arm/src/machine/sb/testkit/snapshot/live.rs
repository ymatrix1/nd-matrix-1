//! OWNER: W1-VGEN. Tests that [`snapshot`] reads every field it claims to compare from the live
//! [`Machine`]. The tests of `snapshot.rs` mutate a built `Snapshot`, which proves `diff` compares
//! the field but not that `snapshot` extracts it; these mutate the machine (and `snapshot_run`'s
//! stop reason), one field per case, and expect exactly that field named by the diff.
//!
//! A guard test parses the `Debug` output of [`Snapshot`] and [`CpuSnap`] for their field names, so
//! a field added without a case here fails, and so does a case that names no field.

use super::{snapshot, snapshot_run, CpuSnap};
use crate::cpu::{Bus, Fault};
use crate::machine::grouper::Message;
use crate::machine::sb::testkit::state::attach_boards;
use crate::machine::sysbus::{GPIO_BASE, PORT1_BASE, TICK_ADDRESS};
use crate::machine::{Machine, Stop};
use std::collections::BTreeSet;
use std::fmt::Debug;

/// Pages every case may touch, mapped in the base machine so that a store does not also map a
/// page (an auto-mapped page is itself part of the snapshot).
const PAGES: [u32; 5] = [
    0x5000,
    0x6000_0000,
    TICK_ADDRESS & !0xFFF,
    PORT1_BASE & !0xFFF,
    GPIO_BASE,
];
/// Where the poll-breaker case reads. ASSUMED: inside `DEVICE_WINDOWS` (`0x6000_0000..0x6A00_0000`,
/// `sysbus.rs`); checked by the case itself, which names `poll_sites` only if the breaker ran.
const POLLED: u32 = 0x6000_0010;

/// A change of the live machine.
type Poke = fn(&mut Machine);

/// The machine every case starts from: boards attached, the pages of [`PAGES`] mapped, and a
/// non-zero `bus.pc` so a device access is attributed to an instruction.
fn base() -> Machine {
    let mut m = Machine::new();
    attach_boards(&mut m.bus);
    for page in PAGES {
        m.bus.mem.map(page, 0x1000);
    }
    m.bus.pc = 0xC000_0000;
    m
}

/// The field names a diff lists: `r3: 0x0 != 0x9; auto_mapped[1]: ..` gives `r3`, `auto_mapped`.
fn named(diff: &str) -> BTreeSet<String> {
    diff.split("; ")
        .map(|e| e.split([':', '[']).next().unwrap_or("").to_string())
        .collect()
}

/// The field names of a struct, read from its pretty `Debug` output (the lines one level deep).
fn debug_fields<T: Debug>(v: &T) -> BTreeSet<String> {
    format!("{v:#?}")
        .lines()
        .filter_map(|l| l.strip_prefix("    "))
        .filter(|l| !l.starts_with([' ', '}', ']', ')']))
        .filter_map(|l| l.split_once(':').map(|(name, _)| name.to_string()))
        .collect()
}

/// Apply `poke` to a clone of `m` and return the field names the snapshot diff reports.
fn changed(m: &Machine, poke: Poke) -> BTreeSet<String> {
    let mut m2 = m.clone();
    poke(&mut m2);
    snapshot(m, None)
        .diff(&snapshot(&m2, None))
        .map_or_else(BTreeSet::new, |d| named(&d))
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

/// One field poked directly: the diff must name that field and nothing else.
const FIELDS: &[(&str, Poke)] = &[
    ("bus_pc", |m| m.bus.pc ^= 4),
    ("mem_hash", |m| m.bus.mem.write32(0x5000, 1)),
    ("device_reads", |m| m.bus.device_reads += 1),
    ("poll_hits", |m| m.bus.poll_hits += 1),
    ("messages", |m| {
        m.bus.messages.push(Message {
            path: 1,
            comm: 0,
            word1: 1,
            word2: 2,
            pc: 3,
        })
    }),
    ("auto_mapped", |m| m.bus.mem.auto_mapped.push(0x1000)),
    ("auto_mapped_count", |m| m.bus.mem.auto_mapped_count += 1),
    ("tick_t", |m| m.bus.tick_t += 1),
    ("tick_step", |m| m.bus.tick_step += 1),
    ("gpio_writes", |m| m.bus.gpio_writes += 1),
    ("dropped_messages", |m| m.bus.dropped_messages += 1),
    ("probing", |m| m.bus.probing = true),
    ("impure", |m| m.bus.impure = true),
    ("null_calls", |m| m.null_calls.push((1, 2))),
    ("null_call_count", |m| m.null_call_count += 1),
    ("wfi", |m| m.wfi += 1),
    ("irq_while_masked", |m| m.irq_while_masked += 1),
    ("heap_top", |m| m.heap_top += 16),
    ("panel", |m| m.bus.panel = None),
    ("i2c", |m| m.bus.i2c = None),
    ("uart2", |m| m.bus.uart2 = None),
    ("q", |m| m.cpu.q = true),
    ("thumb", |m| m.cpu.thumb = true),
    ("irq_off", |m| m.cpu.irq_off = !m.cpu.irq_off),
    ("fiq_off", |m| m.cpu.fiq_off = !m.cpu.fiq_off),
    ("mode", |m| m.cpu.mode = 0x10),
    ("mode_changes", |m| m.cpu.mode_changes += 1),
    ("spsr", |m| m.cpu.spsr ^= 1),
    ("cp15_ctrl", |m| m.cpu.cp15_ctrl ^= 1),
    ("insns", |m| m.cpu.insns += 1),
];

/// `nzcv` is one field of the snapshot but four flags of the CPU.
const FLAGS: [Poke; 4] = [
    |m| m.cpu.n = true,
    |m| m.cpu.z = true,
    |m| m.cpu.c = true,
    |m| m.cpu.v = true,
];

#[test]
fn every_field_poked_on_the_live_machine_is_read_by_the_snapshot_and_named_alone() {
    let m = base();
    for (field, poke) in FIELDS {
        assert_eq!(changed(&m, *poke), set(&[field]), "{field}");
    }
    for (i, poke) in FLAGS.iter().enumerate() {
        assert_eq!(changed(&m, *poke), set(&["nzcv"]), "flag {i}");
    }
    for i in 0..16 {
        let mut m2 = m.clone();
        m2.cpu.r[i] ^= 0x10;
        let d = snapshot(&m, None).diff(&snapshot(&m2, None)).unwrap();
        assert_eq!(named(&d), set(&[&format!("r{i}")]), "{d}");
    }
}

/// The same fields reached through the bus the way the guest reaches them: the side effects of
/// the device models are listed, so a snapshot that dropped one of them is caught.
#[test]
fn the_effects_of_real_bus_accesses_are_all_named() {
    let m = base();
    let cases: &[(&[&str], Poke)] = &[
        (&["messages", "mem_hash"], |m| {
            m.bus.write32(PORT1_BASE, 1);
            m.bus.write32(PORT1_BASE, 2);
        }),
        (&["grouper_pending", "mem_hash"], |m| {
            m.bus.write32(PORT1_BASE, 1)
        }),
        (&["tick_t", "mem_hash"], |m| {
            m.bus.read32(TICK_ADDRESS);
        }),
        (&["gpio_writes", "mem_hash"], |m| {
            m.bus.write32(GPIO_BASE + 0x18, 1)
        }),
        (&["auto_mapped", "auto_mapped_count"], |m| {
            m.bus.mem.read32(0x7777_0000);
        }),
        (
            &["device_reads", "poll_hits", "poll_sites", "mem_hash"],
            |m| {
                for _ in 0..41 {
                    m.bus.read32(POLLED);
                }
            },
        ),
    ];
    for (names, poke) in cases {
        assert_eq!(changed(&m, *poke), set(names), "{names:?}");
    }
}

#[test]
fn the_fault_and_the_stop_reason_of_a_run_snapshot_are_read_from_its_arguments() {
    let m = base();
    let plain = snapshot(&m, None);
    let with_fault = snapshot(&m, Some(Fault::Wfi));
    assert_eq!(named(&plain.diff(&with_fault).unwrap()), set(&["fault"]));
    let (a, b) = (
        snapshot_run(&m, &Stop::Budget),
        snapshot_run(&m, &Stop::Until),
    );
    assert_eq!(named(&a.diff(&b).unwrap()), set(&["stop"]));
    let f = snapshot_run(&m, &Stop::Fault(Fault::Wfi));
    assert_eq!(
        named(&a.diff(&f).unwrap()),
        set(&["fault", "stop"]),
        "a fault stop carries the fault into the cpu half"
    );
}

#[test]
fn a_field_of_the_snapshot_without_a_case_here_fails_the_guard() {
    let m = base();
    let mut covered: BTreeSet<String> = FIELDS.iter().map(|(f, _)| f.to_string()).collect();
    // the cases of the other tests: flags and registers (`nzcv`, `r`), the two run arguments, and
    // the fields only a device access reaches (`poll_sites`, `grouper_pending`)
    covered.extend(
        [
            "nzcv",
            "r",
            "fault",
            "stop",
            "poll_sites",
            "grouper_pending",
        ]
        .map(String::from),
    );
    let mut fields = debug_fields(&snapshot(&m, None));
    assert!(fields.remove("cpu"), "the cpu half is a field of its own");
    fields.extend(debug_fields(&CpuSnap::of(&m.cpu, None)));
    assert!(fields.len() > 30, "the parse found the fields: {fields:?}");
    assert_eq!(
        fields.difference(&covered).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "a field of the snapshot has no case in this file"
    );
    assert_eq!(
        covered.difference(&fields).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "a case names a field the snapshot does not have"
    );
}
