//! An [`Editor`] implementation for egui.

use crate::egui::Vec2;
use crate::egui::ViewportCommand;
use crate::EguiState;
use baseview::gl::GlConfig;
use baseview::PhySize;
use baseview::{Size, WindowHandle, WindowOpenOptions, WindowScalePolicy};
use crossbeam::atomic::AtomicCell;
use egui_baseview::egui::Context;
use egui_baseview::EguiWindow;
use nih_plug::prelude::{Editor, GuiContext, ParamSetter, ParentWindowHandle};
use parking_lot::RwLock;
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// An [`Editor`] implementation that calls an egui draw loop.
pub(crate) struct EguiEditor<T> {
    pub(crate) egui_state: Arc<EguiState>,
    /// The plugin's state. This is kept in between editor openenings.
    pub(crate) user_state: Arc<RwLock<T>>,

    /// The user's build function. Applied once at the start of the application.
    pub(crate) build: Arc<dyn Fn(&Context, &mut T) + 'static + Send + Sync>,
    /// The user's update function.
    pub(crate) update: Arc<dyn Fn(&Context, &ParamSetter, &mut T) + 'static + Send + Sync>,

    /// The scaling factor reported by the host, if any. On macOS this will never be set and we
    /// should use the system scaling factor instead.
    pub(crate) scaling_factor: AtomicCell<Option<f32>>,
}

/// This version of `baseview` uses a different version of `raw_window_handle than NIH-plug, so we
/// need to adapt it ourselves.
struct ParentWindowHandleAdapter(nih_plug::editor::ParentWindowHandle);

unsafe impl HasRawWindowHandle for ParentWindowHandleAdapter {
    fn raw_window_handle(&self) -> RawWindowHandle {
        match self.0 {
            ParentWindowHandle::X11Window(window) => {
                let mut handle = raw_window_handle::XcbWindowHandle::empty();
                handle.window = window;
                RawWindowHandle::Xcb(handle)
            }
            ParentWindowHandle::AppKitNsView(ns_view) => {
                let mut handle = raw_window_handle::AppKitWindowHandle::empty();
                handle.ns_view = ns_view;
                RawWindowHandle::AppKit(handle)
            }
            ParentWindowHandle::Win32Hwnd(hwnd) => {
                let mut handle = raw_window_handle::Win32WindowHandle::empty();
                handle.hwnd = hwnd;
                RawWindowHandle::Win32(handle)
            }
        }
    }
}

impl<T> Editor for EguiEditor<T>
where
    T: 'static + Send + Sync,
{
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn std::any::Any + Send> {
        let build = self.build.clone();
        let update = self.update.clone();
        let state = self.user_state.clone();
        let egui_state = self.egui_state.clone();

        let (unscaled_width, unscaled_height) = self.egui_state.size();
        let scaling_factor = self.scaling_factor.load();
        let window = EguiWindow::open_parented(
            &ParentWindowHandleAdapter(parent),
            WindowOpenOptions {
                title: String::from("egui window"),
                // Baseview should be doing the DPI scaling for us
                size: Size::new(unscaled_width as f64, unscaled_height as f64),
                // NOTE: For some reason passing 1.0 here causes the UI to be scaled on macOS but
                //       not the mouse events.
                scale: scaling_factor
                    .map(|factor| WindowScalePolicy::ScaleFactor(factor as f64))
                    .unwrap_or(WindowScalePolicy::SystemScaleFactor),

                #[cfg(feature = "opengl")]
                gl_config: Some(GlConfig {
                    version: (3, 2),
                    red_bits: 8,
                    blue_bits: 8,
                    green_bits: 8,
                    alpha_bits: 8,
                    depth_bits: 24,
                    stencil_bits: 8,
                    samples: None,
                    srgb: true,
                    double_buffer: true,
                    vsync: true,
                    ..Default::default()
                }),
            },
            Default::default(),
            state,
            move |egui_ctx, _queue, state| build(egui_ctx, &mut state.write()),
            move |egui_ctx, queue, state| {
                let setter = ParamSetter::new(context.as_ref());

                // If the window was requested to resize
                // NOTE (nd matrix 1): upstream took the request out of `requested_size` before
                // calling `request_resize()`, which reads `Editor::size()`; that then returned the
                // OLD size, so the host was asked to keep its size while egui drew at the new one.
                // The request now stays set during the call.
                if let Some(new_size) = egui_state.requested_size.load() {
                    let ok = context.request_resize();
                    egui_state.requested_size.store(None);
                    if ok {
                        // Resize the content of egui window
                        let ppp = egui_ctx.native_pixels_per_point().unwrap_or(1.0);
                        queue.resize(PhySize::new(
                            (new_size.0 as f32 * ppp).round() as u32,
                            (new_size.1 as f32 * ppp).round() as u32,
                        ));
                        egui_ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(
                            new_size.0 as f32,
                            new_size.1 as f32,
                        )));

                        // Update the state
                        egui_state.size.store(new_size);
                    }
                }

                // For now, just always redraw. Most plugin GUIs have meters, and those almost always
                // need a redraw. Later we can try to be a bit more sophisticated about this. Without
                // this we would also have a blank GUI when it gets first opened because most DAWs open
                // their GUI while the window is still unmapped.
                egui_ctx.request_repaint();
                (update)(egui_ctx, &setter, &mut state.write());
            },
        );

        self.egui_state.open.store(true, Ordering::Release);
        Box::new(EguiEditorHandle {
            egui_state: self.egui_state.clone(),
            window,
        })
    }

    /// Size of the editor window
    fn size(&self) -> (u32, u32) {
        let new_size = self.egui_state.requested_size.load();
        // This method will be used to ask the host for new size.
        // If the editor is currently being resized and new size hasn't been consumed and set yet, return new requested size.
        if let Some(new_size) = new_size {
            new_size
        } else {
            self.egui_state.size()
        }
    }

    fn set_scale_factor(&self, factor: f32) -> bool {
        // If the editor is currently open then the host must not change the current HiDPI scale as
        // we don't have a way to handle that. Ableton Live does this.
        if self.egui_state.is_open() {
            return false;
        }

        self.scaling_factor.store(Some(factor));
        true
    }

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {
        // As mentioned above, for now we'll always force a redraw to allow meter widgets to work
        // correctly. In the future we can use an `Arc<AtomicBool>` and only force a redraw when
        // that boolean is set.
    }

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {}

    fn param_values_changed(&self) {
        // Same
    }
}

