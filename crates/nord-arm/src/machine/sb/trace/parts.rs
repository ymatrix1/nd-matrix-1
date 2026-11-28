//! OWNER: W1-FLOW. The small decisions of the trace builder, as free functions so each one can
//! be tested alone: may a trace read or follow an address (I9), what the `fold` callback of the
//! lowerers returns, and the early exit every store and `GENERIC` uop needs (I6).

use super::super::decode::Insn;
use super::super::lower_flow::mutant;
use super::super::uop::{k, Exit, ExitKind, Uop};
use crate::mem::{Mem, HOOK_R, HOOK_W, HOOK_X};

/// An aligned address on a mapped page without `HOOK_X`: code the builder may read (I9).
pub(super) fn fetchable(mem: &Mem, pc: u32) -> bool {
    pc & 3 == 0 && mem.is_mapped(pc) && mem.flags(pc) & HOOK_X == 0
}

/// May the trace continue at `pc`: fetchable, not yet in the trace, within the page cap.
pub(super) fn follow_ok(
    mem: &Mem,
    visited: &[u32],
    pages: &[u32],
    max_pages: usize,
    pc: u32,
) -> bool {
    fetchable(mem, pc)
        && !visited.contains(&pc)
        && (pages.contains(&(pc >> 12)) || pages.len() < max_pages)
}

/// The `fold` callback of the lowerers: the literal at `addr`, registered in the page list and
/// the word snapshot (each address once), or `None`: folding off, unaligned, unmapped, a page
/// with a device hook (`HOOK_R`, `HOOK_W`, `HOOK_X`), or a new page over the page cap. A refusal
/// reads and maps nothing (I9).
pub(super) fn fold_word(
    mem: &mut Mem,
    words: &mut Vec<(u32, u32)>,
    pages: &mut Vec<u32>,
    max_pages: usize,
    enabled: bool,
    addr: u32,
) -> Option<u32> {
    if !enabled || addr & 3 != 0 || !mem.is_mapped(addr) {
        return None;
    }
    if mem.flags(addr) & (HOOK_R | HOOK_W | HOOK_X) != 0 {
        return None;
    }
    if !pages.contains(&(addr >> 12)) {
        if pages.len() >= max_pages {
            return None;
        }
        pages.push(addr >> 12);
    }
    let word = mem.read32(addr);
    if !mutant(215) && !words.iter().any(|(a, _)| *a == addr) {
        words.push((addr, word));
    }
    Some(word)
}

/// Uops that need the early exit of I6: a store may make decoded code stale, and `GENERIC` runs
/// arbitrary non-enders (a store among them).
fn needs_early_exit(u: &Uop) -> bool {
    (k::is_store(u.kind) && !mutant(212)) || u.kind == k::GENERIC
}

/// Allocate the `Stale` exit of the instruction `insn` that lowered to `uops[from..]` (`done`
/// instructions retired before it) and name it in every uop that needs one. All uops of one
/// instruction share the exit, which retires the whole instruction and goes on at the next word.
pub(super) fn early_exit(
    uops: &mut [Uop],
    exits: &mut Vec<Exit>,
    from: usize,
    insn: &Insn,
    done: u16,
) {
    if !uops[from..].iter().any(needs_early_exit) {
        return;
    }
    let idx = exits.len() as u8;
    let next = if mutant(210) {
        insn.pc
    } else {
        insn.pc.wrapping_add(4)
    };
    exits.push(Exit::new(ExitKind::Stale, done + 1, insn.pc, next));
    for u in uops[from..].iter_mut().filter(|u| needs_early_exit(u)) {
        u.set_exit_idx(idx);
    }
}

#[cfg(test)]
pub(in crate::machine::sb) use tests::run_all;

#[cfg(test)]
mod tests {
    use super::super::testexec::suite;
    use super::*;
    use crate::machine::sb::decode::decode;

    fn mem_with(addr: u32, words: &[u32]) -> Mem {
        let mut mem = Mem::new();
        for (i, w) in words.iter().enumerate() {
            mem.write32(addr + 4 * i as u32, *w);
        }
        mem
    }

