//! `ViewRenderer` FFI bindings for capturing views to RGBA pixels.
//!
//! This module provides FFI functions for native backends to install their
//! view rendering implementation. The renderer captures view hierarchies
//! (native widgets + GPU surfaces) to RGBA pixel data.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;

use waterui_core::view_renderer::{CustomViewRenderer, RenderResult, RenderSize, ViewRenderer};
use waterui_core::AnyView;

use super::layout::WuiSize;
use crate::WuiEnv;

/// Callback for returning rendered RGBA data to Rust.
#[repr(C)]
pub struct ViewRenderCallback {
    /// Opaque data pointer passed to the callback.
    pub data: *mut (),
    /// Callback function.
    /// - `data`: The opaque data pointer
    /// - `rgba_ptr`: Pointer to RGBA pixel data (4 bytes per pixel)
    /// - `rgba_len`: Length of the RGBA data in bytes
    /// - `width`: Rendered width in pixels
    /// - `height`: Rendered height in pixels
    pub call: unsafe extern "C" fn(
        data: *mut (),
        rgba_ptr: *const u8,
        rgba_len: usize,
        width: u32,
        height: u32,
    ),
}

/// Type alias for the native view render function.
///
/// Native implements this function to render a view to RGBA pixels:
/// 1. Create an offscreen rendering context at the given size
/// 2. Render the `AnyView` hierarchy (native widgets + GPU surfaces)
/// 3. Capture the final composited result to RGBA pixels
/// 4. Call the callback with the pixel data
///
/// The view pointer is an `AnyView` that native should render.
pub type ViewRenderFn = unsafe extern "C" fn(
    view: *mut (),    // AnyView pointer (boxed)
    size: WuiSize,    // Target size
    callback: ViewRenderCallback,
);

/// FFI-compatible `ViewRenderer` implementation.
struct FFIViewRenderer {
    render_fn: ViewRenderFn,
}

impl CustomViewRenderer for FFIViewRenderer {
    fn render_to_rgba(
        &self,
        view: AnyView,
        size: RenderSize,
    ) -> Pin<Box<dyn Future<Output = RenderResult> + 'static>> {
        let render_fn = self.render_fn;
        let view_ptr = Box::into_raw(Box::new(view)).cast::<()>();
        let wui_size = WuiSize {
            width: size.width,
            height: size.height,
        };

        Box::pin(async move {
            // Use a oneshot channel pattern for async callback
            let (tx, rx) = async_channel::bounded::<RenderResult>(1);

            // Create callback that sends result through channel
            let tx_box: Box<async_channel::Sender<RenderResult>> = Box::new(tx);
            let callback_data = Box::into_raw(tx_box).cast::<()>();

            unsafe extern "C" fn render_trampoline(
                data: *mut (),
                rgba_ptr: *const u8,
                rgba_len: usize,
                width: u32,
                height: u32,
            ) {
                let tx = unsafe { Box::from_raw(data.cast::<async_channel::Sender<RenderResult>>()) };

                // Copy the RGBA data (native owns the original buffer)
                let rgba_data = if rgba_ptr.is_null() || rgba_len == 0 {
                    Vec::new()
                } else {
                    unsafe { core::slice::from_raw_parts(rgba_ptr, rgba_len) }.to_vec()
                };

                let result = RenderResult {
                    rgba_data,
                    width,
                    height,
                };

                // Send result (ignore error if receiver dropped)
                let _ = tx.try_send(result);
            }

            let callback = ViewRenderCallback {
                data: callback_data,
                call: render_trampoline,
            };

            // Call native render function
            unsafe {
                (render_fn)(view_ptr, wui_size, callback);
            }

            // Wait for result
            rx.recv().await.unwrap_or_else(|_| RenderResult {
                rgba_data: Vec::new(),
                width: 0,
                height: 0,
            })
        })
    }
}

/// Installs a `ViewRenderer` into the environment from a native function pointer.
///
/// Native backends call this during initialization to register their view
/// rendering implementation. The renderer is used to capture views as RGBA
/// pixels for the preview system.
///
/// # Safety
///
/// The caller must ensure that:
/// - `env` is a valid pointer to a `WuiEnv`
/// - `render_fn` is a valid function pointer to the native view renderer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_install_view_renderer(
    env: *mut WuiEnv,
    render_fn: ViewRenderFn,
) {
    if env.is_null() {
        return;
    }
    let env = unsafe { &mut *env };

    let renderer = ViewRenderer::new(FFIViewRenderer { render_fn });
    env.insert(renderer);
}
