//! Whole-pass AOT lowering v2 (bean `ns4-lle3`, design note `docs/research/sharc-lowering-v2.md`,
//! stages C and D). The 1:1 oracle stays the v1 path (`core/aot_*.rs`); this tree adds what v2
//! needs on top of it, in small files with their own tests. It is a child module of `core` (see
//! the `#[path]` in `core/mod.rs`) so it can use the core's private state.
pub(super) mod emit_ev;
pub(super) mod events;

#[cfg(test)]
mod tests {
    use crate::core::aot_lower::AotTier;
    use crate::core::support::core_with;
    use std::collections::BTreeMap;

    /// Three equal compute NOP-class instructions: one batch, so the v2 text wraps it.
    #[test]
    fn event_aware_text_differs_from_v1_only_by_the_event_checks() {
        let add = [0x013e, 0x0000, 0x1312];
        let p: Vec<u16> = add.iter().cycle().take(9).copied().collect();
        let c = core_with(&p);
        let mut ex = BTreeMap::new();
        let mut pc = c.pc;
        for k in 0..3 {
            ex.insert(pc, k == 0);
            pc += c.fetch(pc).insn.len as u32;
        }
        let v1 = c.emit_whole_pass_rust(&ex, &[], AotTier::Pe).unwrap();
        let v2 = c
            .emit_whole_pass_rust_ev(&ex, &[], AotTier::Pe, true)
            .unwrap();
        assert!(!v1.contains("aot_ev_span") && v1.contains("ev: false"));
        assert!(v2.contains("aot_ev_span") && v2.contains("ev: true"));
        assert!(v2.contains("aot_sig_ev"));
    }
}
