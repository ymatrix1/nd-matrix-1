//! Installation of a block's uop boundaries into the pc map.
//!
//! OWNER: W1-LOWER. Every uop that starts at a distinct pc is an entry point, so a block can be
//! re-entered after a `Slow` exit at any uop boundary (steps inside a fused uop that are not
//! boundaries are finished by `Core::step`). Skeleton: [`install`] is complete.
use super::super::hot::PcMap;
use super::super::types::Uop;

/// Maps the pc of each uop of block `id` to `(id, index)`. When several uops start at the same pc
/// (the second uop of one instruction) the first one is the entry.
pub fn install(map: &mut PcMap, id: u32, uops: &[Uop]) {
    for (i, u) in uops.iter().enumerate().rev() {
        map.set(u.pc, (id, i as u32));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::UopKind;

    #[test]
    fn the_first_uop_at_a_pc_is_the_entry_and_every_boundary_is_mapped() {
        let u = |pc| Uop::new(UopKind::NOP, 1, pc, 0, 0);
        let mut m = PcMap::new();
        install(
            &mut m,
            7,
            &[u(0x12_0000), u(0x12_0001), u(0x12_0001), u(0x12_0004)],
        );
        assert_eq!(m.get(0x12_0000), Some((7, 0)));
        assert_eq!(
            m.get(0x12_0001),
            Some((7, 1)),
            "the first of two uops at one pc"
        );
        assert_eq!(m.get(0x12_0004), Some((7, 3)));
        assert_eq!(m.get(0x12_0002), None);
    }
}
