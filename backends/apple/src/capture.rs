//! The offscreen subtree capture `preview` and `view_renderer` share when
//! `gpu_surface` is off, and the pure-native readback `preview` and
//! `view_renderer` use under `preview` or `native-test` on macOS and Mac
//! Catalyst.
//!
//! In production builds (without `preview` or `native-test`), macOS hosts the
//! view in a never-ordered window and rasterizes it through `AppKit`'s
//! `cacheDisplayInRect:`, while `UIKit` renders its layer into a bitmap inside
//! an offscreen `UIWindow`. Under `preview` or `native-test` on macOS and Mac
//! Catalyst (`render_server_capture`), `capture_presented` routes through
//! `cocoa_ui`'s opt-in in-process `Core Animation` render server so glass and
//! backdrop layers render without ordering any window on screen. `Metal` layer
//! contents never reach that bitmap, so GPU-bearing trees stay on
//! `capture_image`'s `ViewCapture`.

#[cfg(all(target_os = "macos", render_server_capture))]
use cocoa_ui::bitmap::force_text_fields_display;
#[cfg(render_server_capture)]
use cocoa_ui::bitmap::{
    HdrRaster, render_layer_via_render_server, render_layer_via_render_server_hdr,
};
#[cfg(any(target_os = "macos", render_server_capture))]
use cocoa_ui::dynamic_range::{DynamicRange, apply_to_view};
use cocoa_ui::view::backing_scale_factor;
#[cfg(render_server_capture)]
use cocoa_ui::view::bounds as view_bounds;
#[cfg(any(target_os = "ios", all(target_os = "macos", render_server_capture)))]
use cocoa_ui::view::layer;
#[cfg(render_server_capture)]
use cocoa_ui::view::prepare_for_capture;
#[cfg(all(target_os = "macos", render_server_capture))]
use cocoa_ui::view::window as view_window;
use waterui_capture::view_renderer::RenderResult;

/// Why the platform capture produced no bitmap.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// `Core Graphics` refused the destination bitmap context.
    #[cfg(not(render_server_capture))]
    #[error("could not create a {width}x{height} RGBA bitmap context")]
    BitmapContext {
        /// Bitmap width in pixels.
        width: usize,
        /// Bitmap height in pixels.
        height: usize,
    },
    /// The capture host is in no window, or its window has no content view,
    /// so there is nothing to tag with a dynamic range.
    #[cfg(target_os = "macos")]
    #[error("the capture host is not inside a window with a content view")]
    NoWindow,
    /// `AppKit` gave the view no bitmap to cache its display into.
    #[cfg(all(target_os = "macos", not(render_server_capture)))]
    #[error("the view provided no bitmap representation to cache its display into")]
    NoBitmapRep,
    /// The cached display bitmap could not be read as an image.
    #[cfg(target_os = "macos")]
    #[error("the cached display bitmap has no CGImage")]
    NoImage,
    /// A zero-sized view has no pixels — an error, not a 1x1 fallback.
    #[error("the capture view is zero-sized ({width}x{height} pt)")]
    ZeroSize {
        /// The view's logical width.
        width: f64,
        /// The view's logical height.
        height: f64,
    },
    /// No connected window scene can host the offscreen capture window.
    #[cfg(all(
        target_os = "ios",
        not(feature = "gpu_surface"),
        not(all(render_server_capture, catalyst))
    ))]
    #[error("no UIWindowScene is connected to host the capture window")]
    NoWindowScene,
    /// The opt-in in-process `Core Animation` render server refused to render
    /// the offscreen layer subtree.
    #[cfg(render_server_capture)]
    #[error("in-process Core Animation render server failed to render {width}x{height} raster")]
    RenderServer {
        /// Raster width in pixels.
        width: usize,
        /// Raster height in pixels.
        height: usize,
    },
}

