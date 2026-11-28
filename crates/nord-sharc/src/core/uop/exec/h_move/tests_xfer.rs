//! Handler-level differentials of the transfer kinds (P8): every `DM/PM` load and store form is
//! lowered by `MoveFamily`, run through the table handler and compared with `Core::step` on the
//! same state, in scalar and SIMD mode (`support::run`). Each test shows that it exercised what it
//! claims (kinds seen, exits taken, state really changed), so a test that stopped reaching its
//! target fails instead of passing empty (P10).
//!
//! OWNER: W2-MOVE.
use super::support::{loop_end_here, rand_insn, run, Run, DATA};
use crate::core::uop::exec::mem::LEN_SHIFT;
use crate::core::uop::run::RF_SMC;
use crate::core::uop::types::{Exit, UF_CB, UF_SIMD};
use crate::core::uop::verify::gen::{Rng, CODE_PC};
use crate::core::Core;
use crate::visa::Kind;
use std::collections::BTreeSet;

const TRANSFERS: &[Kind] = &[
    Kind::T3a,
    Kind::T3b,
    Kind::T3c,
    Kind::T4a,
    Kind::T4b,
    Kind::T14a,
    Kind::T15a,
    Kind::T15b,
];
const OK: u32 = Exit::Ok as u32;
const SMC: u32 = Exit::Smc as u32;
const JUMP: u32 = Exit::Jump as u32;
const SLOW: u32 = Exit::Slow as u32;
/// MODE1: `BDCST9` 22, `BDCST1` 23, `CBUFEN` 24.
const BDCST: u32 = 3 << 22;
pub(super) const CBUFEN: u32 = 1 << 24;

/// `T14a`: `ureg = DM(addr)` or `DM(addr) = ureg` (ureg in the low bits of the first parcel, the
/// store bit is 0x100, the address in the two other parcels).
fn t14a(addr: u32, ureg: u16, store: bool) -> [u16; 3] {
    [
        0x1000 | ureg | u16::from(store) << 8,
        (addr >> 16) as u16,
        addr as u16,
    ]
}

/// `R1 = DM(Ii, 2)` post-modify, type 4b (`6c7e 10bf` is I6): `ii` 0..8 plus the DAG2 bank.
pub(super) fn t4b_post(ii: u16, bank: u16) -> [u16; 2] {
    [
        (0x6c7e & !(7 << 9) & !(1 << 8)) | ii << 9 | bank << 8,
        0x10bf,
    ]
}

fn exit(r: &Run) -> Option<u32> {
    r.exit.map(|e| e as u32)
}

fn at_loop_end(c: &mut Core) {
    loop_end_here(c);
}

pub(super) fn mark_code(c: &mut Core) {
    c.uop_run.active = true;
    c.uop_run.code_pages.set_range(CODE_PC, CODE_PC + 0x100);
}

#[test]
fn every_transfer_kind_equals_the_interpreter_on_random_states() {
    let mut rng = Rng::new(0x5EED_0001);
    let mut kinds = BTreeSet::new();
    let (mut taken, mut ok) = (0, 0);
    let mut k = 0u64;
    while (k < 1000 || kinds.len() < 26) && k < 40_000 {
        let p = rand_insn(&mut rng, TRANSFERS);
        let simd = k % 2 == 1;
        let r = run(&p, 7000 + k, simd, false, &|_| {});
        assert_eq!(r.diff, None, "{p:x?} simd {simd} seed {}", 7000 + k);
        if r.taken {
            taken += 1;
            kinds.insert(r.uops[0].h);
            ok += u32::from(exit(&r) == Some(OK));
            assert_eq!(r.uops.len(), 1, "one instruction, one uop");
            assert_eq!(r.uops[0].a >> LEN_SHIFT, p.len() as u32);
        }
        k += 1;
    }
    assert_eq!(
        kinds.len(),
        26,
        "all 26 reachable (write, mode, class) kinds ran: {kinds:x?}"
    );
    assert!(
        taken > 300 && ok > taken / 2,
        "taken {taken}, plain exits {ok}"
    );
}

#[test]
fn a_wrap_of_i7_and_i15_is_latched_like_the_interpreter() {
    for (bank, sticky) in [(0u16, 17), (1, 18)] {
        let reg = 7 + usize::from(bank) * 8;
        for wraps in [true, false] {
            let tweak = move |c: &mut Core| {
                // L = 2 at B with the index on its second word: `I + 2` leaves the buffer
                let b = DATA + 0x80;
                (c.b[reg], c.l[reg], c.i[reg]) =
                    (b, if wraps { 2 } else { 8 }, b + u32::from(wraps));
                c.mode1 |= CBUFEN;
            };
            let r = run(&t4b_post(7, bank), 11, false, false, &tweak);
            assert_eq!(r.diff, None, "I{reg} wraps {wraps}");
            assert!(r.taken && r.uops[0].flags & UF_CB != 0, "UF_CB on I{reg}");
            assert_eq!(exit(&r), Some(OK));
            assert_eq!(
                (r.got.sys.stky[0] >> sticky) & 1 == 1,
                wraps,
                "STKY bit {sticky}"
            );
            assert_eq!(r.got.cb_wrap.get(), 0, "latched by the dispatch loop");
        }
    }
}

