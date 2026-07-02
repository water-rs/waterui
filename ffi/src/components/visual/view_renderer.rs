//! `ViewRenderer` FFI bindings for capturing views to RGBA pixels.
//!
//! This module provides FFI functions for native backends to install their
//! view rendering implementation. The renderer captures view hierarchies
//! (native widgets + GPU surfaces) to RGBA pixel data.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::future::{Future, ready};

use waterui_core::AnyView;
use waterui_core::view_renderer::{CustomViewRenderer, RenderResult, RenderSize, ViewRenderer};

use crate::WuiEnv;
use crate::components::layout::WuiSize;

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
    view: *mut (), // AnyView pointer (boxed)
    size: WuiSize, // Target size
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
    ) -> impl Future<Output = RenderResult> {
        let render_fn = self.render_fn;
        let view_ptr = Box::into_raw(Box::new(view));
        let view_ptr_void = view_ptr.cast::<()>();
        let wui_size = WuiSize {
            width: size.width,
            height: size.height,
        };

        // Use a oneshot channel pattern for callback handoff.
        let (tx, rx) = async_channel::bounded::<RenderResult>(1);

        struct CallbackData {
            sender: async_channel::Sender<RenderResult>,
        }

        // Create callback data that owns the sender.
        // The view pointer is consumed by native (waterui_view_body) and must not be dropped here.
        let callback_data = Box::new(CallbackData { sender: tx });
        let callback_data = Box::into_raw(callback_data).cast::<()>();

        unsafe extern "C" fn render_trampoline(
            data: *mut (),
            rgba_ptr: *const u8,
            rgba_len: usize,
            width: u32,
            height: u32,
        ) {
            let data = unsafe { &*data.cast::<CallbackData>() };

            // Copy the RGBA data (native owns the original buffer)
            let rgba_data = if rgba_len == 0 {
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
            let _ = data.sender.try_send(result);
        }

        let callback = ViewRenderCallback {
            data: callback_data,
            call: render_trampoline,
        };

        // Call native render function (must call callback synchronously)
        unsafe {
            (render_fn)(view_ptr_void, wui_size, callback);
        }

        let recv_result = rx.try_recv().unwrap_or_else(|err| {
            panic!(
                "Native view renderer must invoke callback synchronously before returning: \
                 {err}"
            );
        });

        // Free callback data after the callback completes.
        unsafe {
            drop(Box::from_raw(callback_data.cast::<CallbackData>()));
        }

        ready(recv_result)
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
    let env =
        unsafe { crate::expect_non_null_mut(env, "waterui_env_install_view_renderer", "env") };

    let renderer = ViewRenderer::new(FFIViewRenderer { render_fn });
    env.insert(renderer);
}
