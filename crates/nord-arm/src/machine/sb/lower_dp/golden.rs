//! Golden decisions of `lower_dp`: hand-assembled words and the uop each must become. The words
//! are the ones of the design note's table (3.1) and the rejected forms of its `r15` rule.
//! `cfg(test)` only.

use super::support::{done, generic, lower, uop, PC0};
use crate::machine::sb::uop::k;

#[test]
fn immediates_pick_the_specialised_id_and_keep_the_fields_the_fuser_needs() {
    assert_eq!(done(0xE3A0_3040), uop(k::MOV_IMM, (3, 0, 0, 0, 0, 0x40)));
    // rot != 0: the rotated value, and a specialised non-S uop keeps sh = 0 either way
    assert_eq!(
        done(0xE3A0_34FF),
        uop(k::MOV_IMM, (3, 0, 0, 0, 0, 0xFF00_0000))
    );
    assert_eq!(done(0xE3E0_1000), uop(k::MVN_IMM, (1, 0, 0, 0, 0, 0)));
    assert_eq!(done(0xE281_0004), uop(k::ADD_IMM, (0, 1, 0, 0, 0, 4)));
    assert_eq!(done(0xE241_0004), uop(k::SUB_IMM, (0, 1, 0, 0, 0, 4)));
    assert_eq!(done(0xE382_2080), uop(k::ORR_IMM, (2, 2, 0, 0, 0, 0x80)));
    // cmp: no destination, no shifter field (CMP_GUARD_I fuses only a plain uop)
    assert_eq!(done(0xE355_0010), uop(k::CMP_IMM, (0, 5, 0, 0, 0, 0x10)));
    // tst keeps the carry bit: 1 when rot == 0, 0 otherwise
    assert_eq!(done(0xE315_0010), uop(k::TST_IMM, (0, 5, 0, 0, 1, 0x10)));
    assert_eq!(
        done(0xE315_020F),
        uop(k::TST_IMM, (0, 5, 0, 0, 0, 0xF000_0000))
    );
    // unused fields are canonical: Rd of a compare, Rn of a mov
    assert_eq!(done(0xE350_F000), uop(k::CMP_IMM, (0, 0, 0, 0, 0, 0)));
    assert_eq!(done(0xE3AF_0001), uop(k::MOV_IMM, (0, 0, 0, 0, 0, 1)));
}

#[test]
fn registers_with_lsl_0_pick_the_specialised_id_and_any_shift_stays_generic() {
    assert_eq!(done(0xE1A0_1002), uop(k::MOV_REG, (1, 0, 2, 0, 0, 0)));
    assert_eq!(done(0xE081_0002), uop(k::ADD_REG, (0, 1, 2, 0, 0, 0)));
    assert_eq!(done(0xE041_0002), uop(k::SUB_REG, (0, 1, 2, 0, 0, 0)));
    assert_eq!(done(0xE151_0002), uop(k::CMP_REG, (0, 1, 2, 0, 0, 0)));
    assert_eq!(done(0xE111_0002), uop(k::TST_REG, (0, 1, 2, 0, 0, 0)));
    // the same instructions shifted, or S, or without a specialised id: generic ids
    let id = k::dp;
    assert_eq!(done(0xE1B0_0001), uop(id(1, 13, true), (0, 0, 1, 0, 0, 0))); // movs
    assert_eq!(done(0xE091_0002), uop(id(1, 4, true), (0, 1, 2, 0, 0, 0))); // adds
    assert_eq!(done(0xE181_0002), uop(id(1, 12, false), (0, 1, 2, 0, 0, 0))); // orr
    assert_eq!(done(0xE1E0_0001), uop(id(1, 15, false), (0, 0, 1, 0, 0, 0))); // mvn
    assert_eq!(done(0xE041_0102), uop(id(1, 2, false), (0, 1, 2, 0, 2, 0))); // sub .., lsl #2
    assert_eq!(done(0xE171_0002), uop(id(1, 11, true), (0, 1, 2, 0, 0, 0))); // cmn
    assert_eq!(done(0xE331_0000), uop(id(0, 9, true), (0, 1, 0, 0, 1, 0))); // teq
}

