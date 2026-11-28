//! Handlers of the move range `0x300..0x400`.
//!
//! OWNER: W2-MOVE. Kinds: the 26 transfer handlers (`xfer`: load or store, five address modes,
//! three register classes), the immediate loads and stores and the index modifies (`imm`), the
//! register moves and swaps (`regs`) and the dual data moves (`dual`); `layout` fixes the kind
//! numbers and the operand fields the lowering (`lower/moves`) writes and the handlers read.
//! SRAM-window accesses take a direct fast path (`exec::mem`), every other address goes through
//! `dm_read` / `dm_write` with the core materialized at the uop boundary, and the handler then
//! re-checks the deadline and the self-modified-code flag (`Mem::finish`: `Jump` to the next
//! instruction when `irq.due` moved, `Smc` after a store into translated code). An instruction
//! that can fault after mutating state, a conditional move, a long-word access, a move with a
//! compute and any access to a system register stay `Interp` uops (rule 3 of design 4.5).
//!
//! # Exactness
//!
//! Every handler is the interpreter arm for the unconditional form, with the same order of
//! reads, writes and updates (see `xfer.rs` for the five steps). Two inputs the block cannot know
//! are read at run time: `MODE1` (bit-reversed output of `I0`/`I8`, circular-buffer enable,
//! broadcast loads of `I1`/`I9`) and the value of `L`/`B`. The PEy mode is baked into the block
//! (`UF_SIMD`); a block never runs in the other mode (the `Simd` row of the entry guard).
//!
//! A uop that must run `finish_step` afterwards (`UF_FIN`: a delay slot or a loop end) cannot
//! return `Smc` or `Jump`, because the dispatch loop would skip `finish_step`; it returns `Slow`
//! before it mutates anything when an access is not plain (not SRAM, watched, or on a page of
//! translated code), and the interpreter runs the instruction.
//!
//! # Static instruction counts (design 9.1: 18 cycles per Tier 1 move against about 83 in v1)
//!
//! Measured on the compiler assembly of the test build (rustc 1.100.0, `x86_64-pc-windows-msvc`,
//! opt-level 3, one codegen unit) with `tools/uop_insn_count.py`; `probe.rs` holds the
//! `uop_probe_m_*` symbols. Each number is the static count of x86-64 instructions of the whole
//! handler body, including the rare paths laid out inline (circular wrap, bit reversal, the SIMD
//! twin, the broadcast, the slow `dm_*` call, the `UF_FIN` preflight). It is a count of
//! instructions, never of time, and bounds the straight path from above: reading the asm of the
//! scalar post-modify load, the hot path (SRAM window, no wrap, no bit reversal, no twin) runs
//! about 70 of its 203 (an estimate from the listing, not a measurement). `--build` of the tool
//! does not find the `.s` file under `target/debug/build/nord-sharc/<hash>/out/` and its parser
//! does not read the v0 symbol mangling of rustc 1.100, so the run fed that file to the tool's
//! parser through a wrapper that only teaches it the v0 tail.
//!
//! | probe (handler) | insns | calls | branches |
//! |---|---|---|---|
//! | `load_post_r` (R = DM(Ii, Mj)) | 203 | 6 | 39 |
//! | `load_postimm_r` (R = DM(Ii, imm)) | 200 | 6 | 39 |
//! | `load_preimm_r` / `load_preimm_s` (pre-modify) | 158 | 6 | 29 |
//! | `load_preimm_other` (not R/S, no twin) | 97 | 4 | 11 |
//! | `load_direct_r` (R = DM(addr)) | 84 | 3 | 16 |
//! | `load_imm_r` (R = imm) | 13 | 0 | 1 |
//! | `store_post_r` (DM(Ii, Mj) = R) | 232 | 5 | 46 |
//! | `store_postimm_r` (DM(Ii, imm) = R) | 229 | 5 | 46 |
//! | `store_preimm_r` (pre-modify) | 187 | 5 | 37 |
//! | `store_direct_r` (DM(addr) = R) | 155 | 5 | 32 |
//! | `imm_store` (DM(..) = imm) | 222 | 5 | 44 |
//! | `dual` (two accesses, two updates) | 742 | 21 | 129 |
//!
//! The decode-cache invalidation of a store into cached code is outlined (`mem::note_code_write`,
//! cold): inlined it added about 65 instructions to every store handler (`store_post_r` 296).
mod dual;
mod imm;
pub(crate) mod layout;
mod regs;
mod xfer;

#[cfg(test)]
mod probe;
#[cfg(test)]
mod support;
#[cfg(test)]
mod tests_misc;
#[cfg(test)]
mod tests_xfer;

use super::super::types::Handler;

/// Fills the move range of `t`.
pub(crate) fn register(t: &mut [Handler]) {
    xfer::register(t);
    imm::register(t);
    regs::register(t);
    dual::register(t);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::{bail_handler, KindFamily};

    #[test]
    fn registration_fills_exactly_the_move_kinds_and_nothing_else() {
        let marker: Handler = bail_handler;
        let mut t = vec![marker; 0x500];
        register(&mut t);
        let range = KindFamily::Move.range();
        let changed: Vec<usize> = (0..t.len())
            .filter(|&i| t[i] as usize != marker as usize)
            .collect();
        assert_eq!(
            changed.len(),
            26 + 3 + 1 + 3 + 2 + 1,
            "every reachable kind of layout.rs"
        );
        assert!(changed.iter().all(|i| range.contains(i)));
        // the engine's own table gets them too, next to the other families
        let table = crate::core::uop::hot::UopHot::handler_table();
        let bail = bail_handler as Handler as usize;
        for i in &changed {
            assert_ne!(table[*i] as usize, bail, "slot {i:#x}");
        }
    }
}
