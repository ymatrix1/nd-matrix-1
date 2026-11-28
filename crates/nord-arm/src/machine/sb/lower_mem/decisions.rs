//! OWNER: W1-MEM (tests). Decision tests of `lower_mem`: which kind is chosen, how the fields are
//! packed, which forms stay `GENERIC`, how literals fold. 300 random words per class, assembled
//! from chosen fields by `words.rs` (so the expected uop is built from the choice, not decoded from
//! the word), plus explicit lists of the forms that must be rejected. Semantics are in
//! `semantics.rs`.

use super::words::{block, half, literal, rrx, single, wild, PC, RRX};
use super::{lower_mem, FLAG_P, FLAG_U};
use crate::machine::sb::decode::{decode, lower_or_generic, touches_r15, LowerCtx, Lowered};
use crate::machine::sb::testkit::Rng;
use crate::machine::sb::trace::BuildOpts;
use crate::machine::sb::uop::fused::{expand, fuse_pair};
use crate::machine::sb::uop::{k, Uop};

/// Words per class (WP W1-MEM).
const COUNT: u32 = 300;

/// `lower_mem` itself (no registry, no predicate wrap) on `word` at `pc`.
pub(super) fn lower_at(
    word: u32,
    pc: u32,
    opts: &BuildOpts,
    fold: &mut dyn FnMut(u32) -> Option<u32>,
) -> (Lowered, Vec<Uop>) {
    let mut out = Vec::new();
    let mut cx = LowerCtx {
        insn: decode(pc, word),
        out: &mut out,
        fold,
        opts,
    };
    (lower_mem(&mut cx), out)
}

fn lower(word: u32) -> (Lowered, Vec<Uop>) {
    lower_at(word, PC, &BuildOpts::default(), &mut |_| None)
}

fn assert_lowers_to(word: u32, want: Uop) {
    let (r, out) = lower(word);
    assert_eq!((r, out), (Lowered::Done, vec![want]), "{word:#010x}");
}

#[test]
fn word_and_byte_forms_pack_kind_flags_offset_and_raw_shift() {
    let mut rng = Rng::new(0x11);
    for reg in [false, true] {
        for (load, byte) in [(true, false), (true, true), (false, false), (false, true)] {
            let (mut flags, mut shifts) = ([false; 8], [false; 4]);
            for _ in 0..COUNT {
                let (word, want) = single(&mut rng, load, byte, reg);
                assert_lowers_to(word, want);
                flags[(want.rs & 7) as usize] = true;
                shifts[(want.sh >> 5) as usize] = true;
            }
            // P = 0 always writes back, so 6 of the 8 flag triples exist
            assert_eq!(
                flags.iter().filter(|f| **f).count(),
                6,
                "{load} {byte} {reg}"
            );
            let want_shifts = if reg {
                [true; 4]
            } else {
                [true, false, false, false]
            };
            assert_eq!(
                shifts, want_shifts,
                "shift types seen (an immediate offset has none)"
            );
        }
    }
}

#[test]
fn half_signed_and_double_forms_pack_kind_flags_and_offset() {
    let mut rng = Rng::new(0x22);
    for reg in [false, true] {
        for (load, sh) in [
            (true, 1),
            (true, 2),
            (true, 3),
            (false, 1),
            (false, 2),
            (false, 3),
        ] {
            let mut flags = [false; 16];
            for _ in 0..COUNT {
                let (word, want) = half(&mut rng, load, sh, reg);
                assert_lowers_to(word, want);
                flags[(want.rs & 15) as usize] = true;
            }
            assert_eq!(flags.iter().filter(|f| **f).count(), 6, "{load} {sh} {reg}");
        }
    }
}