#[test]
fn the_shifter_fields_follow_the_table_of_the_design_note() {
    let id = k::dp;
    // form 0: the rot == 0 flag for every generic id, the rotated value in imm
    assert_eq!(done(0xE291_0001), uop(id(0, 4, true), (0, 1, 0, 0, 1, 1)));
    assert_eq!(
        done(0xE201_0F3F),
        uop(id(0, 0, false), (0, 1, 0, 0, 0, 0xFC))
    );
    // form 1: type << 5 | raw amount, including the special encodings
    assert_eq!(done(0xE001_0182), uop(id(1, 0, false), (0, 1, 2, 0, 3, 0))); // lsl #3
    assert_eq!(
        done(0xE1A0_0021),
        uop(id(1, 13, false), (0, 0, 1, 0, 0x20, 0))
    ); // lsr #32
    assert_eq!(
        done(0xE1A0_0041),
        uop(id(1, 13, false), (0, 0, 1, 0, 0x40, 0))
    ); // asr #32
    assert_eq!(
        done(0xE1A0_0061),
        uop(id(1, 13, false), (0, 0, 1, 0, 0x60, 0))
    ); // rrx
    assert_eq!(
        done(0xE1A0_0FC1),
        uop(id(1, 13, false), (0, 0, 1, 0, 0x5F, 0))
    ); // asr #31
       // form 2: type << 5, rm and rs
    assert_eq!(done(0xE081_0312), uop(id(2, 4, false), (0, 1, 2, 3, 0, 0))); // lsl r3
    assert_eq!(
        done(0xE1A0_0271),
        uop(id(2, 13, false), (0, 0, 1, 2, 0x60, 0))
    ); // ror r2
}

#[test]
fn add_rd_pc_imm_and_mov_rd_pc_fold_to_a_constant_and_nothing_else_does() {
    assert_eq!(done(0xE28F_0004), uop(k::MOV_IMM, (0, 0, 0, 0, 0, 0x200C)));
    assert_eq!(done(0xE28F_1E0F), uop(k::MOV_IMM, (1, 0, 0, 0, 0, 0x20F8)));
    assert_eq!(done(0xE1A0_200F), uop(k::MOV_IMM, (2, 0, 0, 0, 0, 0x2008)));
    // the condition does not change the lowering: the registry wraps the predicate
    assert_eq!(done(0x028F_0004), done(0xE28F_0004));
    // the address wraps like the interpreter's
    let (_, out) = lower(0xE28F_0004, 0xFFFF_FFF8);
    assert_eq!(
        (out[0].kind, out[0].imm, out[0].pc),
        (k::MOV_IMM, 4, 0xFFFF_FFF8)
    );
    for word in [
        0xE29F_0004, // adds r0,pc,#4 (S)
        0xE24F_0004, // sub r0,pc,#4: not one of the two folds
        0xE38F_0004, // orr r0,pc,#4
        0xE1A0_010F, // mov r0,pc,lsl #2
        0xE1B0_000F, // movs r0,pc
        0xE081_000F, // add r0,r1,pc
        0xE08F_0001, // add r0,pc,r1
        0xE35F_0000, // cmp pc,#0
        0xE15F_0001, // cmp pc,r1
    ] {
        generic(word);
    }
}

#[test]
fn a_register_shifted_operand_is_never_folded_and_rs_is_never_the_pc() {
    generic(0xE1A0_011F); // mov r0,pc,lsl r1 (reads pc + 12)
    generic(0xE08F_0211); // add r0,pc,r1,lsl r2
    generic(0xE081_021F); // add r0,r1,pc,lsl r2
    generic(0xE1A0_0F11); // mov r0,r1,lsl pc
                          // mov ignores Rn, so an Rn of 15 is not a pc operand
    assert_eq!(done(0xE1AF_0211).kind, k::dp(2, 13, false));
}

#[test]
fn a_destination_of_pc_is_left_to_lower_flow_but_a_compare_ignores_it() {
    for word in [
        0xE1A0_F001, // mov pc,r1
        0xE1B0_F00E, // movs pc,lr
        0xE281_F001, // add pc,r1,#1
        0xE3A0_F000, // mov pc,#0
        0xE25E_F004, // subs pc,lr,#4
        0xE1A0_F211, // mov pc,r1,lsl r2
    ] {
        generic(word);
    }
    assert_eq!(done(0xE151_F001).kind, k::CMP_REG); // cmp r1,r1 with Rd = 15
    assert_eq!(done(0xE331_F000).kind, k::dp(0, 9, true)); // teq r1,#0 with Rd = 15
    assert_eq!(lower(0xE1A0_F001, PC0).0, super::Lowered::Generic);
}

#[test]
fn instructions_outside_the_family_are_not_lowered() {
    for word in [
        0xE001_0092, // mul
        0xE591_0000, // ldr
        0xE12F_FF1E, // bx lr
        0xE129_F000, // msr cpsr_fc,r0
        0xE10F_3000, // mrs r3,cpsr
        0xE1A0_0090, // swp space
        0xF57F_F05F, // the unconditional space
        0xEA00_0000, // b
    ] {
        generic(word);
    }
}
