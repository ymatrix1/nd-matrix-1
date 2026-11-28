# Vendored nih_plug_egui

Copy of `nih_plug_egui` from https://github.com/robbert-vdh/nih-plug (rev de42101, ISC, see
`LICENSE`), used through `[patch]` in the workspace `Cargo.toml`. Local changes:

1. `editor.rs`: `queue.resize` took the LOGICAL size as a physical size, so after a plugin-driven
   resize egui rendered into a surface `ppp` times too small (content bottom-left, black rest) on
   HiDPI hosts. It now multiplies by the native pixels-per-point.
2. `editor.rs`: the resize request stays set while `request_resize()` runs; upstream cleared it first, so the host was asked for the OLD size.
3. `EguiState::set_requested_size` is `pub`.

4. `editor.rs`: on Windows, `EguiEditorHandle::drop` destroys the window synchronously (`DestroyWindow`) instead of only posting baseview's async close message. Hosts unload the DLL right after `removed()`, and the still-queued message called a window proc in unmapped code (access violation on closing the VST3, issue #629).
5. `lib.rs`: `EguiState::requested_size` getter (read-only), so tests can see a resize the editor asked for.
6. `editor.rs` + `close_plan.rs`: if the synchronous `DestroyWindow` of item 4 fails (host drops the handle on another thread), the window procedure is replaced by user32's `DefWindowProcW` instead of falling back to baseview's posted close (issue #629, refs the unloaded-DLL window procedure).
