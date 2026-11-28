//! Block discovery: the leaders of the code range.
//!
//! OWNER: W1-LOWER. Design 4.2: a block starts at a pass entry, a static jump or call target, the
//! instruction after a terminator and the first instruction of a loop body; dynamic targets
//! (computed jumps, returns) land on a block start or on a uop boundary found through the pc map.
//! Skeleton: every instruction start is a leader, which together with the cut-after-every-
//! instruction rule of `cut.rs` gives one-instruction blocks.
use super::decode::decode_at;
use crate::core::Core;

/// Instruction starts in `lo..hi` in ascending order, found by walking the code from `lo`.
pub(in crate::core) fn leaders(core: &mut Core, lo: u32, hi: u32) -> Vec<u32> {
    let mut v = Vec::new();
    let mut pc = lo;
    while pc < hi {
        v.push(pc);
        let next = decode_at(core, pc).next_pc;
        if next <= pc {
            break;
        }
        pc = next;
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;

    #[test]
    fn leaders_walk_instruction_starts_by_length() {
        // NOP, R3 = R1 + R2 (3 parcels), NOP, NOP
        let mut c = core_with(&[0x0001, 0x013e, 0x0000, 0x1312, 0x0001, 0x0001]);
        let pc = c.pc;
        assert_eq!(leaders(&mut c, pc, pc + 6), [pc, pc + 1, pc + 4, pc + 5]);
        assert_eq!(
            leaders(&mut c, pc, pc + 2),
            [pc, pc + 1],
            "the last start may run past hi"
        );
        assert!(leaders(&mut c, pc, pc).is_empty());
    }
}
