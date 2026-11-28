//! Entry into a native block from the driver.
//!
//! OWNER: W4-NCORE. `drive.rs` calls [`enter`] when `Block::native != NONE` (the one
//! `#[cfg(feature = "uop-native")]` call of design 10.4). The generated function has the shape
//! `fn(*mut UopCtx, *const Uop) -> u32` and materializes state at every exit like Tier 1.
//! Skeleton: no block carries a native entry, so this is never reached; if it were, it declines
//! with `Slow` and no step done, which sends the caller to the interpreter.
use crate::core::uop::hot::UopHot;
use crate::core::uop::types::Exit;
use crate::core::Core;

/// Runs block `id` natively with `room` steps before the event deadline. Returns the exit, the
/// index of the uop it happened at and the steps accounted, like `exec::run_block`.
pub(crate) fn enter(core: &mut Core, hot: &UopHot, id: u32, room: u32) -> (Exit, u32, u32) {
    let _ = (core, hot, id, room);
    (Exit::Slow, 0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;

    #[test]
    fn the_skeleton_declines_without_touching_the_core() {
        let mut c = core_with(&[0x0001]);
        let (pc, steps) = (c.pc, c.steps);
        assert_eq!(enter(&mut c, &UopHot::new(), 0, 8), (Exit::Slow, 0, 0));
        assert_eq!((c.pc, c.steps), (pc, steps));
    }
}