#[test]
fn double_transfers_need_an_even_register_below_r14() {
    let mut rng = Rng::new(0x33);
    for reg in [false, true] {
        for sh in [2, 3] {
            for rd in 0..16u32 {
                let (mut word, want) = half(&mut rng, false, sh, reg);
                word = word & !(0xF << 12) | rd << 12;
                let (r, out) = lower(word);
                if rd % 2 == 0 && rd <= 12 {
                    let want = Uop {
                        rd: rd as u8,
                        ..want
                    };
                    assert_eq!((r, out), (Lowered::Done, vec![want]), "rd {rd}");
                } else {
                    assert_eq!((r, out), (Lowered::Generic, vec![]), "rd {rd} {word:#010x}");
                }
            }
        }
    }
}

#[test]
fn block_transfers_pack_the_list_flags_and_popcount() {
    let mut rng = Rng::new(0x44);
    for load in [false, true] {
        let mut flags = [false; 8];
        for _ in 0..COUNT {
            let (word, want) = block(&mut rng, load);
            assert_lowers_to(word, want);
            flags[(want.rs & 7) as usize] = true;
        }
        assert!(flags.iter().all(|f| *f), "all U, P, W combinations seen");
    }
}

#[test]
fn forms_the_interpreter_must_run_stay_generic_and_push_nothing() {
    for (word, why) in [
        (0xE581_F000, "str pc,[r1]"),
        (0xE5C1_F000, "strb pc,[r1]"),
        (0xE591_F000, "ldr pc,[r1] (a block ender)"),
        (0xE791_000F, "ldr r0,[r1,pc]"),
        (0xE781_000F, "str r0,[r1,pc]"),
        (0xE791_0312, "ldr r0,[r1,r2,lsl r3] (undefined media space)"),
        (0xE58F_0004, "str r0,[pc,#4]"),
        (0xE79F_0001, "ldr r0,[pc,r1]"),
        (0xE49F_0004, "ldr r0,[pc],#4 (post-index writes pc)"),
        (0xE5BF_0004, "ldr r0,[pc,#4]! (write-back writes pc)"),
        (0xE1C0_10D0, "ldrd r1,[r0] (odd register)"),
        (0xE1C0_10F0, "strd r1,[r0] (odd register)"),
        (0xE1C0_E0D0, "ldrd r14,[r0] (pairs with pc)"),
        (0xE1C0_E0F0, "strd r14,[r0] (pairs with pc)"),
        (0xE1C0_D0D0, "ldrd r13,[r0] (odd register)"),
        (0xE1CF_00D8, "ldrd r0,[pc,#8]"),
        (0xE1C1_F0B0, "strh pc,[r1]"),
        (0xE1D1_F0B0, "ldrh pc,[r1] (a block ender)"),
        (0xE191_00BF, "ldrh r0,[r1,pc]"),
        (0xE19F_00B1, "ldrh r0,[pc,r1]"),
        (0xE890_0000, "ldmia r0,{}"),
        (0xE880_0000, "stmia r0,{}"),
        (0xE8D0_0002, "ldmia r0,{r1}^ (user bank)"),
        (0xE8C0_0002, "stmia r0,{r1}^ (user bank)"),
        (0xE880_8002, "stmia r0,{r1,pc}"),
        (0xE890_8002, "ldmia r0,{r1,pc} (a block ender)"),
        (0xE89F_0002, "ldmia pc,{r1}"),
        (0xE8AF_0002, "stmia pc!,{r1}"),
        (0xE001_0092, "mul r1,r2,r0 (no kind id)"),
        (0xF57F_F05F, "the unconditional space"),
    ]
    .into_iter()
    // ldr/str/ldrb/strb with a `ROR #0` register offset, RRX: the carry flag is an input no
    // memory uop has (pre-indexed, with write-back, post-indexed)
    .chain(RRX)
    {
        assert_eq!(lower(word), (Lowered::Generic, vec![]), "{why}");
    }
}

