//! Lowering of move instructions to uops.
//!
//! OWNER: W2-MOVE. Register and memory moves: DAG loads and stores (post- and pre-modify, with a
//! modify register or an immediate, circular buffers, bit-reversed addressing), absolute and
//! offset accesses, immediate loads and stores, `MODIFY` / `BITREV`, register moves and swaps and
//! the dual data moves. `form` decides which instructions are taken and what each becomes (the
//! kinds and operand layout are in `exec/h_move/layout.rs`); the handlers are in `exec/h_move/`.
//!
//! Everything the handlers cannot reproduce stays an `Interp` uop (exact by construction): a
//! conditional move, a long-word access, a move with a compute in parallel and any access to a
//! system register. `docs/specs/sharc-uop.md` lists the forms.
mod form;

use super::builder::BlockBuilder;
use super::{Family, LowerCtx};
use crate::core::decode_cache::CachedInsn;

/// The move family.
pub(in crate::core) struct MoveFamily;

impl Family for MoveFamily {
    fn lower(cx: &LowerCtx<'_>, c: &CachedInsn, out: &mut BlockBuilder) -> bool {
        match form::lowered(cx, c) {
            Some(u) => {
                out.push(u);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::CODE;
    use crate::core::uop::lower::lower_first;
    use crate::core::uop::lower::support::{build_default, spans, JUMP_DB, NOP};
    use crate::core::uop::types::{GuardClass, KindFamily, UopKind, UF_CB, UF_SIMD};

    /// `R0 = DM(I2, M2)`, one parcel (type 3c).
    const LOAD: u16 = (0b1001 << 12) | (2 << 9) | (2 << 6) | (1 << 4);
    /// `R1 = DM(I7, 2)` post-modify (type 4b, two parcels): 6c7e with the index field set to 7.
    const WRAP: [u16; 2] = [0x6c7e & !(7 << 9) | 7 << 9, 0x10bf];

    #[test]
    fn nop_and_compute_are_not_move_forms() {
        assert_eq!(lower_first::<MoveFamily>(&[NOP]), (false, 0));
        assert_eq!(
            lower_first::<MoveFamily>(&[0x013e, 0x0000, 0x1312]),
            (false, 0)
        );
        assert_eq!(lower_first::<MoveFamily>(&[LOAD]), (true, 1));
    }

    #[test]
    fn a_taken_instruction_emits_exactly_one_move_uop_for_one_step() {
        let (_, built) = build_default(&[LOAD, LOAD, NOP]);
        let (block, uops) = built.at(CODE, false).unwrap();
        assert_eq!(block.steps, 3);
        let kinds: Vec<_> = uops.iter().map(|u| UopKind(u.h).family()).collect();
        assert_eq!(
            kinds,
            [
                Some(KindFamily::Move),
                Some(KindFamily::Move),
                Some(KindFamily::Core)
            ],
            "two moves, the NOP is not ours"
        );
        assert_eq!(
            uops.iter().map(|u| (u.pc, u.steps)).collect::<Vec<_>>(),
            [(CODE, 1), (CODE + 1, 1), (CODE + 2, 1)]
        );
    }

    #[test]
    fn the_simd_flag_follows_the_mode_the_block_was_translated_for() {
        let (_, built) = build_default(&[LOAD, NOP]);
        let flag = |simd| built.at(CODE, simd).unwrap().1[0].flags & UF_SIMD;
        assert_eq!((flag(false), flag(true)), (0, UF_SIMD));
    }

    #[test]
    fn a_post_modify_through_i7_ends_its_block_and_the_others_do_not() {
        let program = [&WRAP[..], &[NOP; 3]].concat();
        let (_, built) = build_default(&program);
        assert_eq!(spans(&built, false), [(CODE, 1), (CODE + 2, 3)]);
        let (block, uops) = built.at(CODE, false).unwrap();
        assert_eq!(uops[0].flags & UF_CB, UF_CB);
        assert_eq!(block.guard, GuardClass::Cb);
        let (_, built) = build_default(&[LOAD, LOAD, LOAD]);
        assert_eq!(
            spans(&built, false),
            [(CODE, 3)],
            "I2 never raises CB7I / CB15I"
        );
    }

    #[test]
    fn in_a_delay_group_a_wrap_candidate_is_left_to_the_interpreter() {
        // NOP, JUMP (DB), slot, slot: the slot after the branch cannot end the block
        let program = [&[NOP][..], &JUMP_DB, &WRAP, &[NOP; 2]].concat();
        let (_, built) = build_default(&program);
        let (_, uops) = built.at(CODE, false).unwrap();
        let slot = uops.iter().find(|u| u.pc == CODE + 4).unwrap();
        assert_eq!(
            slot.h,
            UopKind::INTERP.0,
            "the translator replaced our UF_CB uop"
        );
    }

    #[test]
    fn a_move_in_a_delay_slot_gets_the_finish_step_flag() {
        use crate::core::uop::types::UF_FIN;
        let program = [&[NOP][..], &JUMP_DB, &[LOAD, LOAD]].concat();
        let (_, built) = build_default(&program);
        let (_, uops) = built.at(CODE, false).unwrap();
        let slots: Vec<u8> = uops
            .iter()
            .filter(|u| u.pc >= CODE + 4)
            .map(|u| u.flags & UF_FIN)
            .collect();
        assert_eq!(slots, [UF_FIN, UF_FIN], "every slot runs finish_step");
        assert!(uops
            .iter()
            .all(|u| u.h != UopKind::INTERP.0 || u.pc < CODE + 4));
    }
}
