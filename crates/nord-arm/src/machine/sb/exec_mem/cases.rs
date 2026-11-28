//! Hand-computed cases of the memory arms: each states the expected registers and the exact
//! access log (kind, address, value, pc; loads log value 0) from the ARM rules, not from the
//! interpreter, so a wrong rule cannot hide behind a wrong oracle. The interpreter comparison on
//! random words is `fuzz.rs`.

use super::diff::{enc_block, enc_half, enc_single, mode, uop_of, Mode, Off};
use super::*;
use crate::machine::sb::testkit::LogBus;

const PC: u32 = 0xC000_0100;

/// Run the arm for `word` with registers `init` and memory words `pokes`; every access of the
/// run must carry [`PC`].
fn run(word: u32, init: &[(usize, u32)], pokes: &[(u32, u32)]) -> ([u32; 16], LogBus) {
    let u = uop_of(word, PC).expect("a form the lowerers emit");
    run_uop(&u, init, pokes)
}

fn run_uop(u: &Uop, init: &[(usize, u32)], pokes: &[(u32, u32)]) -> ([u32; 16], LogBus) {
    let mut r = [0u32; 16];
    for &(i, v) in init {
        r[i] = v;
    }
    let mut bus = LogBus::new(0);
    for &(a, v) in pokes {
        bus.poke32(a, v);
    }
    exec_mem(&mut r, &mut bus, u);
    assert!(
        bus.log.iter().all(|e| e.3 == PC),
        "pc of every access: {:x?}",
        bus.log
    );
    (r, bus)
}

const LDR: u32 = 1;
const STR: u32 = 0;
const WORD: u32 = 0;
const BYTE: u32 = 1;
const PRE_UP: Mode = mode(1, 1, 0);

#[test]
fn a_word_load_rotates_an_unaligned_address() {
    let word = enc_single(LDR, WORD, PRE_UP, 1, 0, Off::Imm(0));
    for (base, loaded) in [
        (0x1000, 0x4433_2211),
        (0x1001, 0x1144_3322),
        (0x1002, 0x2211_4433),
        (0x1003, 0x3322_1144),
    ] {
        let (r, bus) = run(
            word,
            &[(0, 0xFFFF_FFFF), (1, base)],
            &[(0x1000, 0x4433_2211)],
        );
        assert_eq!(r[0], loaded, "base {base:#x}");
        assert_eq!(bus.log, [(2, 0x1000, 0, PC)]);
    }
}

#[test]
fn a_byte_load_zero_extends_and_reads_the_exact_address() {
    let (r, bus) = run(
        enc_single(LDR, BYTE, PRE_UP, 1, 0, Off::Imm(1)),
        &[(0, 0xFFFF_FFFF), (1, 0x1000)],
        &[(0x1000, 0x4433_2211)],
    );
    assert_eq!(r[0], 0x22);
    assert_eq!(bus.log, [(0, 0x1001, 0, PC)]);
}

#[test]
fn a_word_store_aligns_down_and_a_byte_store_writes_one_byte() {
    let init = [(0, 0xDEAD_BEEF), (1, 0x2001)];
    let (r, bus) = run(
        enc_single(STR, WORD, mode(1, 1, 1), 1, 0, Off::Imm(6)),
        &init,
        &[],
    );
    assert_eq!(
        (r[1], bus.log),
        (0x2007, vec![(5, 0x2004, 0xDEAD_BEEF, PC)])
    );
    let (r, bus) = run(
        enc_single(STR, BYTE, mode(1, 1, 0), 1, 0, Off::Imm(6)),
        &init,
        &[],
    );
    assert_eq!((r[1], bus.log), (0x2001, vec![(3, 0x2007, 0xEF, PC)]));
}