#[test]
fn a_load_through_i1_or_i9_is_broadcast_to_the_other_element() {
    // `R0 = DM(I1, M2)` (type 3c) and `R1 = DM(I9, 2)` (type 4b with the DAG2 bank bit)
    let t3c = [(0b1001u16 << 12) | (1 << 9) | (2 << 6) | (1 << 4)];
    let t4b = t4b_post(1, 1);
    let cases: [(&[u16], usize, usize); 2] = [(&t3c[..], 1, 0), (&t4b[..], 9, 1)];
    for (p, ii, reg) in cases {
        for bdcst in [0, BDCST] {
            let tweak = move |c: &mut Core| {
                c.i[ii] = DATA + 0x20;
                c.mode1 = (c.mode1 & !BDCST) | bdcst;
                c.s[reg] = 0xDEAD_0000 + reg as u32; // the twin the broadcast overwrites
            };
            let r = run(p, 21, false, false, &tweak);
            assert_eq!(r.diff, None, "I{ii} bdcst {bdcst:#x}");
            assert!(r.taken);
            assert_eq!(
                r.got.s[reg] == r.got.r[reg],
                bdcst != 0,
                "I{ii} broadcast {bdcst:#x}"
            );
        }
    }
}

#[test]
fn bit_reversed_output_applies_to_i0_and_i8_only() {
    let t3c_i0 = [(0b1001u16 << 12) | (1 << 4)]; // R0 = DM(I0, M0)
    let t4b_i8 = t4b_post(0, 1); // R1 = DM(I8, 2)
    let cases: [(&[u16], usize, u32); 2] = [(&t3c_i0[..], 0, 1 << 1), (&t4b_i8[..], 8, 1)];
    for (p, ii, br_bit) in cases {
        let reg = usize::from(ii == 8);
        for on in [false, true] {
            let tweak = move |c: &mut Core| {
                let target = DATA + 0x20;
                c.i[ii] = if on { target.reverse_bits() } else { target };
                (c.m[ii], c.l[ii]) = (1, 0);
                c.mode1 = (c.mode1 & !3) | if on { br_bit } else { 0 };
                c.dm_write(target, 0xFEED_0000 + ii as u32).unwrap();
            };
            let r = run(p, 33, false, false, &tweak);
            assert_eq!(r.diff, None, "I{ii} reversed {on}");
            assert!(r.taken && exit(&r) == Some(OK), "the target is plain SRAM");
            assert_eq!(r.got.r[reg], 0xFEED_0000 + ii as u32, "I{ii} reversed {on}");
        }
    }
}

#[test]
fn addresses_outside_the_sram_window_go_through_the_interpreters_memory_path() {
    let addrs = [
        0x3_E003u32, // IOP
        0x12_4000,   // short-word alias of SRAM
        0x10_0000,   // first word above the window
        0x8_FFFF,    // last word below it
        0x0802_0000, // SDRAM
        0x70_0000,   // device store
        0x3108_D004, // peripheral register
        0x1081,      // low peripheral
        0xFFFF_FFFF, // wraps at addr + 1 in SIMD
        0xF_FFFF,    // last SRAM word, addr + 1 leaves the window
    ];
    let models = |c: &mut Core| {
        c.enable_sc57x(0);
        c.enable_access_log();
    };
    let forms = [
        (0, false, false),
        (0, true, true),
        (0x53, false, true),
        (0x22, true, true),
        (0x22, false, true),
    ];
    let mut cases = 0;
    for &addr in &addrs {
        for (ureg, store, simd) in forms {
            let r = run(&t14a(addr, ureg, store), 43, simd, false, &models);
            let what = format!("{addr:#x} ureg {ureg:#x} store {store} simd {simd}");
            assert_eq!(r.diff, None, "{what}");
            assert!(r.taken && matches!(exit(&r), Some(OK | JUMP)), "{what}");
            cases += 1;
        }
    }
    assert_eq!(cases, 50);
}

#[test]
fn watched_and_read_watched_words_are_logged_in_program_order() {
    let watch = |c: &mut Core| {
        (c.watch, c.watch_len) = (DATA + 0x40, 4);
        (c.rwatch, c.rwatch_len, c.rwatch_on) = (DATA + 0x50, 4, true);
    };
    let mut logged = 0;
    for simd in [false, true] {
        // a store (pair) next to and into the watch window, a load next to and into the read watch
        for (addr, store) in [
            (DATA + 0x40, true),
            (DATA + 0x3F, true),
            (DATA + 0x43, true),
            (DATA + 0x50, false),
            (DATA + 0x4F, false),
            (DATA + 0x60, false),
        ] {
            let r = run(&t14a(addr, 0, store), 55, simd, false, &watch);
            assert_eq!(r.diff, None, "{addr:#x} store {store} simd {simd}");
            assert!(r.taken && exit(&r) == Some(OK));
            logged += r.got.watch_hits.len() + r.got.rwatch_hits.len();
        }
    }
    assert!(logged >= 8, "{logged} accesses were logged");
}

