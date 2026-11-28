//! Block-local register promotion (`FAM_REGPROMO`), stretch goal.
//!
//! OWNER: W6-NREG. Keeps the registers a block uses in host registers between its uops and writes
//! them back at every exit (verified by the exit sweep on all blocks). Built only if gate K6
//! passes (tag and type specialization applies to at least 60 percent of float ops). Skeleton: no
//! block is promotable.
use crate::core::uop::types::Block;

/// Whether `block` may run with promoted registers.
pub fn promotable(block: &Block) -> bool {
    let _ = block;
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skeleton_promotes_nothing() {
        assert!(!promotable(&Block::empty(0x12_0000)));
    }
}
