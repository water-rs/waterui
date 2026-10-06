//! The offscreen subtree capture `preview` and `view_renderer` share on
//! macOS: a window that is created but never ordered in hosts the view,
//! mounted GPU surfaces and filters are driven through their first
//! presented frames — the occlusion gates the runtime's own path waits
//! behind never open offscreen — and `AppKit`'s `cacheDisplayInRect:`
//! rasterizes the whole subtree, private-`AppKit` and `IOSurface` layer
//! contents alike, into one premultiplied RGBA8 bitmap.

#[cfg(target_os = "macos")]
use waterui_core::view_renderer::RenderResult;

/// Why the platform capture produced no bitmap.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// `Core Graphics` refused the destination bitmap context.
    #[error("could not create a {width}x{height} RGBA bitmap context")]
    BitmapContext {
        /// Bitmap width in pixels.
        width: usize,
        /// Bitmap height in pixels.
        height: usize,
    },
    /// `AppKit` gave the view no bitmap to cache its display into.
    #[cfg(target_os = "macos")]
    #[error("the view provided no bitmap representation to cache its display into")]
    NoBitmapRep,
    /// The cached display bitmap could not be read as an image.
    #[cfg(target_os = "macos")]
    #[error("the cached display bitmap has no CGImage")]
    NoImage,
    /// A zero-sized view has no pixels — an error, not a 1x1 fallback.
    #[cfg(target_os = "macos")]
    #[error("the capture view is zero-sized ({width}x{height} pt)")]
    ZeroSize {
        /// The view's logical width.
        width: f64,
        /// The view's logical height.
        height: f64,
    },
    /// No connected window scene can host the offscreen capture window.
    #[cfg(target_os = "ios")]
    #[error("no UIWindowScene is connected to host the capture window")]
    NoWindowScene,
}

/// Attaches `view` inside a window that never orders in and captures the
/// presented result — the `view_renderer` service's entry: the view arrives
/// already measured and laid out under its proposal.
///
/// # Errors
///
/// Returns [`CaptureError`] when `AppKit` cannot produce the bitmap.
#[cfg(target_os = "macos")]
#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread; the Retained AppKit objects it holds across the wait are not Send"
)]
pub async fn capture_view(
    view: &cocoa_ui::PlatformView,
    actual: cocoa_ui::Size,
    mtm: cocoa_ui::MainThreadMarker,
) -> Result<RenderResult, CaptureError> {
    let window = cocoa_ui::bitmap::make_offscreen_window(mtm, actual);
    let content_view = window.contentView().expect("offscreen window content");
    cocoa_ui::view::add_subview(&content_view, view);
    capture_presented(view).await
}

/// The shared tail of every `AppKit` capture: `host` is already inside its
/// never-ordered window and laid out under its proposal. GPU surfaces and
/// filters are driven to a presented frame, then the whole subtree is
/// rasterized into premultiplied RGBA8.
///
/// # Errors
///
/// Returns [`CaptureError`] when `AppKit` cannot produce the bitmap.
#[cfg(target_os = "macos")]
#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread; the Retained AppKit objects it holds across the wait are not Send"
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "`ceil` yields an integral value bounded by the view size, which fits usize/u32"
)]
#[expect(
    clippy::cast_sign_loss,
    reason = "the zero-size guard above keeps bounds and scale positive"
)]
pub async fn capture_presented(
    host: &cocoa_ui::PlatformView,
) -> Result<RenderResult, CaptureError> {
    // The dynamic-range tag rides an ancestor: `require_inherited` resolves
    // it above the tagged view, and a never-ordered window has no screen
    // for the untagged fallback to read. `Standard` is the honest answer —
    // the capture has no display to be extended-range on.
    let window = cocoa_ui::view::window(host).expect("capture host is inside a window");
    let ancestor = window
        .contentView()
        .expect("capture window has a content view");
    cocoa_ui::dynamic_range::apply_to_view(
        cocoa_ui::dynamic_range::DynamicRange::Standard,
        &ancestor,
    );

    let scale = cocoa_ui::view::backing_scale_factor(host);
    cocoa_ui::view::prepare_for_capture(host);
    cocoa_ui::bitmap::force_text_fields_display(host);

    #[cfg(feature = "gpu_surface")]
    {
        crate::components::gpu_surface::present_first_frames(host, scale);
        #[cfg(feature = "applied_filter")]
        crate::components::filtered::present_first_frames(host, scale);
        crate::components::gpu_surface::wait_for_first_frames(host).await;
        #[cfg(feature = "applied_filter")]
        crate::components::filtered::wait_for_capture_frames(host).await;
    }

    let bounds = cocoa_ui::view::bounds(host);
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return Err(CaptureError::ZeroSize {
            width: bounds.size.width,
            height: bounds.size.height,
        });
    }
    let pixel_width = (bounds.size.width * scale).ceil() as usize;
    let pixel_height = (bounds.size.height * scale).ceil() as usize;
    let mut pixels = alloc::vec![0u8; pixel_width * pixel_height * 4];
    let context = cocoa_ui::bitmap::bitmap_context(&mut pixels, pixel_width, pixel_height).ok_or(
        CaptureError::BitmapContext {
            width: pixel_width,
            height: pixel_height,
        },
    )?;
    cocoa_ui::bitmap::scale_to_pixels(&context, scale);

    let rep =
        cocoa_ui::view::bitmap_rep_for_caching_display(host).ok_or(CaptureError::NoBitmapRep)?;
    cocoa_ui::view::cache_display(host, &rep);
    let image = rep.CGImage().ok_or(CaptureError::NoImage)?;
    cocoa_ui::bitmap::draw_image(&context, &image, bounds);
    Ok(RenderResult {
        rgba_data: pixels,
        width: pixel_width as u32,
        height: pixel_height as u32,
    })
}
