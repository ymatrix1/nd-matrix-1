//! Handlers of the move range `0x300..0x400`.
//!
//! OWNER: W2-MOVE. Planned kinds: register moves, DM/PM loads and stores, immediate loads, dual
//! moves, DAG post-modify and circular buffers (`UF_CB`). SRAM-window accesses take a direct fast
//! path (`exec::mem`), every other address goes through `dm_read` / `dm_write`, peripheral first,
//! then re-checks the deadline. A store handler checks `uop_run.flags` once after the write and
//! returns `Exit::Smc` when the write hit a code page (design 7.4). An instruction that can fault
//! after mutating state is lowered as `Interp`, never as a fast uop (rule 3 of design 4.5).
//!
//! Skeleton: nothing registered, so a move uop (none is emitted yet) would exit `Slow`.
use super::super::types::Handler;

/// Fills the move range of `t`.
pub(crate) fn register(t: &mut [Handler]) {
    let _ = t;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::bail_handler;

    #[test]
    fn the_skeleton_registers_nothing() {
        let mut t = vec![bail_handler as Handler; 0x500];
        register(&mut t);
        assert!(t.iter().all(|h| *h as usize == bail_handler as usize));
    }
}
