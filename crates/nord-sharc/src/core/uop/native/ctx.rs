//! Layout of the structures generated code reads: `UopCtx` and the `Core` fields it touches.
//!
//! OWNER: W4-NCORE. Emitted code addresses `UopCtx` by byte offset, so the offsets are constants
//! derived with `offset_of!` and pinned by the test below; the `Core` field offsets are added with
//! the emitter (a layout test per field, design 10 W4-NCORE obligations).
use crate::core::uop::types::UopCtx;
use std::mem::offset_of;

/// Offset of `UopCtx::core`.
pub const CTX_CORE: usize = offset_of!(UopCtx, core);
/// Offset of `UopCtx::base_steps`.
pub const CTX_BASE_STEPS: usize = offset_of!(UopCtx, base_steps);
/// Offset of `UopCtx::done`.
pub const CTX_DONE: usize = offset_of!(UopCtx, done);
/// Offset of `UopCtx::room`.
pub const CTX_ROOM: usize = offset_of!(UopCtx, room);
/// Offset of `UopCtx::next_pc`.
pub const CTX_NEXT_PC: usize = offset_of!(UopCtx, next_pc);
/// Offset of `UopCtx::exit_idx`.
pub const CTX_EXIT_IDX: usize = offset_of!(UopCtx, exit_idx);

#[cfg(test)]
mod tests {
    use super::*;

    /// 64-bit targets (x86-64 and arm64, the only ones the native tier supports): an 8-byte
    /// pointer, then the counters, no padding.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn the_ctx_is_repr_c_with_the_documented_offsets() {
        assert_eq!(
            [
                CTX_CORE,
                CTX_BASE_STEPS,
                CTX_DONE,
                CTX_ROOM,
                CTX_NEXT_PC,
                CTX_EXIT_IDX
            ],
            [0, 8, 16, 20, 24, 28]
        );
        assert_eq!(std::mem::size_of::<UopCtx>(), 32);
    }
}
