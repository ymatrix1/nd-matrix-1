//! Decides how the editor window is closed on Windows (portable so it is unit-tested on every OS).
//!
//! NOTE (nd matrix 1): baseview's `WindowHandle::close()` only POSTS a close message. Its window
//! procedure lives in the plugin DLL, and hosts unload the DLL right after the editor is removed,
//! so any message still queued for (or later sent to) the window runs unmapped code (0xC0000005,
//! issue #629). Every path therefore ends with the window destroyed or its procedure replaced by
//! user32's `DefWindowProcW`, never with a posted message.

/// What the editor handle must still do after `DestroyWindow` was tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseStep {
    /// The window is gone (or never existed): nothing else may run.
    Done,
    /// The window outlived `DestroyWindow` (wrong thread, re-entrancy): point its window procedure at
    /// `DefWindowProcW` so a late message can never reach this DLL.
    Neutralise,
}

/// `destroyed`: `DestroyWindow` returned success. `still_a_window`: `IsWindow(hwnd)` afterwards.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn plan(destroyed: bool, still_a_window: bool) -> CloseStep {
    if destroyed || !still_a_window {
        CloseStep::Done
    } else {
        CloseStep::Neutralise
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destroyed_window_needs_nothing_more() {
        assert_eq!(plan(true, false), CloseStep::Done);
        assert_eq!(plan(true, true), CloseStep::Done);
    }

    #[test]
    fn dead_handle_needs_nothing_more() {
        assert_eq!(plan(false, false), CloseStep::Done);
    }

    /// Destroy failed (for example the host dropped the handle on another thread) and the window is
    /// alive: its procedure must be replaced instead of falling back to baseview's posted close.
    #[test]
    fn surviving_window_is_neutralised_never_posted() {
        assert_eq!(plan(false, true), CloseStep::Neutralise);
    }
}
