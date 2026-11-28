//! OWNER: W2-PEEP (tests). What the fusion pass does to the traces of small guest programs: which
//! pairs it makes, which it must not, and that the exit table and the counts stay as they were.

use super::harness::Passes;
use super::{run_counted, Fused};
use crate::machine::sb::testgen::asm::{self as a, AL};
use crate::machine::sb::testgen::CODE;
use crate::machine::sb::trace::build;
use crate::machine::sb::trace::testexec::suite;
use crate::machine::sb::uop::{k, Sb};
use crate::mem::Mem;

/// The trace at `CODE` of `words` followed by a spin, built with `passes`.
fn trace(words: &[u32], passes: Passes) -> Sb {
    let mut mem = Mem::new();
    let end = CODE + 4 * words.len() as u32;
    for (i, w) in words.iter().enumerate() {
        mem.write32(CODE + 4 * i as u32, *w);
    }
    mem.write32(end, a::branch(AL, false, end, end));
    build(&mut mem, CODE, &passes.opts()).expect("a trace")
}

fn kinds(sb: &Sb) -> Vec<u8> {
    sb.uops.iter().map(|u| u.kind).collect()
}

fn under(cond: u32, w: u32) -> u32 {
    w & 0x0FFF_FFFF | cond << 28
}

/// `bcc` at word `i` over the next `skip` words.
fn bcc(cond: u32, i: usize, skip: usize) -> u32 {
    a::branch(
        cond,
        false,
        CODE + 4 * i as u32,
        CODE + 4 * (i + 1 + skip) as u32,
    )
}