#[test]
fn a_store_into_translated_code_ends_the_uop_with_smc() {
    // the code is at word 0x9_0000: a store into its marked pages is `Smc`, one far away is not
    for (addr, smc) in [(0x9_0010u32, true), (0x9_0040, true), (DATA + 5, false)] {
        for simd in [false, true] {
            let r = run(&t14a(addr, 0, true), 66, simd, false, &mark_code);
            assert_eq!(r.diff, None, "{addr:#x} simd {simd}");
            assert_eq!(exit(&r), Some(if smc { SMC } else { OK }), "{addr:#x}");
            assert_eq!(r.got.uop_run.flags & RF_SMC != 0, smc);
        }
    }
}

#[test]
fn a_store_that_moves_the_deadline_ends_the_uop_with_a_jump_to_the_next_instruction() {
    // a store of 69 to the SEC register drains a queued interrupt: `irq.due` drops to 0
    let queue = |c: &mut Core| {
        c.enable_sc57x(1);
        c.sc57x().unwrap().raise_source(69);
        c.r[0] = 69;
    };
    for simd in [false, true] {
        let r = run(&t14a(0x3108_900C, 0, true), 77, simd, false, &queue);
        assert_eq!(r.diff, None, "simd {simd}");
        assert_eq!(exit(&r), Some(JUMP));
        assert_eq!(r.got.irq.due, 0, "the event is due now");
        assert_eq!(
            r.got.pc,
            CODE_PC + 3,
            "the pc after the 3-parcel instruction"
        );
    }
}

#[test]
fn a_uop_that_must_finish_its_step_bails_before_it_changes_anything() {
    // the instruction ends a loop body: `finish_step` runs after it. A plain SRAM access runs in
    // place; one the handler could not hand back by `Jump` or `Smc` is `Slow`
    let plain = run(&t14a(DATA + 9, 0, false), 88, false, true, &at_loop_end);
    assert_eq!(plain.diff, None);
    assert_eq!(exit(&plain), Some(OK));
    assert_eq!(
        plain.got.pc,
        CODE_PC + 0x40,
        "the loop went back to its start"
    );
    fn watched(c: &mut Core) {
        loop_end_here(c);
        (c.watch, c.watch_len) = (DATA + 0x10, 1);
    }
    fn marked(c: &mut Core) {
        loop_end_here(c);
        mark_code(c);
    }
    // (address, tweak, whether a load is slow too)
    type Tweak = fn(&mut Core);
    let cases: [(u32, Tweak, bool); 4] = [
        (0x3_E003, at_loop_end, true),  // IOP
        (0x12_4000, at_loop_end, true), // short-word alias
        (DATA + 0x10, watched, false),  // a watch only sees writes
        (0x9_0020, marked, false),      // a marked code page too
    ];
    let mut bails = 0;
    for (addr, tweak, load_slow) in cases {
        for (store, simd) in [(false, false), (true, false), (true, true), (false, true)] {
            let r = run(&t14a(addr, 0, store), 89, simd, true, &tweak);
            let what = format!("{addr:#x} store {store} simd {simd}");
            assert_eq!(r.diff, None, "{what}");
            let slow = store || load_slow;
            assert_eq!(exit(&r), Some(if slow { SLOW } else { OK }), "{what}");
            bails += usize::from(slow);
        }
    }
    assert_eq!(bails, 2 * 4 + 2 * 2, "every slow combination bailed");
    // SIMD: the first word is plain, the second (`addr + 1`) leaves the window
    let r = run(&t14a(0xF_FFFF, 0, true), 90, true, true, &at_loop_end);
    assert_eq!(r.diff, None);
    assert_eq!(exit(&r), Some(SLOW));
}

#[test]
fn a_slow_uop_does_not_note_a_wrap_it_did_not_make() {
    // I7 post-modify at the end of a circular buffer in the IOP, finishing its step: the preflight
    // bails before the update is computed, so `cb_wrap` is as the core had it (`run` compares the
    // bailing core with the untouched one) and the interpreter latches the wrap exactly once
    let tweak = |c: &mut Core| {
        loop_end_here(c);
        (c.b[7], c.l[7], c.i[7]) = (0x3_E000, 2, 0x3_E001);
        c.mode1 |= CBUFEN;
    };
    let r = run(&t4b_post(7, 0), 99, false, true, &tweak);
    assert_eq!(r.diff, None);
    assert_eq!(exit(&r), Some(SLOW));
    assert_eq!(
        (r.got.sys.stky[0] >> 17) & 1,
        1,
        "the interpreter latched the wrap"
    );
}

#[test]
fn the_simd_flag_and_the_length_come_from_the_lowering_not_from_the_state() {
    let r = run(&t14a(DATA, 0, false), 5, true, false, &|_| {});
    assert!(r.taken && r.uops[0].flags & UF_SIMD != 0);
    assert_eq!(r.uops[0].a >> LEN_SHIFT, 3);
    // the same instruction in a core without PEy runs without the twin
    let r = run(&t14a(DATA, 0, false), 5, false, false, &|_| {});
    assert!(r.uops[0].flags & UF_SIMD == 0);
}