/// The shared tail of every `AppKit` capture: `host` is already inside its
/// never-ordered window and laid out under its proposal. Production builds
/// rasterize the subtree through `AppKit`'s `cacheDisplayInRect:`; under
/// `feature = "native-test"` the subtree renders through `cocoa_ui`'s
/// test-only in-process `Core Animation` render server so glass and backdrop
/// layers composite without ordering a window.
///
/// # Errors
///
/// Returns [`CaptureError`] when `AppKit` or the render server cannot produce
/// the bitmap.
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
#[expect(
    clippy::unused_async,
    reason = "the signature stays `async` so `async` callers need no feature gating"
)]
pub async fn capture_presented(
    host: &cocoa_ui::PlatformView,
) -> Result<RenderResult, CaptureError> {
    // The dynamic-range tag rides an ancestor: `require_inherited` resolves
    // it above the tagged view, and a never-ordered window has no screen
    // for the untagged fallback to read. `Standard` is the honest answer —
    // the capture has no display to be extended-range on.
    let ancestor = cocoa_ui::view::window(host)
        .and_then(|window| window.contentView())
        .ok_or(CaptureError::NoWindow)?;
    apply_to_view(DynamicRange::Standard, &ancestor);

    let scale = backing_scale_factor(host);
    cocoa_ui::view::prepare_for_capture(host);
    cocoa_ui::bitmap::force_text_fields_display(host);

    let bounds = cocoa_ui::view::bounds(host);
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return Err(CaptureError::ZeroSize {
            width: bounds.size.width,
            height: bounds.size.height,
        });
    }
    let pixel_width = (bounds.size.width * scale).ceil() as usize;
    let pixel_height = (bounds.size.height * scale).ceil() as usize;
    #[cfg(render_server_capture)]
    {
        let target_layer = layer(host).ok_or(CaptureError::NoImage)?;
        let pixels =
            render_layer_via_render_server(&target_layer, scale, pixel_width, pixel_height).ok_or(
                CaptureError::RenderServer {
                    width: pixel_width,
                    height: pixel_height,
                },
            )?;
        Ok(RenderResult::new(
            pixels,
            pixel_width as u32,
            pixel_height as u32,
        ))
    }
    #[cfg(not(render_server_capture))]
    {
        let mut pixels = alloc::vec![0u8; pixel_width * pixel_height * 4];
        let context = cocoa_ui::bitmap::bitmap_context(&mut pixels, pixel_width, pixel_height)
            .ok_or(CaptureError::BitmapContext {
                width: pixel_width,
                height: pixel_height,
            })?;
        cocoa_ui::bitmap::scale_to_pixels(&context, scale);

        let rep = cocoa_ui::view::bitmap_rep_for_caching_display(host)
            .ok_or(CaptureError::NoBitmapRep)?;
        cocoa_ui::view::cache_display(host, &rep);
        let image = rep.CGImage().ok_or(CaptureError::NoImage)?;
        cocoa_ui::bitmap::draw_image(&context, &image, bounds);
        Ok(RenderResult::new(
            pixels,
            pixel_width as u32,
            pixel_height as u32,
        ))
    }
}

/// Captures `host` on macOS into an unclamped extended linear Display P3 `f32`
/// RGBA raster via the opt-in in-process `Core Animation` render server
/// (`kCVPixelFormatType_64RGBAHalf` + `kCGColorSpaceExtendedLinearDisplayP3`).
///
/// # Errors
///
/// Returns [`CaptureError`] when the view has no window, is zero-sized, or the
/// render server fails.
#[cfg(all(target_os = "macos", render_server_capture))]
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
    reason = "the zero-size guard keeps bounds and scale positive"
)]
#[expect(
    clippy::unused_async,
    reason = "the signature stays `async` to mirror `capture_presented`"
)]
pub async fn capture_presented_hdr(
    host: &cocoa_ui::PlatformView,
) -> Result<HdrRaster, CaptureError> {
    let ancestor = view_window(host)
        .and_then(|window| window.contentView())
        .ok_or(CaptureError::NoWindow)?;
    apply_to_view(DynamicRange::High, &ancestor);
    apply_to_view(DynamicRange::High, host);

    let scale = backing_scale_factor(host);
    prepare_for_capture(host);
    force_text_fields_display(host);

    let bounds = view_bounds(host);
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return Err(CaptureError::ZeroSize {
            width: bounds.size.width,
            height: bounds.size.height,
        });
    }
    let pixel_width = (bounds.size.width * scale).ceil() as usize;
    let pixel_height = (bounds.size.height * scale).ceil() as usize;
    let target_layer = layer(host).ok_or(CaptureError::NoImage)?;
    let pixels =
        render_layer_via_render_server_hdr(&target_layer, scale, pixel_width, pixel_height).ok_or(
            CaptureError::RenderServer {
                width: pixel_width,
                height: pixel_height,
            },
        )?;
    Ok(HdrRaster {
        pixels,
        width: pixel_width as u32,
        height: pixel_height as u32,
    })
}