#[test]
fn pre_and_post_indexing_pick_the_address_and_write_back_the_moved_base() {
    let cases = [
        (mode(1, 1, 1), 8, 0x2008, 0x2008), // ldr r0,[r1,#8]!
        (mode(0, 1, 0), 8, 0x2000, 0x2008), // ldr r0,[r1],#8
        (mode(1, 0, 0), 8, 0x1FF8, 0x2000), // ldr r0,[r1,#-8]
        (mode(0, 0, 0), 8, 0x2000, 0x1FF8), // ldr r0,[r1],#-8
        (mode(0, 1, 1), 8, 0x2000, 0x2008), // ldrt r0,[r1],#8 (W with P = 0 still post-indexes)
    ];
    for (m, off, accessed, base_after) in cases {
        let (r, bus) = run(
            enc_single(LDR, WORD, m, 1, 0, Off::Imm(off)),
            &[(1, 0x2000)],
            &[],
        );
        assert_eq!(bus.log, [(2, accessed, 0, PC)], "{m:?}");
        assert_eq!(r[1], base_after, "{m:?}");
    }
}

#[test]
fn a_load_with_base_equal_to_dest_keeps_the_loaded_value_and_a_store_the_old_one() {
    for m in [mode(0, 1, 0), mode(1, 1, 1)] {
        let word = enc_single(LDR, WORD, m, 1, 1, Off::Imm(4));
        let at = if m.p == 1 { 0x3004 } else { 0x3000 };
        let (r, _) = run(word, &[(1, 0x3000)], &[(at, 0x1234_5678)]);
        assert_eq!(
            r[1], 0x1234_5678,
            "{m:?}: the loaded value wins over the write-back"
        );
    }
    let (r, bus) = run(
        enc_single(STR, WORD, mode(0, 1, 0), 1, 1, Off::Imm(8)),
        &[(1, 0x3000)],
        &[],
    );
    assert_eq!((r[1], bus.log), (0x3008, vec![(5, 0x3000, 0x3000, PC)]));
}

#[test]
fn register_offsets_shift_and_a_zero_lsr_or_asr_amount_means_32() {
    let ldr = |m: Mode, ty: u32, amount: u32| {
        enc_single(LDR, WORD, m, 1, 0, Off::Shifted { rm: 2, ty, amount })
    };
    let at = |word: u32, r2: u32| run(word, &[(1, 0x2000), (2, r2)], &[]).1.log[0].1;
    assert_eq!(at(ldr(PRE_UP, 0, 2), 3), 0x200C, "lsl #2");
    assert_eq!(
        at(ldr(PRE_UP, 0, 0), 0x10),
        0x2010,
        "lsl #0 is the plain register"
    );
    assert_eq!(
        at(ldr(PRE_UP, 1, 0), 0xFFFF_FFFF),
        0x2000,
        "lsr #0 is lsr #32: offset 0"
    );
    assert_eq!(
        at(ldr(PRE_UP, 2, 0), 0x8000_0000),
        0x1FFC,
        "asr #0 is asr #32: offset -1"
    );
    assert_eq!(at(ldr(PRE_UP, 2, 4), 0x8000_0000), 0xF800_2000, "asr #4");
    assert_eq!(at(ldr(PRE_UP, 3, 8), 0xFF), 0xFF00_2000, "ror #8");
    assert_eq!(at(ldr(mode(1, 0, 0), 0, 0), 0x10), 0x1FF0, "subtract");
    assert_eq!(
        at(ldr(mode(0, 1, 0), 0, 2), 3),
        0x2000,
        "post-indexed uses the base"
    );
}

#[test]
fn halfword_accesses_ignore_bit_0_and_signed_loads_extend() {
    let mem = [(0x2000, 0xFFFF_8081), (0x2004, 0x0000_007F)];
    let init = [(0, 0xCAFE_BABE), (1, 0x2000)];
    let ld =
        |l: u32, sh: u32, off: u32| run(enc_half(l, sh, PRE_UP, 1, 0, Off::Imm(off)), &init, &mem);
    let (r, bus) = ld(1, 1, 3);
    assert_eq!(
        (r[0], bus.log),
        (0xFFFF, vec![(1, 0x2002, 0, PC)]),
        "ldrh at 0x2003"
    );
    let (r, bus) = ld(1, 3, 0);
    assert_eq!(
        (r[0], bus.log),
        (0xFFFF_8081, vec![(1, 0x2000, 0, PC)]),
        "ldrsh"
    );
    let (r, bus) = ld(1, 2, 1);
    assert_eq!(
        (r[0], bus.log),
        (0xFFFF_FF80, vec![(0, 0x2001, 0, PC)]),
        "ldrsb of 0x80"
    );
    assert_eq!(ld(1, 2, 4).0[0], 0x7F, "ldrsb of 0x7F");
    assert_eq!(ld(1, 3, 4).0[0], 0x7F, "ldrsh of 0x007F");
    let word = enc_half(0, 1, mode(1, 1, 1), 1, 0, Off::Imm(1));
    let (r, bus) = run(word, &init, &mem);
    assert_eq!(
        (r[1], bus.log),
        (0x2001, vec![(4, 0x2000, 0xBABE, PC)]),
        "strh r0,[r1,#1]!"
    );
}

