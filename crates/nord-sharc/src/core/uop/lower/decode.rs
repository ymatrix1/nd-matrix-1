//! The decode path of lowering: instructions are read exactly as `Core::step` reads them, so a
//! block is built from the same `CachedInsn` the interpreter would execute.
//!
//! OWNER: W1-LOWER. Skeleton: [`decode_at`] is complete (decode cache first, then
//! `fetch_and_cache`, which also widens `sram_code_nw_lo/hi` so stores into translated code
//! reach the invalidation hook).
use crate::core::decode_cache::CachedInsn;
use crate::core::Core;

/// The instruction at `pc` as `Core::step` would execute it.
pub(in crate::core) fn decode_at(core: &mut Core, pc: u32) -> CachedInsn {
    let isa = core.isa_at(pc);
    let hit = core
        .decode_cache
        .as_deref()
        .and_then(|cache| cache.get(pc, core.code_gen, isa))
        .copied();
    match hit {
        Some(c) => c,
        None => core.fetch_and_cache(pc, isa),
    }
}

/// Hash of the code parcels `lo..hi` (FNV-1a, 64 bit, over the parcels in address order). The one
/// function both sides use: the translator stores it in `Block::code_hash` (parcels
/// `block.pc..block.end_pc`, so the last instruction is covered whatever its length) and
/// revalidation after a write recomputes it over the same range, so a block whose code was
/// rewritten with identical words is kept. ASSUMED: 64 bits make a collision on structured code
/// negligible (about 2^-64 per block); a stale block that survived would show in the exit sweep
/// and in lockstep on a program-change trace.
pub(in crate::core) fn code_hash(core: &Core, lo: u32, hi: u32) -> u64 {
    const BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    (lo..hi).fold(BASIS, |h, pc| {
        (h ^ u64::from(core.parcel_at(pc))).wrapping_mul(PRIME)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;

    #[test]
    fn decode_at_reads_lengths_and_the_next_pc_like_the_interpreter() {
        // NOP (1 parcel), R3 = R1 + R2 (3 parcels), NOP
        let mut c = core_with(&[0x0001, 0x013e, 0x0000, 0x1312, 0x0001]);
        let pc = c.pc;
        let a = decode_at(&mut c, pc);
        let b = decode_at(&mut c, a.next_pc);
        let b_again = decode_at(&mut c, a.next_pc);
        assert_eq!((a.pc, a.next_pc, a.isa), (pc, pc + 1, false));
        assert_eq!((b.pc, b.next_pc), (pc + 1, pc + 4));
        assert_eq!(
            (b.w, b.insn.len),
            (b_again.w, 3),
            "the second read is a cache hit"
        );
        // one interpreter step on the same code lands on the pc the decode says
        c.step().unwrap();
        assert_eq!(c.pc, a.next_pc);
    }

    #[test]
    fn code_hash_follows_the_words_and_only_the_words() {
        let mut c = core_with(&[0x0001, 0x013e, 0x0000, 0x1312]);
        let (lo, hi) = (c.pc, c.pc + 4);
        let h = code_hash(&c, lo, hi);
        assert_eq!(h, code_hash(&c, lo, hi), "deterministic");
        assert_ne!(h, code_hash(&c, lo, hi - 1), "the range matters");
        c.dm_write(lo / 2, 0x013e_0001).unwrap(); // the same two parcels again
        assert_eq!(h, code_hash(&c, lo, hi), "identical words keep the hash");
        c.dm_write(lo / 2, 0x013e_0002).unwrap();
        assert_ne!(h, code_hash(&c, lo, hi), "a changed parcel changes it");
    }
}
