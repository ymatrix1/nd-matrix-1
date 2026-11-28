//! Handler-level differentials of the other move kinds (P8): immediate loads and stores,
//! `MODIFY` / `BITREV`, register moves and swaps and the dual data moves, plus the sweeps that
//! cross every kind with the uop flags (`UF_FIN` loop ends) and with wild index registers.
//!
//! OWNER: W2-MOVE. Same recipe as `tests_xfer`: lower with `MoveFamily`, run the table handlers,
//! compare with `Core::step` through `exact_diff`, and assert the sweep reached what it names.
use super::layout::{
    load_imm_kind, BITREV_IMM, CLASS_OTHER, CLASS_R, CLASS_S, DUAL, IMM_STORE, MODIFY, MODIFY_IMM,
    REG_MOVE, REG_SWAP,
};
use super::support::{loop_end_here, rand_insn, run};
use crate::core::uop::types::Exit;
use crate::core::uop::verify::gen::{census_encodings, edge_word, Rng};
use crate::core::Core;
use crate::visa::{bits, word48, Kind};
use std::collections::BTreeSet;

const LOADS: &[Kind] = &[Kind::T17a, Kind::T17b];
const STORES: &[Kind] = &[Kind::T16a, Kind::T16b];
const MODIFIES: &[Kind] = &[Kind::T7a, Kind::T7b, Kind::T19a];
const REGS: &[Kind] = &[Kind::T5a, Kind::T5b];
const DUALS: &[Kind] = &[Kind::T1a, Kind::T1b];
const ALL: &[Kind] = &[
    Kind::T1a,
    Kind::T1b,
    Kind::T3a,
    Kind::T3b,
    Kind::T3c,
    Kind::T4a,
    Kind::T4b,
    Kind::T5a,
    Kind::T5b,
    Kind::T7a,
    Kind::T7b,
    Kind::T14a,
    Kind::T15a,
    Kind::T15b,
    Kind::T16a,
    Kind::T16b,
    Kind::T17a,
    Kind::T17b,
    Kind::T19a,
];

/// What a sweep reached.
#[derive(Default)]
struct Sweep {
    /// Uop kinds that ran.
    kinds: BTreeSet<u16>,
    taken: u32,
    ok: u32,
    slow: u32,
    /// Cases in which `CB7S` or `CB15S` ended up set: a wrap was made, noted and latched.
    wrapped: u32,
}

/// `n` random instructions, the kinds of `kinds` in turn (so every kind gets its share), in
/// alternating scalar and SIMD mode, each compared with the interpreter. `fin` makes the
/// instruction the last of a loop body and its uop a `UF_FIN` one. `tweak` adjusts both cores
/// after the fill, knowing the parcels.
fn sweep(kinds: &[Kind], base: u64, n: u64, fin: bool, tweak: &dyn Fn(&mut Core, &[u16])) -> Sweep {
    let mut rng = Rng::new(base);
    let mut s = Sweep::default();
    for k in 0..n {
        let p = rand_insn(&mut rng, &[kinds[k as usize % kinds.len()]]);
        let (simd, seed) = (k % 2 == 1, base + k);
        let t = |c: &mut Core| {
            if fin {
                loop_end_here(c);
            }
            tweak(c, &p);
        };
        let r = run(&p, seed, simd, fin, &t);
        assert_eq!(r.diff, None, "{p:x?} simd {simd} seed {seed} fin {fin}");
        if !r.taken {
            continue;
        }
        s.taken += 1;
        s.kinds.extend(r.uops.iter().map(|u| u.h));
        match r.exit {
            Some(Exit::Ok) => s.ok += 1,
            Some(Exit::Slow) => s.slow += 1,
            _ => {}
        }
        s.wrapped += u32::from((r.got.sys.stky[0] >> 17) & 3 != 0);
    }
    s
}

fn nothing(_: &mut Core, _: &[u16]) {}

#[test]
fn immediate_loads_equal_the_interpreter_in_every_register_class() {
    let s = sweep(LOADS, 0x1000, 600, false, &nothing);
    let want: BTreeSet<u16> = [CLASS_R, CLASS_S, CLASS_OTHER]
        .into_iter()
        .map(|c| load_imm_kind(c).0)
        .collect();
    assert_eq!(s.kinds, want);
    assert!(
        s.taken > 200 && s.ok == s.taken,
        "taken {} ok {}",
        s.taken,
        s.ok
    );
}

#[test]
fn immediate_stores_write_both_elements_in_simd_mode() {
    let s = sweep(STORES, 0x2000, 600, false, &nothing);
    assert_eq!(s.kinds, BTreeSet::from([IMM_STORE.0]));
    assert!(
        s.taken > 500 && s.ok > s.taken / 2,
        "taken {} ok {}",
        s.taken,
        s.ok
    );
}

#[test]
fn modifies_and_bit_reversal_equal_the_interpreter_and_latch_their_wraps() {
    let s = sweep(MODIFIES, 0x3000, 1500, false, &nothing);
    assert_eq!(
        s.kinds,
        BTreeSet::from([MODIFY.0, MODIFY_IMM.0, BITREV_IMM.0])
    );
    assert!(
        s.taken > 800 && s.ok == s.taken,
        "taken {} ok {}",
        s.taken,
        s.ok
    );
    assert!(
        s.wrapped > 10,
        "{} cases wrapped the buffer of I7 or I15",
        s.wrapped
    );
}

