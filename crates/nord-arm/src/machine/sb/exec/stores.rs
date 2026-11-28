//! OWNER: W2-XCORE (tests). Invariant I6 for every store kind: a store that leaves
//! `Mem::stale_code` non-empty ends the trace before the next uop, whatever the store.
//!
//! `exits.rs` pins the rule for `STR_IMM` and `STM`; replacing the executor's test
//! `k::is_store(u.kind) && bus.stale()` by an explicit check of those two kinds left the whole
//! crate green. Here every store form the lowerers emit (word, byte, halfword and doubleword,
//! immediate and register offset, pre and post indexed, with and without write-back, `STM` in all
//! four addressing modes) is built by `trace::build` from its instruction word, run against a
//! page that holds decoded code, and must leave through its `Stale` exit before the `mov r0,#99`
//! that follows it; the same store to a data page must not. `kills.rs` runs the suite under
//! mutant 458 (only `STR_IMM` and `STM` end the trace).

use super::*;
use crate::machine::sb::testgen::asm::{self, AL};
use crate::machine::sb::trace::testexec::suite;
use crate::machine::sb::trace::{build, BuildOpts};
use crate::machine::SysBus;
use std::collections::BTreeSet;

const PC: u32 = 0xC000_0000;
const DATA: u32 = 0x9000_0000;

/// `(word, the uop kind it must lower to)`.
fn stores() -> Vec<(u32, u8)> {
    let block = |p, u, w| asm::block(AL, false, p, u, w, 2, 0x30); // stm{ia,ib,da,db}{!} r2, {r4,r5}
    vec![
        (asm::str(1, 2, 0), k::STR_IMM),
        (asm::str(1, 2, 4), k::STR_IMM),
        (asm::ldst_imm(false, false, true, true, 2, 1, 4), k::STR_IMM), // str r1,[r2,#4]!
        (
            asm::ldst_imm(false, false, false, false, 2, 1, 4),
            k::STR_IMM,
        ), // str r1,[r2],#4
        (
            asm::ldst_imm(false, true, true, false, 2, 1, 0),
            k::STRB_IMM,
        ),
        (
            asm::ldst_imm(false, true, false, false, 2, 1, 1),
            k::STRB_IMM,
        ),
        (0xE782_1003, k::STR_REG),  // str r1,[r2,r3]
        (0xE702_1003, k::STR_REG),  // str r1,[r2,-r3]
        (0xE7A2_1003, k::STR_REG),  // str r1,[r2,r3]!
        (0xE782_1103, k::STR_REG),  // str r1,[r2,r3,lsl #2]
        (0xE7C2_1003, k::STRB_REG), // strb r1,[r2,r3]
        (0xE6C2_1003, k::STRB_REG), // strb r1,[r2],r3
        (0xE1C2_10B0, k::STRH),     // strh r1,[r2]
        (0xE1C2_10B4, k::STRH),     // strh r1,[r2,#4]
        (0xE182_10B3, k::STRH),     // strh r1,[r2,r3]
        (0xE0C2_10B2, k::STRH),     // strh r1,[r2],#2
        (0xE1C2_40F0, k::STRD),     // strd r4,[r2]
        (0xE182_40F3, k::STRD),     // strd r4,[r2,r3]
        (block(false, true, false), k::STM),
        (block(true, true, false), k::STM),
        (block(false, false, false), k::STM),
        (block(true, false, false), k::STM),
        (block(false, true, true), k::STM),
        (block(true, false, true), k::STM),
    ]
}

/// The trace of `word ; mov r0,#99 ; bx lr`, run with the store aimed at `target`, on a bus whose
/// code page is marked. `(exit kind taken, r0, stale pages queued)`.
fn run_store(word: u32, want: u8, target: u32) -> (ExitKind, u32, usize) {
    let mut bus = SysBus::new();
    for (i, w) in [word, asm::mov_imm(0, 99), asm::bx(AL, 14)]
        .iter()
        .enumerate()
    {
        bus.mem.write32(PC + 4 * i as u32, *w);
    }
    let sb = build(&mut bus.mem, PC, &BuildOpts::default()).expect("a trace");
    assert!(
        sb.uops.iter().any(|u| u.kind == want),
        "{word:#010x} lowers to a uop of kind {want}: {:?}",
        sb.uops.iter().map(|u| u.kind).collect::<Vec<_>>()
    );
    for page in &sb.pages[..sb.n_pages as usize] {
        bus.mem.mark_code(page << 12);
    }
    bus.mem.write32(DATA, 0);
    let mut cpu = Cpu::new();
    cpu.r[1] = 0xDEAD_BEEF;
    cpu.r[2] = target;
    cpu.r[3] = 8;
    (cpu.r[4], cpu.r[5]) = (0x1111_1111, 0x2222_2222);
    cpu.r[14] = DATA;
    let mut left = 10;
    let out = run_sb(&mut cpu, &mut bus, &sb, &mut left, false);
    (
        sb.exits[usize::from(out.exit)].kind,
        cpu.r[0],
        bus.mem.stale_code.len(),
    )
}

suite! {
    #[test]
    fn every_store_form_into_decoded_code_leaves_through_the_stale_exit() {
        let mut kinds = BTreeSet::new();
        for (word, want) in stores() {
            // the store lands in the page of the code (offset 0x100: no overlap with the words)
            let (kind, r0, stale) = run_store(word, want, PC + 0x100);
            assert_eq!(
                (kind, r0, stale),
                (ExitKind::Stale, 0, 1),
                "{word:#010x}: the trace ends before the next uop"
            );
            kinds.insert(want);
        }
        let every: BTreeSet<u8> = (0..=255u8).filter(|id| k::is_store(*id)).collect();
        assert_eq!(kinds, every, "every store kind is covered");
    }

    #[test]
    fn the_same_stores_into_a_data_page_run_the_whole_trace() {
        for (word, want) in stores() {
            let (kind, r0, stale) = run_store(word, want, DATA + 0x100);
            assert_eq!(
                (kind, r0, stale),
                (ExitKind::Indirect, 99, 0),
                "{word:#010x}: no stale code, no early exit"
            );
        }
    }
}
