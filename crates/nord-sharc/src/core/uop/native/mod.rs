//! Tier 2: native code for the blocks of Tier 1, emitted with Cranelift (feature `uop-native`).
//!
//! OWNER: W4-NCORE (`ctx`, `emit`, `call`, `enter`, `cache`), W4-NSVC (`service`, `tv`), W5-NFLOAT
//! (`inline_float`), W5-NMOVE (`inline_move`, `inline_flow`), W6-NREG (`regpromo`). A block runs
//! natively only when `UopConfig::native` is set and its `Block::native` slot holds an entry; a
//! block without one runs in Tier 1, so an engine with every native stub (this skeleton) is
//! Tier 1 exactly. This file declares the tree and the family bits of `UopConfig::families`.
pub mod cache;
pub mod call;
pub mod ctx;
pub mod emit;
pub mod enter;
pub mod inline_float;
pub mod inline_flow;
pub mod inline_move;
pub mod regpromo;
pub mod service;
pub mod tv;

pub use cache::NativeTable;
pub(crate) use emit::Emitter;
pub(crate) use enter::enter;

/// Call-threaded emission: each uop is a call of its Tier 1 handler.
pub const FAM_CALL: u32 = 1;
/// Inline float family (add, sub, mul, avg, select, compare, convert).
pub const FAM_FLOAT: u32 = 2;
/// Inline moves and memory.
pub const FAM_MOVE: u32 = 4;
/// Inline flow.
pub const FAM_FLOW: u32 = 8;
/// Block-local register promotion.
pub const FAM_REGPROMO: u32 = 16;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_bits_are_distinct_single_bits() {
        let bits = [FAM_CALL, FAM_FLOAT, FAM_MOVE, FAM_FLOW, FAM_REGPROMO];
        assert!(bits.iter().all(|b| b.count_ones() == 1));
        assert_eq!(bits.iter().fold(0, |m, b| m | b), 31);
    }
}