/// The `UIKit` tail of every capture: `host` is already inside its
/// offscreen `UIWindow` and laid out under its proposal; its layer renders
/// the whole subtree into premultiplied RGBA8 at the view's display scale.
///
/// In production builds (without `preview` or `native-test`), `renderInContext`
/// replays the layer model through `Core Graphics`, not the `Core Animation`
/// compositor, so backdrop filters are not evaluated: a `UIVisualEffectView`
/// material draws its tint without blurring what lies behind it
/// (water-rs/waterui#2660). Under `preview` or `native-test` on Mac Catalyst
/// (`render_server_capture`), the subtree renders through `cocoa_ui`'s opt-in
/// in-process `Core Animation` render server, evaluating backdrop and glass
/// materials without ordering any window.
///
/// # Errors
///
/// Returns [`CaptureError`] when the view is zero-sized, `Core Graphics`
/// refuses the bitmap context, or the opt-in render server fails.
#[cfg(target_os = "ios")]
#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread; the UIKit view it borrows is not Sync"
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "`ceil` yields an integral value bounded by the view size, which fits usize/u32"
)]
#[expect(
    clippy::cast_sign_loss,
    reason = "the zero-size guard above keeps bounds and scale positive"
)]
#[expect(
    clippy::unused_async,
    reason = "the signature stays `async` so `async` callers need no platform gating"
)]
pub async fn capture_presented(
    host: &cocoa_ui::PlatformView,
) -> Result<RenderResult, CaptureError> {
    let bounds = cocoa_ui::view::bounds(host);
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return Err(CaptureError::ZeroSize {
            width: bounds.size.width,
            height: bounds.size.height,
        });
    }
    let scale = backing_scale_factor(host).max(1.0);
    let pixel_width = (bounds.size.width * scale).ceil() as usize;
    let pixel_height = (bounds.size.height * scale).ceil() as usize;
    #[cfg(render_server_capture)]
    {
        prepare_for_capture(host);
        let target_layer = layer(host).expect("every `UIView` owns a backing layer");
        let pixels =
            render_layer_via_render_server(&target_layer, scale, pixel_width, pixel_height).ok_or(
                CaptureError::RenderServer {
                    width: pixel_width,
                    height: pixel_height,
                },
            )?;
        Ok(RenderResult::new(
            pixels,
            pixel_width as u32,
            pixel_height as u32,
        ))
    }
    #[cfg(not(render_server_capture))]
    {
        let mut pixels = alloc::vec![0u8; pixel_width * pixel_height * 4];
        let context = cocoa_ui::bitmap::bitmap_context(&mut pixels, pixel_width, pixel_height)
            .ok_or(CaptureError::BitmapContext {
                width: pixel_width,
                height: pixel_height,
            })?;
        cocoa_ui::bitmap::scale_to_pixels(&context, scale);
        cocoa_ui::bitmap::begin_layer_flip(&context, bounds.size.height);
        let target_layer = layer(host).expect("every `UIView` owns a backing layer");
        cocoa_ui::bitmap::with_uikit_context(&context, || {
            cocoa_ui::bitmap::render_layer(&target_layer, &context);
        });
        cocoa_ui::bitmap::end_layer_flip(&context);
        drop(context);
        Ok(RenderResult::new(
            pixels,
            pixel_width as u32,
            pixel_height as u32,
        ))
    }
}

/// Captures `host` on Mac Catalyst (`UIKit`) into an unclamped extended linear
/// Display P3 `f32` RGBA raster via the opt-in in-process `Core Animation`
/// render server (`kCVPixelFormatType_64RGBAHalf` +
/// `kCGColorSpaceExtendedLinearDisplayP3`).
///
/// # Errors
///
/// Returns [`CaptureError`] when the view is zero-sized or the render server
/// fails.
#[cfg(all(target_os = "ios", render_server_capture))]
#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread; the UIKit view it borrows is not Sync"
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "`ceil` yields an integral value bounded by the view size, which fits usize/u32"
)]
#[expect(
    clippy::cast_sign_loss,
    reason = "the zero-size guard keeps bounds and scale positive"
)]
#[expect(
    clippy::unused_async,
    reason = "the signature stays `async` to mirror `capture_presented`"
)]
pub async fn capture_presented_hdr(
    host: &cocoa_ui::PlatformView,
) -> Result<HdrRaster, CaptureError> {
    let bounds = view_bounds(host);
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return Err(CaptureError::ZeroSize {
            width: bounds.size.width,
            height: bounds.size.height,
        });
    }
    apply_to_view(DynamicRange::High, host);
    let scale = backing_scale_factor(host).max(1.0);
    let pixel_width = (bounds.size.width * scale).ceil() as usize;
    let pixel_height = (bounds.size.height * scale).ceil() as usize;
    prepare_for_capture(host);
    let target_layer = layer(host).expect("every `UIView` owns a backing layer");
    let pixels =
        render_layer_via_render_server_hdr(&target_layer, scale, pixel_width, pixel_height).ok_or(
            CaptureError::RenderServer {
                width: pixel_width,
                height: pixel_height,
            },
        )?;
    Ok(HdrRaster {
        pixels,
        width: pixel_width as u32,
        height: pixel_height as u32,
    })
}