#[test]
fn a_register_offset_halfword_reads_rm() {
    let word = enc_half(1, 1, PRE_UP, 1, 0, Off::Reg(3));
    let (r, bus) = run(word, &[(1, 0x2000), (3, 0x10)], &[(0x2010, 0x0000_1234)]);
    assert_eq!((r[0], bus.log), (0x1234, vec![(1, 0x2010, 0, PC)]));
}

#[test]
fn ldrd_and_strd_move_a_pair_in_ascending_order_and_the_pair_beats_the_write_back() {
    let mem = [(0x2008, 0x1111_1111), (0x200C, 0x2222_2222)];
    let pair = vec![(2, 0x2008, 0, PC), (2, 0x200C, 0, PC)];
    let (r, bus) = run(
        enc_half(0, 2, PRE_UP, 0, 2, Off::Imm(8)),
        &[(0, 0x2000)],
        &mem,
    );
    assert_eq!((r[2], r[3], bus.log), (0x1111_1111, 0x2222_2222, pair));
    let wb = mode(1, 1, 1);
    let (r, _) = run(enc_half(0, 2, wb, 2, 2, Off::Imm(8)), &[(2, 0x2000)], &mem);
    assert_eq!(
        (r[2], r[3]),
        (0x1111_1111, 0x2222_2222),
        "rn == rd: the pair wins"
    );
    let (r, _) = run(enc_half(0, 2, wb, 3, 2, Off::Imm(8)), &[(3, 0x2000)], &mem);
    assert_eq!(
        (r[2], r[3]),
        (0x1111_1111, 0x2222_2222),
        "rn == rd + 1: the pair wins"
    );
    let (r, bus) = run(
        enc_half(0, 3, wb, 0, 2, Off::Imm(5)),
        &[(0, 0x2000), (2, 5), (3, 6)],
        &[],
    );
    assert_eq!(r[0], 0x2005);
    assert_eq!(
        bus.log,
        [(5, 0x2004, 5, PC), (5, 0x2008, 6, PC)],
        "strd, aligned down"
    );
}

#[test]
fn push_and_pop_use_the_full_descending_stack() {
    let list = 1 << 4 | 1 << 5 | 1 << 6 | 1 << 14;
    let init = [(13, 0x3000), (4, 0x44), (5, 0x55), (6, 0x66), (14, 0xEE)];
    let (r, bus) = run(enc_block(0, mode(1, 0, 1), 13, list), &init, &[]);
    assert_eq!(r[13], 0x2FF0);
    assert_eq!(
        bus.log,
        [
            (5, 0x2FF0, 0x44, PC),
            (5, 0x2FF4, 0x55, PC),
            (5, 0x2FF8, 0x66, PC),
            (5, 0x2FFC, 0xEE, PC)
        ]
    );
    let mem = [(0x3000, 0xA), (0x3004, 0xB), (0x3008, 0xC), (0x300C, 0xD)];
    let (r, bus) = run(enc_block(1, mode(0, 1, 1), 13, list), &[(13, 0x3000)], &mem);
    assert_eq!(
        (r[4], r[5], r[6], r[14], r[13]),
        (0xA, 0xB, 0xC, 0xD, 0x3010)
    );
    assert_eq!(bus.log.len(), 4);
}

