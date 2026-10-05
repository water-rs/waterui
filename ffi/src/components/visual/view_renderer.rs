//! `ViewRenderer` FFI bindings for capturing views to RGBA pixels.
//!
//! This module provides FFI functions for native backends to install their
//! view rendering implementation. The renderer captures view hierarchies
//! (native widgets + GPU surfaces) to RGBA pixel data.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::future::Future;

use waterui_core::AnyView;
use waterui_core::view_renderer::{
    CustomViewRenderer, RenderError, RenderResult, RenderSize, ViewRenderer,
};

use crate::WuiEnv;
use crate::closure::ForeignCallbackContext;
use crate::components::layout::WuiSize;

/// One-shot completion handed to [`ViewRenderFn`].
///
/// Native completes a render by invoking exactly one of `call` or `fail`,
/// exactly once. It may do so asynchronously. Either function releases `data`,
/// so neither may be invoked after the first.
#[repr(C)]
#[derive(Debug)]
pub struct ViewRenderCallback {
    /// Opaque data pointer passed to `call` or `fail`.
    pub data: *mut (),
    /// Completes the render with pixels.
    /// - `data`: The opaque data pointer
    /// - `rgba_ptr`: Pointer to RGBA pixel data (4 bytes per pixel)
    /// - `rgba_len`: Length of the RGBA data in bytes
    /// - `width`: Rendered width in pixels
    /// - `height`: Rendered height in pixels
    ///
    /// The pixel buffer is only read during the call; native keeps ownership.
    pub call: unsafe extern "C" fn(
        data: *mut (),
        rgba_ptr: *const u8,
        rgba_len: usize,
        width: u32,
        height: u32,
    ),
    /// Completes the render with a failure instead of pixels.
    /// - `data`: The opaque data pointer
    /// - `message_ptr`: Pointer to a UTF-8 message describing the failure
    /// - `message_len`: Length of the message in bytes
    ///
    /// The message is only read during the call; native keeps ownership. It
    /// must be valid UTF-8: a message that is not valid UTF-8 aborts the
    /// process.
    pub fail: unsafe extern "C" fn(data: *mut (), message_ptr: *const u8, message_len: usize),
}

/// Type alias for the native view render function.
///
/// Native implements this function to render a view to RGBA pixels:
/// 1. Create an offscreen rendering context at the given size
/// 2. Render the `AnyView` hierarchy (native widgets + GPU surfaces)
/// 3. Capture the final composited result to RGBA pixels
/// 4. Invoke the callback's `call` with the pixel data, or its `fail` with
///    the reason when the capture cannot produce the view's pixels
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

/// The completion a [`ViewRenderCallback`] owns: the sending half of the
/// render's one-shot channel.
struct CallbackData {
    sender: async_channel::Sender<Result<RenderResult, RenderError>>,
}

impl CallbackData {
    /// Takes back the boxed completion `data` was registered with and
    /// delivers `result` through it.
    ///
    /// # Safety
    ///
    /// `data` is the `CallbackData` pointer a [`ViewRenderCallback`] was
    /// built with, and this is the first and only completion of it.
    unsafe fn complete(data: *mut (), result: Result<RenderResult, RenderError>) {
        // SAFETY: the caller contract makes `data` the boxed `CallbackData`
        // this callback was registered with, completed once.
        let Self { sender } = *unsafe { Box::from_raw(data.cast::<Self>()) };
        // Dropping the returned future is legal cancellation. Native still
        // completes the callback once so this payload is released.
        let _ = sender.try_send(result);
    }
}

/// Copies `len` bytes native lent at `ptr` for the duration of a callback.
///
/// # Safety
///
/// When `len` is non-zero, `ptr` points to `len` initialized bytes valid for
/// the duration of the call.
unsafe fn copy_borrowed_bytes(ptr: *const u8, len: usize) -> Vec<u8> {
    if len == 0 {
        Vec::new()
    } else {
        // SAFETY: the caller contract makes `ptr` valid for `len` bytes; the
        // copy is taken before returning, so the borrow does not escape.
        unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec()
    }
}

/// [`ViewRenderCallback::call`]: completes the render with pixels.
unsafe extern "C" fn render_completed(
    data: *mut (),
    rgba_ptr: *const u8,
    rgba_len: usize,
    width: u32,
    height: u32,
) {
    // SAFETY: native passes `rgba_len` initialized bytes at `rgba_ptr`, valid
    // for this call.
    let rgba_data = unsafe { copy_borrowed_bytes(rgba_ptr, rgba_len) };
    let result = RenderResult {
        rgba_data,
        width,
        height,
    };
    // SAFETY: `data` is this callback's `CallbackData`, completed once.
    unsafe { CallbackData::complete(data, Ok(result)) };
}