#[test]
fn register_moves_and_swaps_equal_the_interpreter() {
    let s = sweep(REGS, 0x4000, 1200, false, &nothing);
    assert_eq!(s.kinds, BTreeSet::from([REG_MOVE.0, REG_SWAP.0]));
    assert!(
        s.taken > 400 && s.ok == s.taken,
        "taken {} ok {}",
        s.taken,
        s.ok
    );
}

/// `Rdst = Rsrc` (type 5b, 32 bits) between data registers: the float extension and the PEy twin
/// travel with the value (`reg_moves.rs`), and `fill_stale_ext` leaves extensions of an older value.
#[test]
fn a_move_between_float_registers_carries_the_extension_like_the_interpreter() {
    let mut moved = 0;
    for src in 0..16u16 {
        for dst in [0u16, 5, 15] {
            let p = [
                0x7000 | (src >> 2) << 6 | 0x3E | (src >> 1) & 1,
                (src & 1) << 15 | dst << 7 | 0x3F,
            ];
            for simd in [false, true] {
                let r = run(
                    &p,
                    u64::from(src) * 31 + u64::from(dst),
                    simd,
                    false,
                    &|_| {},
                );
                assert_eq!(r.diff, None, "R{dst} = R{src} simd {simd}");
                assert!(r.taken && r.uops[0].h == REG_MOVE.0);
                moved += u32::from(r.got.fx.lo[usize::from(dst)] != 0);
            }
        }
    }
    assert!(moved > 10, "{moved} moves left a nonzero extension byte");
}

/// A dual move whose two indices often point at the same word, so the order of the two accesses
/// (DM first, then PM) decides the result.
fn collide(c: &mut Core, p: &[u16]) {
    let w = word48(&[p[0], p[1], p.get(2).copied().unwrap_or(0)]);
    let (dmi, pmi) = (bits(w, 43, 41) as usize, 8 + bits(w, 32, 30) as usize);
    if (c.i[dmi] ^ c.i[pmi]) & 1 == 0 {
        c.i[pmi] = c.i[dmi];
    }
}

#[test]
fn dual_moves_keep_the_order_of_their_two_accesses() {
    let s = sweep(DUALS, 0x5000, 1500, false, &collide);
    assert_eq!(s.kinds, BTreeSet::from([DUAL.0]));
    assert!(
        s.taken > 900 && s.ok > s.taken / 3,
        "taken {} ok {}",
        s.taken,
        s.ok
    );
    assert!(s.wrapped > 5, "{} dual moves wrapped I7 or I15", s.wrapped);
}

#[test]
fn a_uop_that_finishes_its_step_runs_in_place_or_bails_for_every_kind() {
    let s = sweep(ALL, 0x6000, 2500, true, &nothing);
    assert!(s.taken > 800, "{} instructions taken", s.taken);
    assert!(s.ok > 300 && s.slow > 100, "ok {} slow {}", s.ok, s.slow);
    assert_eq!(
        s.kinds.len(),
        36,
        "every kind ran at a loop end: {:x?}",
        s.kinds
    );
    assert_eq!(
        s.ok + s.slow,
        s.taken,
        "a UF_FIN uop answers Ok or Slow, never Jump or Smc"
    );
}

/// Index registers of any value: `L` and `B` without relation to `I`, addresses all over the
/// space, and nothing may panic or differ.
fn wild(c: &mut Core, p: &[u16]) {
    let mut rng = Rng::new(u64::from(p[0]) ^ u64::from(p.len() as u32) << 20);
    for k in 0..16 {
        if rng.below(2) == 0 {
            c.i[k] = edge_word(&mut rng);
        }
        if rng.below(2) == 0 {
            c.m[k] = edge_word(&mut rng);
            c.l[k] = edge_word(&mut rng) & 0xFFFF;
        }
        if rng.below(2) == 0 {
            c.b[k] = edge_word(&mut rng);
        }
    }
    c.mode1 |= edge_word(&mut rng) & 0x1C0_0003;
}

#[test]
fn wild_index_registers_neither_panic_nor_differ() {
    let s = sweep(ALL, 0x7000, 2000, false, &wild);
    assert!(s.taken > 700 && s.ok > 200, "taken {} ok {}", s.taken, s.ok);
    assert_eq!(s.kinds.len(), 36, "every kind ran: {:x?}", s.kinds);
}

#[test]
fn the_encodings_of_the_firmware_census_equal_the_interpreter() {
    let corpus = census_encodings(|kind, _| ALL.contains(&kind));
    assert!(
        corpus.len() > 100,
        "{} census encodings of the move kinds",
        corpus.len()
    );
    let step = (corpus.len() / 600).max(1);
    let (mut taken, mut declined, mut kinds) = (0, 0, BTreeSet::new());
    for (k, p) in corpus.iter().enumerate().step_by(step) {
        for simd in [false, true] {
            let r = run(p, 0x8000 + k as u64, simd, false, &|_| {});
            assert_eq!(r.diff, None, "{p:x?} simd {simd}");
            if r.taken {
                taken += 1;
                kinds.insert(r.uops[0].h);
            } else {
                declined += 1;
            }
        }
    }
    assert!(
        taken > 100,
        "{taken} census encodings were lowered, {declined} stayed Interp"
    );
    assert!(
        kinds.len() >= 12,
        "{} kinds in the census: {kinds:x?}",
        kinds.len()
    );
}
