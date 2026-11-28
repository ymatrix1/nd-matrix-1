//! Event split: how many steps a block may run before the interrupt controller must look at the
//! core again.
//!
//! OWNER: W2-DRIVE. `Core::step` begins with `if self.steps >= self.irq.due && self.sequencer_event()`
//! (read in `exec.rs`), so a block starting at `steps` may run `due - steps` steps without
//! changing what the interpreter would do. `poke_irq` sets `due` to 0, which makes every later
//! room 0 until the prologue ran. ASSUMED until `event_split_matches_interpreter` and
//! `poke_forces_checked_loop` (W2-DRIVE) pin it; the skeleton computes the room only.
use crate::core::Core;

/// Steps left before `min(limit, irq.due)`, clamped to `u32`.
#[inline]
pub(crate) fn room(core: &Core, limit: u64) -> u32 {
    let end = limit.min(core.irq.due);
    u32::try_from(end.saturating_sub(core.steps)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;

    #[test]
    fn room_is_the_distance_to_the_nearer_of_the_limit_and_the_deadline() {
        let mut c = core_with(&[0x0001]);
        c.steps = 10;
        c.irq.due = u64::MAX;
        assert_eq!(room(&c, 30), 20);
        c.irq.due = 14;
        assert_eq!(room(&c, 30), 4);
        c.irq.due = 10;
        assert_eq!(room(&c, 30), 0, "an event due now leaves no room");
        c.irq.due = 0;
        assert_eq!(room(&c, 30), 0, "poke_irq (due = 0) leaves no room");
        c.irq.due = u64::MAX;
        assert_eq!(room(&c, 5), 0, "a limit already passed");
        assert_eq!(room(&c, u64::MAX), u32::MAX, "clamped to u32");
    }
}