/// [`ViewRenderCallback::fail`]: completes the render with the host's
/// failure message.
unsafe extern "C" fn render_failed(data: *mut (), message_ptr: *const u8, message_len: usize) {
    // SAFETY: native passes `message_len` initialized bytes at `message_ptr`,
    // valid for this call.
    let message = unsafe { copy_borrowed_bytes(message_ptr, message_len) };
    let message = String::from_utf8(message)
        .expect("native view renderer failure message must be valid UTF-8");
    // SAFETY: `data` is this callback's `CallbackData`, completed once.
    unsafe { CallbackData::complete(data, Err(RenderError::Host(message))) };
}

impl CustomViewRenderer for FFIViewRenderer {
    fn render_to_rgba(
        &self,
        view: AnyView,
        size: RenderSize,
    ) -> impl Future<Output = Result<RenderResult, RenderError>> {
        let render_fn = self.render_fn;
        let view_ptr = Box::into_raw(Box::new(view));
        let view_ptr_void = view_ptr.cast::<()>();
        let wui_size = WuiSize {
            width: size.width,
            height: size.height,
        };

        // Use a oneshot channel pattern for callback handoff.
        let (tx, rx) = async_channel::bounded::<Result<RenderResult, RenderError>>(1);

        // Create callback data that owns the sender.
        // The view pointer is consumed by native (waterui_view_body) and must not be dropped here.
        let callback_data = Box::new(CallbackData { sender: tx });
        let callback_data = Box::into_raw(callback_data).cast::<()>();

        let callback = ViewRenderCallback {
            data: callback_data,
            call: render_completed,
            fail: render_failed,
        };

        // Native owns the view and callback payload until it completes the
        // callback exactly once. Rendering may complete asynchronously.
        // SAFETY: `render_fn` and the context are one registration, kept alive by
        // `self`; the view pointer and callback are handed to backend ownership.
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
    // SAFETY: the caller contract requires `env` to be a valid handle, alive and not
    // otherwise borrowed for this call; the exclusive borrow ends here.
    let env = unsafe { crate::borrow_ffi_mut(env) };

    let renderer = ViewRenderer::new(FFIViewRenderer {
        // SAFETY: the caller contract requires `context` and `drop_context` to be one
        // registration from the backend.
        context: unsafe { ForeignCallbackContext::new(context, drop_context) },
        render_fn,
    });
    env.insert(renderer);
}

#[cfg(test)]
mod tests {
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};

    use super::*;

    const FAILURE: &str = "capture window unavailable";

    unsafe extern "C" fn drop_context(_: *mut ()) {}

    /// Takes the view native owns and completes the render synchronously.
    unsafe extern "C" fn completing_render_fn(
        context: *mut (),
        view: *mut (),
        _size: WuiSize,
        callback: ViewRenderCallback,
    ) {
        // SAFETY: `view` is the boxed `AnyView` the renderer handed over.
        drop(unsafe { Box::from_raw(view.cast::<AnyView>()) });
        // SAFETY: each callback is completed once, through one of its functions.
        unsafe {
            if context.is_null() {
                (callback.fail)(callback.data, FAILURE.as_ptr(), FAILURE.len());
            } else {
                let pixels = [1_u8, 2, 3, 4];
                (callback.call)(callback.data, pixels.as_ptr(), pixels.len(), 1, 1);
            }
        }
    }

    /// Renders through `FFIViewRenderer`; a non-null `context` makes the
    /// native side succeed, a null one makes it fail.
    fn render(context: *mut ()) -> Result<RenderResult, RenderError> {
        let renderer = FFIViewRenderer {
            // SAFETY: `drop_context` ignores `context`, which is never dereferenced.
            context: unsafe { ForeignCallbackContext::new(context, drop_context) },
            render_fn: completing_render_fn,
        };
        let future = pin!(renderer.render_to_rgba(AnyView::new(()), RenderSize::new(1.0, 1.0)));
        match future.poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(result) => result,
            Poll::Pending => panic!("a synchronously completed render must be ready"),
        }
    }

    #[test]
    fn native_failure_completes_the_render_with_its_message() {
        let error = render(core::ptr::null_mut()).expect_err("native reported a failure");
        assert!(matches!(error, RenderError::Host(message) if message == FAILURE));
    }

    #[test]
    fn native_pixels_complete_the_render() {
        let mut marker = 0_u8;
        let result = render((&raw mut marker).cast()).expect("native delivered pixels");
        assert_eq!(
            (result.rgba_data, result.width, result.height),
            (alloc::vec![1, 2, 3, 4], 1, 1)
        );
    }
}
