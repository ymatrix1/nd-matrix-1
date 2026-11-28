//! The instruction pools of the fuzz: valid encodings per lowering family, found by asking the
//! lowering itself.
//!
//! OWNER: W3-FUZZ. A pool entry is a whole instruction (parcels) that one family takes as a fast
//! uop, so a program drawn from a pool exercises that family's handlers and not the `Interp`
//! fallback (P10). Two sources, both deterministic: the committed census
//! (`gen::census_encodings`, simulator-measured, so every opcode the hardware was probed with)
//! and random parcels that the VISA classifier reads as one whole instruction (so opcodes and
//! register fields the census never tried, steered half of the time to the unconditional form).
//! The family decides what it takes ([`Family::lower`] on the decoded instruction), no opcode
//! table is repeated here, so the pools follow the lowering when it grows.
use crate::core::decode_cache::{CachedInsn, DecodedOp, MoveOp};
use crate::core::uop::lower::builder::BlockBuilder;
use crate::core::uop::lower::fixed::FixedFamily;
use crate::core::uop::lower::float::FloatFamily;
use crate::core::uop::lower::moves::MoveFamily;
use crate::core::uop::lower::{Family, LowerCtx};
use crate::core::uop::types::UF_CB;
use crate::core::uop::verify::gen::{census_encodings, Rng, CODE_PC};
use crate::core::uop::UopConfig;
use crate::visa::{bits, classify, word48, Insn, Kind};
use std::sync::OnceLock;

/// A lowering family with fast uops that a straight program can use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fam {
    Float,
    Fixed,
    Move,
}

/// Normal-word window the data accesses of the moves stay in by default (the code of a program
/// sits at normal word `0x9_0000`, so a program of up to 32 k parcels never reaches it).
/// ASSUMED: any SRAM region away from the code does; the same place the `h_move` tests use.
pub(super) const DATA: u32 = 0x9_8000;

/// Words around [`DATA`] the state filler writes and the absolute accesses may name.
pub(super) const DATA_SPAN: u32 = 0x300;

/// The instruction at [`CODE_PC`] as the decode path would hand it to the lowering, or `None`
/// when the parcels are not exactly one whole instruction of a known kind.
pub(super) fn cached(parcels: &[u16]) -> Option<CachedInsn> {
    if parcels.is_empty() || parcels.len() > 3 {
        return None;
    }
    let mut p = [0u16; 3];
    p[..parcels.len()].copy_from_slice(parcels);
    let (kind, len) = classify(p[0], parcels.get(1).copied());
    if kind == Kind::Unknown || len != parcels.len() {
        return None;
    }
    let w = word48(&p[..len]);
    let insn = Insn {
        kind,
        parcels: p,
        len,
    };
    Some(CachedInsn {
        pc: CODE_PC,
        gen: 0,
        next_pc: CODE_PC + len as u32,
        isa: false,
        cond: bits(w, 37, 33) as u8,
        insn,
        w,
        op: DecodedOp::from_insn(CODE_PC, &insn, w),
    })
}

/// The lowering function of a family.
type Lower = fn(&LowerCtx<'_>, &CachedInsn, &mut BlockBuilder) -> bool;

/// Which family takes `c` as a fast uop in a block of PEy mode `simd`, and whether that uop has
/// the circular-buffer latch bit (`UF_CB`).
fn lowered_by(c: &CachedInsn, simd: bool) -> Option<(Fam, bool)> {
    let cfg = UopConfig::default();
    let cx = LowerCtx {
        cfg: &cfg,
        stop_pc: None,
        simd,
        isa: false,
    };
    let families: [(Fam, Lower); 3] = [
        (Fam::Float, FloatFamily::lower),
        (Fam::Fixed, FixedFamily::lower),
        (Fam::Move, MoveFamily::lower),
    ];
    families.into_iter().find_map(|(fam, lower)| {
        let mut b = BlockBuilder::new(CODE_PC, simd, false);
        let taken = lower(&cx, c, &mut b);
        taken.then(|| (fam, b.uops().iter().any(|u| u.flags & UF_CB != 0)))
    })
}

/// The family that takes `parcels` in both PEy modes, with the latch flag, or `None`.
pub(super) fn family_of(parcels: &[u16]) -> Option<(Fam, bool)> {
    let c = cached(parcels)?;
    let (a, b) = (lowered_by(&c, false)?, lowered_by(&c, true)?);
    (a.0 == b.0).then_some(a)
}

/// Whether an absolute address stays in a window the state filler populates (`h_move` uses the
/// same list of places a transfer meets): SRAM data, the IOP array, the short-word alias, SDRAM
/// and the device store. An absolute access anywhere else is a bad-address fault, which both
/// engines raise alike but which ends the run, so the pools leave it out.
fn safe_address(a: u32) -> bool {
    let around = |base: u32, n: u32| a >= base && a - base < n;
    around(DATA - 8, DATA_SPAN + 8) || around(0x3_E000, 8) || around(0x70_0000, 8)
}

/// A random whole instruction: three random parcels, the condition field forced to TRUE half of
/// the time, read back through the classifier. `None` when the draw is not one instruction.
fn random_insn(rng: &mut Rng) -> Option<Vec<u16>> {
    let mut p = [
        rng.next_u32() as u16,
        rng.next_u32() as u16,
        rng.next_u32() as u16,
    ];
    if rng.below(2) == 0 {
        p[0] |= 0x3E; // bits 37..33 of the 48-bit word: condition 31, TRUE
    }
    let (_, len) = classify(p[0], Some(p[1]));
    (1..=3).contains(&len).then(|| p[..len].to_vec())
}

/// The pools of one process.
pub(super) struct Pools {
    pub float: Vec<Vec<u16>>,
    pub fixed: Vec<Vec<u16>>,
    pub moves: Vec<Vec<u16>>,
    /// Moves whose uop latches the circular-buffer wrap of I7 or I15.
    pub cb: Vec<Vec<u16>>,
}

impl Pools {
    /// The pool of `fam`.
    pub fn of(&self, fam: Fam) -> &[Vec<u16>] {
        match fam {
            Fam::Float => &self.float,
            Fam::Fixed => &self.fixed,
            Fam::Move => &self.moves,
        }
    }
}

/// Random draws tried per family; ASSUMED: enough for the random half of a pool to hold a few
/// hundred entries (`the_pools_are_full_of_fast_instructions_of_their_own_family` pins the size).
const TRIES: u32 = 120_000;
/// Entries kept per family from the random draws.
const RANDOM_KEEP: usize = 300;

/// The pools, built once from the census and from random draws with a fixed seed.
pub(super) fn pools() -> &'static Pools {
    static POOLS: OnceLock<Pools> = OnceLock::new();
    POOLS.get_or_init(build)
}

