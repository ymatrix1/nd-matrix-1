//! Instruction-count probes of the move handlers: one `#[inline(never)]` function per hot
//! handler body, whose symbol `uop_probe_m_*` `tools/uop_insn_count.py` finds in the compiler
//! assembly of the test build (`--emit=asm`) and counts. Every handler body is an
//! `#[inline(always)]` function of this module tree and the table entry is a one-call wrapper, so
//! a probe is exactly what the dispatch loop pays for the uop, cold paths included as the tool
//! counts them (they are `#[cold]` calls, one `call` instruction each).
//!
//! OWNER: W2-MOVE, test build only. A count of instructions, never of time. The test below runs
//! every probe through `black_box` (which keeps the symbols in the test binary) next to the table
//! handler of the same uop on a second core, in scalar and SIMD mode on random states: a probe
//! that measured a different body than the handler would fail it.
use super::dual::dual;
use super::imm::{imm_store, load_imm};
use super::layout::{CLASS_OTHER, CLASS_R, CLASS_S, DIRECT, POST_IMM, POST_M, PRE_IMM};
use super::xfer::xfer;
use crate::core::uop::types::{Uop, UopCtx};

type Body = unsafe fn(*mut UopCtx, *const Uop) -> u32;

/// One probe: the handler body `$call` (an `#[inline(always)]` function item) behind a symbol of
/// its own.
macro_rules! probe {
    ($name:ident, $call:expr) => {
        #[inline(never)]
        pub(in crate::core) unsafe fn $name(ctx: *mut UopCtx, u: *const Uop) -> u32 {
            $call(ctx, u)
        }
    };
}

// the hot DAG loads and stores: post-modify with a modify register, with an immediate,
// pre-modify with an immediate, absolute
probe!(uop_probe_m_load_post_r, xfer::<false, POST_M, CLASS_R>);
probe!(uop_probe_m_store_post_r, xfer::<true, POST_M, CLASS_R>);
probe!(uop_probe_m_load_postimm_r, xfer::<false, POST_IMM, CLASS_R>);
probe!(uop_probe_m_store_postimm_r, xfer::<true, POST_IMM, CLASS_R>);
probe!(uop_probe_m_load_preimm_r, xfer::<false, PRE_IMM, CLASS_R>);
probe!(uop_probe_m_store_preimm_r, xfer::<true, PRE_IMM, CLASS_R>);
probe!(uop_probe_m_load_preimm_s, xfer::<false, PRE_IMM, CLASS_S>);
probe!(
    uop_probe_m_load_preimm_other,
    xfer::<false, PRE_IMM, CLASS_OTHER>
);
probe!(uop_probe_m_load_direct_r, xfer::<false, DIRECT, CLASS_R>);
probe!(uop_probe_m_store_direct_r, xfer::<true, DIRECT, CLASS_R>);
// the immediate load and store, and the dual move
probe!(uop_probe_m_load_imm_r, load_imm::<CLASS_R>);
probe!(uop_probe_m_imm_store, imm_store);
probe!(uop_probe_m_dual, dual);

/// Every probe with the instruction that makes the uop it measures.
#[cfg(test)]
pub(super) fn all() -> Vec<(&'static str, Body, Vec<u16>)> {
    let t3c = (0b1001u16 << 12) | (2 << 9) | (2 << 6) | (1 << 4);
    vec![
        ("load_post_r", uop_probe_m_load_post_r, vec![t3c]),
        ("store_post_r", uop_probe_m_store_post_r, vec![t3c | 0x20]),
        (
            "load_postimm_r",
            uop_probe_m_load_postimm_r,
            vec![0x6c7e, 0x10bf],
        ),
        (
            "store_postimm_r",
            uop_probe_m_store_postimm_r,
            vec![0x6c7e | 0x80, 0x10bf],
        ),
        (
            "load_preimm_r",
            uop_probe_m_load_preimm_r,
            vec![0x6c3e, 0x10bf],
        ),
        (
            "store_preimm_r",
            uop_probe_m_store_preimm_r,
            vec![0x6c3e | 0x80, 0x10bf],
        ),
        (
            "load_preimm_s",
            uop_probe_m_load_preimm_s,
            vec![0x9e08, 0x53 << 7],
        ),
        (
            "load_preimm_other",
            uop_probe_m_load_preimm_other,
            vec![0x9e08, 0x0c00],
        ),
        (
            "load_direct_r",
            uop_probe_m_load_direct_r,
            vec![0x1000, 0x0009, 0x2196],
        ),
        (
            "store_direct_r",
            uop_probe_m_store_direct_r,
            vec![0x1100, 0x0009, 0x2196],
        ),
        (
            "load_imm_r",
            uop_probe_m_load_imm_r,
            vec![0x0f0e, 0x3eb0, 0xf34d],
        ),
        (
            "imm_store",
            uop_probe_m_imm_store,
            vec![0x9040, 0x0000, 0x0000],
        ),
        ("dual", uop_probe_m_dual, vec![0x3d89, 0xc83f]),
    ]
}

#[cfg(test)]
mod tests {
    use super::super::support::{build, lower};
    use super::*;
    use crate::core::uop::hot::UopHot;
    use crate::core::uop::verify::compare::exact_diff;
    use std::hint::black_box;

    #[test]
    fn every_probe_runs_the_body_of_its_table_handler() {
        let table = UopHot::handler_table();
        for (name, probe, parcels) in all() {
            let probe: Body = black_box(probe);
            let mut ran = 0;
            for simd in [false, true] {
                for seed in 0..12 {
                    let (mut a, mut b) = (build(&parcels, seed, simd), build(&parcels, seed, simd));
                    let (uops, _) = lower(&mut a, simd);
                    let u = uops.unwrap_or_else(|| panic!("{name}: not lowered"))[0];
                    let mut ca = UopCtx::new(&mut a as *mut _, a.steps, 100);
                    let mut cb = UopCtx::new(&mut b as *mut _, b.steps, 100);
                    let (ra, rb) =
                        unsafe { (probe(&mut ca, &u), table[usize::from(u.h)](&mut cb, &u)) };
                    assert_eq!(ra, rb, "{name} simd {simd} seed {seed}: exit");
                    assert_eq!(exact_diff(&a, &b), None, "{name} simd {simd} seed {seed}");
                    assert_eq!((ca.done, ca.next_pc), (cb.done, cb.next_pc), "{name}");
                    ran += 1;
                }
            }
            assert_eq!(ran, 24, "{name}");
        }
    }

    #[test]
    fn the_probes_cover_loads_stores_and_every_register_class() {
        let names: Vec<&str> = all().iter().map(|p| p.0).collect();
        for want in [
            "load_post_r",
            "store_post_r",
            "load_preimm_s",
            "load_preimm_other",
            "dual",
        ] {
            assert!(names.contains(&want), "{want}");
        }
        assert_eq!(names.len(), 13);
    }
}