/// The window handle used for [`EguiEditor`].
struct EguiEditorHandle {
    egui_state: Arc<EguiState>,
    window: WindowHandle,
}

/// The window handle enum stored within 'WindowHandle' contains raw pointers. Is there a way around
/// having this requirement?
unsafe impl Send for EguiEditorHandle {}

impl Drop for EguiEditorHandle {
    fn drop(&mut self) {
        self.egui_state.open.store(false, Ordering::Release);

        // NOTE (nd matrix 1): `WindowHandle::close()` only POSTS a close message, so on Windows the
        // window (and its `wnd_proc`, which lives in this DLL) was destroyed later by the host's
        // message pump. Hosts such as VSTHost unload the plugin right after `removed()`, so the next
        // message called a window proc in unmapped code (0xC0000005 on closing the VST3, issue #629).
        // Destroy it synchronously here: `removed()` runs on the thread that created the window.
        #[cfg(windows)]
        if self.window.is_open() {
            if let RawWindowHandle::Win32(handle) = self.window.raw_window_handle() {
                if win32::destroy_window_now(handle.hwnd) {
                    return;
                }
            }
        }

        // XXX: This should automatically happen when the handle gets dropped, but apparently not
        self.window.close();
    }
}

#[cfg(windows)]
mod win32 {
    use std::ffi::c_void;

    #[link(name = "user32")]
    extern "system" {
        fn DestroyWindow(hwnd: *mut c_void) -> i32;
        #[cfg(test)]
        pub fn IsWindow(hwnd: *mut c_void) -> i32;
    }

    /// Destroy `hwnd` before returning (baseview's own close is asynchronous). Returns whether the
    /// window was destroyed; `WM_NCDESTROY` frees the baseview window state inside this call.
    pub fn destroy_window_now(hwnd: *mut c_void) -> bool {
        if hwnd.is_null() {
            return false;
        }
        // SAFETY: `hwnd` came from a live baseview window created on this thread.
        unsafe { DestroyWindow(hwnd) != 0 }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[link(name = "user32")]
        extern "system" {
            fn CreateWindowExW(
                ex: u32,
                class: *const u16,
                name: *const u16,
                style: u32,
                x: i32,
                y: i32,
                w: i32,
                h: i32,
                parent: *mut c_void,
                menu: *mut c_void,
                inst: *mut c_void,
                param: *mut c_void,
            ) -> *mut c_void;
        }

        fn wide(s: &str) -> Vec<u16> {
            s.encode_utf16().chain(std::iter::once(0)).collect()
        }

        /// The window is gone when the call returns (a posted close message would leave it alive).
        #[test]
        fn destroy_window_now_destroys_synchronously() {
            let class = wide("STATIC");
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    class.as_ptr(),
                    0,
                    0,
                    0,
                    10,
                    10,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            assert!(!hwnd.is_null(), "test window not created");
            assert_eq!(unsafe { IsWindow(hwnd) }, 1);
            assert!(destroy_window_now(hwnd));
            assert_eq!(unsafe { IsWindow(hwnd) }, 0, "window outlived the call");
        }

        #[test]
        fn destroy_window_now_rejects_null() {
            assert!(!destroy_window_now(std::ptr::null_mut()));
        }
    }
}