fn build() -> Pools {
    let mut p = Pools {
        float: Vec::new(),
        fixed: Vec::new(),
        moves: Vec::new(),
        cb: Vec::new(),
    };
    let sort = |parcels: Vec<u16>, p: &mut Pools| {
        let Some((fam, cb)) = family_of(&parcels) else {
            return;
        };
        if fam == Fam::Move && !move_is_tame(&parcels) {
            return;
        }
        match fam {
            Fam::Float => p.float.push(parcels),
            Fam::Fixed => p.fixed.push(parcels),
            Fam::Move if cb => p.cb.push(parcels),
            Fam::Move => p.moves.push(parcels),
        }
    };
    for e in census_encodings(|_, _| true) {
        sort(e.to_vec(), &mut p);
    }
    let mut rng = Rng::new(0x51E7_E000_0000_0001);
    for _ in 0..TRIES {
        if let Some(parcels) = random_insn(&mut rng) {
            let kept = |v: &Vec<Vec<u16>>| v.len();
            let full = kept(&p.float).min(kept(&p.fixed)).min(kept(&p.moves)) >= RANDOM_KEEP
                && kept(&p.cb) >= RANDOM_KEEP / 4;
            if full {
                break;
            }
            // a family that is full stops taking draws, so the rare ones fill up
            let skip = match family_of(&parcels).map(|f| f.0) {
                Some(Fam::Float) => p.float.len() >= RANDOM_KEEP * 2,
                Some(Fam::Fixed) => p.fixed.len() >= RANDOM_KEEP * 2,
                Some(Fam::Move) => p.moves.len() >= RANDOM_KEEP * 2,
                None => true,
            };
            if !skip {
                sort(parcels, &mut p);
            }
        }
    }
    p
}

/// Whether a move instruction keeps the run going: an absolute access names a populated window
/// (the register-indirect forms are steered by the DAG filler instead).
fn move_is_tame(parcels: &[u16]) -> bool {
    match cached(parcels).map(|c| c.op) {
        Some(DecodedOp::Move(MoveOp::DirectMem { addr, .. })) => safe_address(addr),
        Some(DecodedOp::Move(_)) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;
    use crate::core::uop::lower::decode::decode_at;

    #[test]
    fn the_cached_form_equals_the_decode_path_of_the_interpreter() {
        for parcels in [
            vec![0x0001],
            vec![0x013e, 0x0000, 0x1312],
            vec![0x018e, 0x000a],
        ] {
            let mut core = core_with(&parcels);
            let want = decode_at(&mut core, CODE_PC);
            let got = cached(&parcels).unwrap();
            assert_eq!(
                (got.pc, got.next_pc, got.w, got.insn.len, got.cond),
                (want.pc, want.next_pc, want.w, want.insn.len, want.cond)
            );
            assert_eq!(format!("{:?}", got.op), format!("{:?}", want.op));
        }
        assert!(cached(&[]).is_none() && cached(&[0x013e]).is_none());
    }

    #[test]
    fn the_pools_are_full_of_fast_instructions_of_their_own_family() {
        let p = pools();
        for (name, fam, pool) in [
            ("float", Fam::Float, &p.float),
            ("fixed", Fam::Fixed, &p.fixed),
            ("moves", Fam::Move, &p.moves),
        ] {
            assert!(pool.len() >= 100, "{name}: {} entries", pool.len());
            assert!(pool.iter().all(|e| family_of(e).map(|f| f.0) == Some(fam)));
        }
        assert!(!p.cb.is_empty(), "no move latches the circular-buffer wrap");
        assert!(p.cb.iter().all(|e| family_of(e).is_some_and(|f| f.1)));
        assert!(p.moves.iter().all(|e| family_of(e).is_some_and(|f| !f.1)));
        assert_eq!(p.of(Fam::Move).len(), p.moves.len());
    }

    #[test]
    fn the_pools_hold_encodings_the_census_never_tried() {
        let census: std::collections::BTreeSet<_> =
            census_encodings(|_, _| true).into_iter().collect();
        let p = pools();
        for pool in [&p.float, &p.fixed, &p.moves] {
            let new = pool
                .iter()
                .filter(|e| !census.contains(e.as_slice()))
                .count();
            assert!(new >= 20, "only {new} entries beyond the census");
        }
    }

    #[test]
    fn a_non_instruction_and_a_flow_instruction_belong_to_no_pool_family() {
        assert_eq!(
            family_of(&[0x0001]),
            None,
            "a NOP is not a fast uop of these families"
        );
        assert_eq!(
            family_of(&[0x063e, 0x0012, 0x0000]),
            None,
            "a jump is the flow family"
        );
        assert!(safe_address(DATA) && !safe_address(0x1234_5678));
        assert!(!move_is_tame(&[0x0001]));
    }
}