#[test]
fn lsr_and_asr_by_zero_register_offsets_still_lower_raw_and_only_ror_by_zero_is_refused() {
    let uop = |sh: u8| {
        let mut u = Uop::new(k::LDR_REG, PC);
        (u.rd, u.rn, u.rm, u.rs, u.sh) = (0, 1, 2, FLAG_U | FLAG_P, sh);
        u
    };
    assert_lowers_to(0xE791_0002, uop(0x00)); // ldr r0,[r1,r2]  (lsl #0)
    assert_lowers_to(0xE791_0022, uop(0x20)); // ldr r0,[r1,r2,lsr #0]  (= lsr #32)
    assert_lowers_to(0xE791_0042, uop(0x40)); // ldr r0,[r1,r2,asr #0]  (= asr #32)
    assert_lowers_to(0xE791_00E2, uop(0x61)); // ldr r0,[r1,r2,ror #1]
    assert_lowers_to(0xE791_0FE2, uop(0x7F)); // ldr r0,[r1,r2,ror #31]
    assert_eq!(lower(0xE791_0062), (Lowered::Generic, vec![])); // ror #0 = rrx
}

#[test]
fn rrx_stays_generic_whatever_else_the_word_says() {
    let mut rng = Rng::new(0x45);
    for _ in 0..COUNT {
        let word = rrx(&mut rng);
        assert_eq!(lower(word), (Lowered::Generic, vec![]), "{word:#010x}");
    }
}

#[test]
fn lower_mem_itself_never_emits_a_uop_that_names_r15() {
    // 40 000 words with every register field free: the backstop of `lower_or_generic` is not needed.
    let (mut rng, mut done, mut generic) = (Rng::new(0x55), 0, 0);
    for _ in 0..40_000 {
        let word = wild(&mut rng);
        let (r, out) = lower(word);
        assert!(!out.iter().any(touches_r15), "{word:#010x}: {out:?}");
        assert_eq!(out.len(), (r == Lowered::Done) as usize, "{word:#010x}");
        assert!(out.iter().all(|u| u.n == 1 && u.cond == 0 && u.pc == PC));
        (done, generic) = (
            done + (r == Lowered::Done) as u32,
            generic + (r == Lowered::Generic) as u32,
        );
    }
    assert!(
        done > 5_000 && generic > 5_000,
        "{done} lowered, {generic} generic"
    );
}

#[test]
fn literals_call_fold_once_on_the_aligned_address_and_nothing_else_calls_it() {
    let (mut rng, mut seen) = (Rng::new(0x66), [false; 3]);
    let opts = BuildOpts {
        fold_literals: true,
        ..BuildOpts::default()
    };
    for _ in 0..COUNT {
        let pc = 0x4000 + 4 * rng.below(0x400);
        let (word, addr) = literal(&mut rng, pc);
        let mut calls = Vec::new();
        let (r, out) = lower_at(word, pc, &opts, &mut |a| {
            calls.push(a);
            Some(a.wrapping_mul(0x9E37_79B1))
        });
        assert_eq!((r, calls), (Lowered::Done, vec![addr & !3]), "{word:#010x}");
        let rd = (word >> 12 & 0xF) as u8;
        assert_eq!(
            (out.len(), out[0].kind, out[0].rd, out[0].n),
            (1, k::MOV_IMM, rd, 1)
        );
        assert_eq!(
            (out[0].rn, out[0].rm, out[0].rs, out[0].sh, out[0].pc),
            (0, 0, 0, 0, pc)
        );
        seen[(addr & 3).min(2) as usize] = true;
    }
    assert!(
        seen.iter().all(|s| *s),
        "aligned and unaligned literal addresses seen"
    );
    let mut none = |a: u32| -> Option<u32> { panic!("fold({a:#x}) for a non-literal") };
    for _ in 0..COUNT {
        let (a, b, c, sh) = (
            rng.chance(2),
            rng.chance(2),
            rng.chance(2),
            1 + rng.below(3),
        );
        let (w1, w2, w3) = (
            single(&mut rng, a, b, c).0,
            half(&mut rng, b, sh, c).0,
            block(&mut rng, a).0,
        );
        for w in [w1, w2, w3] {
            lower_at(w, PC, &opts, &mut none);
        }
        // a wild word may be a literal (rn = 15): fold is called exactly when a MOV_IMM results
        let mut calls = 0;
        let (_, out) = lower_at(wild(&mut rng), PC, &opts, &mut |a| {
            calls += 1;
            Some(a)
        });
        assert_eq!(calls, out.iter().filter(|u| u.kind == k::MOV_IMM).count());
    }
}