suite! {
    #[test]
    fn a_compare_and_the_branch_it_feeds_become_one_guard_with_the_exit_of_the_branch() {
        let cases = [
            ("cmp r1,#5", a::cmp_imm(1, 5), k::CMP_GUARD_I),
            ("cmp r1,r2", a::dp_shift_imm(10, true, 1, 0, 2, 0, 0), k::CMP_GUARD_R),
            ("tst r1,#0xF0", a::dp_imm(AL, 8, true, 1, 0, 0, 0xF0), k::TST_GUARD_I),
            ("tst r1,#0xF0 ror 8", a::dp_imm(AL, 8, true, 1, 0, 4, 0xF0), k::TST_GUARD_I),
        ];
        for (name, cmp, fused) in cases {
            let words = [cmp, bcc(1, 1, 2), a::mov_imm(3, 1), a::add_imm(false, 4, 4, 2), a::NOP];
            let (plain, sb) = (trace(&words, Passes::NONE), trace(&words, Passes::FUSE));
            assert_eq!(kinds(&sb)[0], fused, "{name}");
            assert_eq!(sb.uops.len(), plain.uops.len() - 1, "{name}: one uop fewer");
            let g = sb.uops.iter().find(|u| u.kind == fused).unwrap();
            let guard = plain.uops.iter().find(|u| u.kind == k::GUARD).unwrap();
            assert_eq!((g.cond, g.rs, g.n, g.pc), (guard.cond, guard.rs, 2, CODE), "{name}");
            assert_eq!(sb.exits[g.exit_idx()].done, 2, "{name}: the guard retires compare and branch");
            assert_eq!(sb.exits[g.exit_idx()].last_pc, CODE + 4, "{name}");
        }
    }

    #[test]
    fn compares_without_a_fused_form_or_without_an_adjacent_branch_stay_as_they_are() {
        // cmn, teq, a compare with a shifted operand, tst r,r (no TST_GUARD_R), and a compare with
        // an instruction between it and its branch
        let stay = [
            a::dp_shift_imm(11, true, 1, 0, 2, 0, 0),
            a::dp_shift_imm(9, true, 1, 0, 2, 0, 0),
            a::dp_shift_imm(10, true, 1, 0, 2, 0, 3),
            a::dp_shift_imm(8, true, 1, 0, 2, 0, 0),
        ];
        for cmp in stay {
            let words = [cmp, bcc(1, 1, 1), a::mov_imm(3, 1)];
            let sb = trace(&words, Passes::FUSE);
            let fused = sb.uops.iter().filter(|u| k::family(u.kind) == k::Family::Fused).count();
            assert_eq!(fused, 0, "compare {cmp:#010x}");
        }
        let words = [a::cmp_imm(1, 5), a::mov_imm(3, 1), bcc(1, 2, 1), a::mov_imm(4, 1)];
        let sb = trace(&words, Passes::FUSE);
        assert_eq!(kinds(&sb)[..3], [k::CMP_IMM, k::MOV_IMM, k::GUARD]);
    }

    #[test]
    fn adjacent_moves_of_small_constants_and_adjacent_loads_off_one_base_pair_up() {
        let words = [
            a::mov_imm(1, 7),
            a::mov_imm(2, 0xFF),
            a::ldr(3, 8, 4),
            a::ldr(4, 8, 8),
            a::NOP,
        ];
        let sb = trace(&words, Passes::FUSE);
        assert_eq!(kinds(&sb)[..2], [k::MOV_PAIR, k::LDR_PAIR]);
        let (pair, load) = (sb.uops[0], sb.uops[1]);
        assert_eq!((pair.rd, pair.rn, pair.imm, pair.n, pair.pc), (1, 2, 7 | 0xFF << 16, 2, CODE));
        assert_eq!((load.rd, load.rm, load.rn, load.imm, load.pc), (3, 4, 8, 4 | 8 << 16, CODE + 8));
        let plain = trace(&words, Passes::NONE);
        assert_eq!(sb.uops.len(), plain.uops.len() - 2);
        // three moves: the first two pair, the third stays alone
        let sb = trace(&[a::mov_imm(1, 1), a::mov_imm(2, 2), a::mov_imm(3, 3)], Passes::FUSE);
        assert_eq!(kinds(&sb)[..2], [k::MOV_PAIR, k::MOV_IMM]);
    }

    #[test]
    fn pairs_the_packing_cannot_carry_stay_unfused() {
        let wide = a::dp_imm(AL, 13, false, 0, 2, 4, 0xFF); // mov r2,#0xFF000000
        let sb = trace(&[a::mov_imm(1, 1), wide], Passes::FUSE);
        assert_eq!(kinds(&sb)[..2], [k::MOV_IMM, k::MOV_IMM]);
        let loads = [
            ("different bases", a::ldr(3, 8, 0), a::ldr(4, 9, 0)),
            ("write-back", a::ldst_imm(true, false, true, true, 8, 3, 4), a::ldr(4, 8, 8)),
            ("post-index", a::ldst_imm(true, false, false, false, 8, 3, 4), a::ldr(4, 8, 8)),
            ("a byte load", a::ldst_imm(true, true, true, false, 8, 3, 4), a::ldr(4, 8, 8)),
            ("a store", a::str(3, 8, 0), a::ldr(4, 8, 8)),
        ];
        for (name, first, second) in loads {
            let sb = trace(&[first, second, a::NOP], Passes::FUSE);
            assert!(sb.uops.iter().all(|u| u.kind != k::LDR_PAIR), "{name}");
        }
    }

    #[test]
    fn nothing_under_a_predicate_is_fused() {
        // the guard of a conditional compare, two moves where one is conditional, two conditional
        // moves, and loads where the second is conditional
        let words = [
            under(0, a::cmp_imm(1, 5)),
            bcc(1, 1, 2),
            under(0, a::mov_imm(2, 1)),
            a::mov_imm(3, 2),
            under(1, a::mov_imm(4, 1)),
            under(1, a::mov_imm(5, 2)),
            a::ldr(6, 8, 0),
            under(0, a::ldr(7, 8, 4)),
            a::NOP,
        ];
        let sb = trace(&words, Passes::FUSE);
        let fused: Vec<u8> = kinds(&sb).into_iter().filter(|kind| *kind >= k::CMP_GUARD_I).collect();
        assert!(fused.is_empty(), "{fused:?} in {:?}", kinds(&sb));
    }

    #[test]
    fn the_exit_table_the_counts_and_the_words_are_the_unfused_trace_s() {
        let words = [
            a::cmp_imm(1, 5),
            bcc(1, 1, 3),
            a::mov_imm(2, 1),
            a::mov_imm(3, 2),
            a::ldr(4, 8, 0),
            a::ldr(5, 8, 4),
            a::NOP,
        ];
        let (plain, sb) = (trace(&words, Passes::NONE), trace(&words, Passes::FUSE));
        assert_eq!((plain.n_guest, plain.entry, &plain.words), (sb.n_guest, sb.entry, &sb.words));
        assert_eq!(plain.exits.len(), sb.exits.len());
        for (x, y) in plain.exits.iter().zip(sb.exits.iter()) {
            assert_eq!((x.kind, x.done, x.last_pc, x.target), (y.kind, y.done, y.last_pc, y.target));
        }
        let covered: u32 = sb.uops.iter().map(|u| u32::from(u.n)).sum();
        assert_eq!(covered, plain.uops.iter().map(|u| u32::from(u.n)).sum::<u32>());
    }

    #[test]
    fn with_the_switch_off_nothing_is_fused() {
        let words = [a::cmp_imm(1, 5), bcc(1, 1, 1), a::mov_imm(2, 1), a::mov_imm(3, 2)];
        let sb = trace(&words, Passes::NONE);
        let mut uops = sb.uops.to_vec();
        let d = run_counted(&mut uops, &Passes::NONE.opts());
        assert_eq!((&uops[..], d), (&sb.uops[..], Fused::default()));
        assert!(run_counted(&mut uops, &Passes::FUSE.opts()).total() >= 2, "and with it on it fuses");
    }
}