#[test]
fn the_four_addressing_modes_start_at_the_right_word() {
    // (p, u) -> addresses of a two-register transfer at base 0x2000
    for ((p, u), want) in [
        ((0, 1), [0x2000, 0x2004]), // ia
        ((1, 1), [0x2004, 0x2008]), // ib
        ((0, 0), [0x1FFC, 0x2000]), // da
        ((1, 0), [0x1FF8, 0x1FFC]), // db
    ] {
        let (r, bus) = run(enc_block(1, mode(p, u, 1), 0, 0b110), &[(0, 0x2000)], &[]);
        let got: Vec<u32> = bus.log.iter().map(|e| e.1).collect();
        assert_eq!(got, want, "p={p} u={u}");
        assert_eq!(r[0], if u == 1 { 0x2008 } else { 0x1FF8 }, "p={p} u={u}");
    }
}

#[test]
fn stm_writes_back_after_its_first_store_and_ldm_skips_it_for_a_base_in_the_list() {
    let init = [(0, 0xA0), (1, 0x2000)];
    let (r, bus) = run(enc_block(0, mode(0, 1, 1), 1, 0b11), &init, &[]);
    assert_eq!(r[1], 0x2008);
    assert_eq!(
        bus.log,
        [(5, 0x2000, 0xA0, PC), (5, 0x2004, 0x2008, PC)],
        "base stored updated"
    );
    let (_, bus) = run(
        enc_block(0, mode(0, 1, 1), 0, 0b11),
        &[(0, 0x2000), (1, 7)],
        &[],
    );
    assert_eq!(
        bus.log[0],
        (5, 0x2000, 0x2000, PC),
        "the lowest register is the old base"
    );
    let (r, _) = run(
        enc_block(1, mode(0, 1, 1), 1, 0b11),
        &[(1, 0x2000)],
        &[(0x2004, 0x1234)],
    );
    assert_eq!(r[1], 0x1234, "no write-back over a loaded base");
    let (r, _) = run(enc_block(1, mode(0, 1, 0), 0, 0b110), &[(0, 0x2000)], &[]);
    assert_eq!(r[0], 0x2000, "no W bit, no write-back");
}

#[test]
fn a_block_transfer_aligns_each_word_address_and_an_empty_list_moves_nothing() {
    let (_, bus) = run(enc_block(1, mode(0, 1, 0), 0, 0b110), &[(0, 0x2002)], &[]);
    let got: Vec<u32> = bus.log.iter().map(|e| e.1).collect();
    assert_eq!(got, [0x2000, 0x2004]);
    for kind in [k::LDM, k::STM] {
        let mut u = Uop::new(kind, PC);
        (u.rn, u.rs) = (3, F_U | F_WB);
        let (r, bus) = run_uop(&u, &[(3, 0x2000)], &[]);
        assert_eq!((r[3], bus.log.len()), (0x2000, 0), "kind {kind}");
    }
}

#[test]
fn an_absolute_load_is_a_word_load_at_a_fixed_address() {
    let mut u = Uop::new(k::LDR_ABS, PC);
    (u.rd, u.imm) = (5, 0x4000);
    let (r, bus) = run_uop(&u, &[], &[(0x4000, 0x4433_2211)]);
    assert_eq!((r[5], bus.log), (0x4433_2211, vec![(2, 0x4000, 0, PC)]));
    u.imm = 0x4001;
    assert_eq!(run_uop(&u, &[], &[(0x4000, 0x4433_2211)]).0[5], 0x1144_3322);
}

#[test]
fn the_arm_finishes_a_store_that_makes_code_stale_and_leaves_the_exit_to_the_executor() {
    let word = enc_half(0, 3, mode(1, 1, 1), 0, 2, Off::Imm(0));
    let u = uop_of(word, PC).unwrap();
    let mut r = [0u32; 16];
    (r[0], r[2], r[3]) = (0x2000, 5, 6);
    let mut bus = LogBus::new(0);
    bus.stale_range = Some((0x2000, 0x2004));
    exec_mem(&mut r, &mut bus, &u);
    assert!(SbBus::stale(&bus), "the first word hit the range");
    assert_eq!(bus.log.len(), 2, "strd still stores its second word");
}
