//! `ViewRenderer` FFI bindings for capturing views to RGBA pixels.
//!
//! This module provides FFI functions for native backends to install their
//! view rendering implementation. The renderer captures view hierarchies
//! (native widgets + GPU surfaces) to RGBA pixel data.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::future::Future;

use waterui_core::AnyView;
use waterui_core::view_renderer::{CustomViewRenderer, RenderResult, RenderSize, ViewRenderer};

use crate::WuiEnv;
use crate::closure::ForeignCallbackContext;
use crate::components::layout::WuiSize;

/// Callback for returning rendered RGBA data to Rust.
#[repr(C)]
#[derive(Debug)]
pub struct ViewRenderCallback {
    /// Opaque data pointer passed to the callback.
    pub data: *mut (),
    /// One-shot callback function. Native may invoke it asynchronously, but
    /// must invoke it exactly once.
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
    context: *mut (),
    view: *mut (), // AnyView pointer (boxed)
    size: WuiSize, // Target size
    callback: ViewRenderCallback,
);

/// FFI-compatible `ViewRenderer` implementation.
struct FFIViewRenderer {
    context: ForeignCallbackContext,
    render_fn: ViewRenderFn,
}

impl CustomViewRenderer for FFIViewRenderer {
    fn render_to_rgba(
        &self,
        view: AnyView,
        size: RenderSize,
    ) -> impl Future<Output = RenderResult> {
        struct CallbackData {
            sender: async_channel::Sender<RenderResult>,
        }

        unsafe extern "C" fn render_trampoline(
            data: *mut (),
            rgba_ptr: *const u8,
            rgba_len: usize,
            width: u32,
            height: u32,
        ) {
            let CallbackData { sender } = *unsafe { Box::from_raw(data.cast::<CallbackData>()) };

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

            // Dropping the returned future is legal cancellation. Native still
            // invokes the callback once so this payload is released.
            let _ = sender.try_send(result);
        }

        let render_fn = self.render_fn;
        let view_ptr = Box::into_raw(Box::new(view));
        let view_ptr_void = view_ptr.cast::<()>();
        let wui_size = WuiSize {
            width: size.width,
            height: size.height,
        };

        // Use a oneshot channel pattern for callback handoff.
        let (tx, rx) = async_channel::bounded::<RenderResult>(1);

        // Create callback data that owns the sender.
        // The view pointer is consumed by native (waterui_view_body) and must not be dropped here.
        let callback_data = Box::new(CallbackData { sender: tx });
        let callback_data = Box::into_raw(callback_data).cast::<()>();

        let callback = ViewRenderCallback {
            data: callback_data,
            call: render_trampoline,
        };

        // Native owns the view and callback payload until it invokes the
        // callback exactly once. Rendering may complete asynchronously.
        unsafe {
            (render_fn)(self.context.data(), view_ptr_void, wui_size, callback);
        }

        async move {
            rx.recv()
                .await
                .expect("Native view renderer dropped its completion callback")
        }
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
/// - `context` remains valid until `drop_context` releases it
/// - `render_fn` is valid for `context`
/// - `drop_context` releases `context` exactly once
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_install_view_renderer(
    env: *mut WuiEnv,
    context: *mut (),
    render_fn: ViewRenderFn,
    drop_context: unsafe extern "C" fn(*mut ()),
) {
    let env = unsafe { crate::borrow_ffi_mut(env) };

    let renderer = ViewRenderer::new(FFIViewRenderer {
        context: unsafe { ForeignCallbackContext::new(context, drop_context) },
        render_fn,
    });
    env.insert(renderer);
}
