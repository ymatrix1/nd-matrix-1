//! The decode path of lowering: instructions are read exactly as `Core::step` reads them, so a
//! block is built from the same `CachedInsn` the interpreter would execute.
//!
//! OWNER: W1-LOWER. [`decode_at`] reads the decode cache first, then `fetch_and_cache`, which
//! also widens `sram_code_nw_lo/hi` so stores into translated code reach the invalidation hook.
//! [`code_hash`] is the one hash both the translator and revalidation use (rule P6).
//! [`window_hides`] is the one place that knows which pcs the decode path cannot be trusted for.
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

/// Whether `pc` lies in an open interrupt-vector window. `Core::fetch` reads those parcels in ISA
/// order (`isa_window_parcels`), so a decode made now would give the instruction boundaries of the
/// vector slot, not those of the same parcels read as VISA code once the window is closed. The
/// translator leaves such pcs to the interpreter (no block starts or runs through them), which is
/// exact by construction; the entry guard (`Window` row) already keeps blocks out of a window.
pub(in crate::core) fn window_hides(core: &Core, pc: u32) -> bool {
    core.irq.window_len != 0 && pc.wrapping_sub(core.irq.window_lo) < core.irq.window_len
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
    fn an_open_vector_window_hides_exactly_its_own_pcs() {
        let mut c = core_with(&[0x0001; 4]);
        let pc = c.pc;
        assert!(!window_hides(&c, pc), "no window open");
        (c.irq.window_lo, c.irq.window_len) = (pc + 1, 2);
        assert_eq!(
            [pc, pc + 1, pc + 2, pc + 3].map(|p| window_hides(&c, p)),
            [false, true, true, false]
        );
        // a pc below the window start wraps to a huge distance and is not hidden
        assert!(!window_hides(&c, 0));
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