    suite! {
        #[test]
        fn only_aligned_mapped_unhooked_addresses_are_fetchable_and_nothing_gets_mapped() {
            let mut mem = mem_with(0xC000_0000, &[0xE1A0_0000]);
            mem.add_flag(0xC000_1000, 0xC000_1000, HOOK_X);
            mem.write32(0xC000_2000, 0);
            mem.add_flag(0xC000_2000, 0xC000_2000, HOOK_R | HOOK_W);
            let mapped = mem.auto_mapped_count;
            assert!(fetchable(&mem, 0xC000_0000));
            assert!(!fetchable(&mem, 0xC000_0002), "misaligned");
            assert!(!fetchable(&mem, 0xC000_5000), "unmapped");
            assert!(!fetchable(&mem, 0xC000_1000), "HOOK_X: a code hook runs there");
            assert!(
                fetchable(&mem, 0xC000_2000),
                "device hooks on reads and writes do not matter for a fetch"
            );
            assert_eq!(mem.auto_mapped_count, mapped);
        }

        #[test]
        fn a_trace_follows_a_new_pc_once_and_only_within_the_page_cap() {
            let mut mem = mem_with(0xC000_0000, &[0xE1A0_0000]);
            mem.write32(0xC000_1000, 0);
            let pc = 0xC000_0000;
            assert!(follow_ok(&mem, &[], &[0xC000_0000 >> 12], 1, pc), "same page");
            assert!(!follow_ok(&mem, &[pc], &[3], 4, pc), "already in the trace");
            assert!(!follow_ok(&mem, &[], &[0xC000_1000 >> 12], 1, pc), "page cap");
            assert!(follow_ok(&mem, &[], &[0xC000_1000 >> 12], 2, pc), "room for a page");
            assert!(!follow_ok(&mem, &[], &[3], 4, 0xC000_0001));
        }

        #[test]
        fn fold_returns_a_literal_once_registers_its_page_and_word_and_refuses_the_rest() {
            let mut mem = mem_with(0xC000_0100, &[0x1111_2222, 0x3333_4444]);
            mem.write32(0xC000_1000, 0x5555_6666);
            let (mut words, mut pages) = (vec![(0xC000_0000, 7)], vec![0xC000_0000 >> 12]);
            let mut fold = |a: u32, cap: usize, on: bool| {
                fold_word(&mut mem, &mut words, &mut pages, cap, on, a)
            };
            assert_eq!(fold(0xC000_0100, 1, true), Some(0x1111_2222));
            assert_eq!(fold(0xC000_0100, 1, true), Some(0x1111_2222), "the same literal twice");
            assert_eq!(fold(0xC000_0104, 1, true), Some(0x3333_4444));
            assert_eq!(fold(0xC000_1000, 1, true), None, "a second page over a cap of one");
            assert_eq!(fold(0xC000_0102, 1, true), None, "unaligned");
            assert_eq!(fold(0xC000_0100, 1, false), None, "folding off");
            assert_eq!(fold(0xC000_9000, 4, true), None, "unmapped, and not mapped by the call");
            assert_eq!(fold(0xC000_1000, 2, true), Some(0x5555_6666));
            assert_eq!(
                words,
                vec![
                    (0xC000_0000, 7),
                    (0xC000_0100, 0x1111_2222),
                    (0xC000_0104, 0x3333_4444),
                    (0xC000_1000, 0x5555_6666)
                ],
                "each address once, in the order read"
            );
            assert_eq!(pages, vec![0xC0000, 0xC0001]);
            assert!(!mem.is_mapped(0xC000_9000));
            mem.add_flag(0xC000_0200, 0xC000_0200, HOOK_R);
            mem.write32(0xC000_0200, 1);
            assert_eq!(fold_word(&mut mem, &mut words, &mut pages, 4, true, 0xC000_0200), None);
        }

        #[test]
        fn a_store_and_a_generic_get_one_shared_stale_exit_and_a_load_gets_none() {
            let insn = decode(0xC000_0010, 0xE580_1000); // str r1,[r0]
            let mut store = Uop::new(k::STR_IMM, insn.pc);
            let mut uops = vec![Uop::new(k::MOV_IMM, insn.pc - 4), store];
            let mut exits = vec![Exit::new(ExitKind::Direct, 1, 0, 0)];
            early_exit(&mut uops, &mut exits, 1, &insn, 5);
            assert_eq!(exits.len(), 2);
            assert_eq!(
                (exits[1].kind, exits[1].done, exits[1].last_pc, exits[1].target),
                (ExitKind::Stale, 6, 0xC000_0010, 0xC000_0014)
            );
            store.set_exit_idx(1);
            assert_eq!(uops[1], store);
            assert_eq!(uops[0].exit_idx(), 0, "an earlier instruction is not touched");
            // a second uop of the same instruction shares the exit; a load needs none
            let mut two = vec![Uop::generic(0xE1A0_0000, insn.pc, 0), Uop::new(k::STM, insn.pc)];
            let mut exits = Vec::new();
            early_exit(&mut two, &mut exits, 0, &insn, 0);
            assert_eq!((exits.len(), two[0].exit_idx(), two[1].exit_idx()), (1, 0, 0));
            let mut load = vec![Uop::new(k::LDR_IMM, insn.pc), Uop::new(k::ADD_IMM, insn.pc)];
            early_exit(&mut load, &mut exits, 0, &insn, 0);
            assert_eq!(exits.len(), 1, "loads and data processing never leave early");
        }

        #[test]
        fn the_stale_exit_of_a_conditional_or_wide_store_covers_the_whole_instruction() {
            let insn = decode(0xC000_0FFC, 0x1580_1000); // strne r1,[r0], last word of a page
            let mut uops = vec![Uop::new(k::PRED, insn.pc), Uop::new(k::STR_IMM, insn.pc)];
            let mut exits = Vec::new();
            early_exit(&mut uops, &mut exits, 0, &insn, 9);
            assert_eq!((exits[0].done, exits[0].target), (10, 0xC000_1000));
            assert_eq!(uops[1].exit_idx(), 0);
            assert_eq!(uops[0].rs, 0, "the PRED itself carries no exit");
        }
    }
}