#[test]
fn an_unfolded_literal_is_ldr_abs_for_aligned_word_loads_and_generic_otherwise() {
    let on = BuildOpts {
        fold_literals: true,
        ..BuildOpts::default()
    };
    let off = BuildOpts {
        fold_literals: false,
        ..BuildOpts::default()
    };
    let abs = |addr: u32| {
        let mut u = Uop::new(k::LDR_ABS, PC);
        (u.rd, u.imm) = (3, addr);
        (Lowered::Done, vec![u])
    };
    let none = (Lowered::Generic, vec![]);
    let ldr = |off: u32| 0xE59F_3000 | off; // ldr r3,[pc,#off]
    for opts in [&on, &off] {
        let mut declines = |_: u32| None; // unmapped, hooked or over the page cap
        assert_eq!(lower_at(ldr(4), PC, opts, &mut declines), abs(PC + 12));
        assert_eq!(lower_at(0xE51F_3004, PC, opts, &mut declines), abs(PC + 4)); // [pc,#-4]
        assert_eq!(
            lower_at(ldr(5), PC, opts, &mut declines),
            none,
            "unaligned address"
        );
        assert_eq!(lower_at(0xE5DF_3004, PC, opts, &mut declines), none, "ldrb");
        assert_eq!(lower_at(0xE1DF_30B4, PC, opts, &mut declines), none, "ldrh");
    }
    let mut never = |a: u32| -> Option<u32> { panic!("fold({a:#x}) with folding switched off") };
    assert_eq!(lower_at(ldr(4), PC, &off, &mut never), abs(PC + 12));
    assert_eq!(lower_at(0xE5DF_3004, PC, &off, &mut never), none);
}

#[test]
fn a_conditional_load_is_wrapped_and_loads_off_one_base_fuse_into_a_pair() {
    let (mut out, mut fold) = (Vec::new(), |_: u32| None);
    let opts = BuildOpts::default();
    let mut cx = LowerCtx {
        insn: decode(PC, 0x1591_0004),
        out: &mut out,
        fold: &mut fold,
        opts: &opts,
    };
    assert_eq!(lower_or_generic(&mut cx), Lowered::Done); // ldrne r0,[r1,#4]
    assert_eq!(
        (out.len(), out[0].kind, out[0].cond, out[0].imm),
        (2, k::PRED, 1, 1)
    );
    assert_eq!((out[1].kind, out[1].cond, out[1].n), (k::LDR_IMM, 0, 1));
    // ldr r0,[r2,#8] ; ldr r1,[r2,#12] at consecutive words
    let first = lower_at(0xE592_0008, PC, &opts, &mut |_| None).1[0];
    let second = lower_at(0xE592_100C, PC + 4, &opts, &mut |_| None).1[0];
    let pair = fuse_pair(&first, &second).expect("pre-indexed loads from one base fuse");
    assert_eq!(
        (pair.kind, pair.rd, pair.rm, pair.rn),
        (k::LDR_PAIR, 0, 1, 2)
    );
    assert_eq!(expand(&pair), Some([first, second]));
    let wb = lower_at(0xE5B2_0008, PC, &opts, &mut |_| None).1[0]; // ldr r0,[r2,#8]!
    assert!(fuse_pair(&wb, &second).is_none(), "write-back is not fused");
}
