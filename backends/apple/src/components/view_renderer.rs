//! The `view_renderer` service: the environment's `ViewRenderer` —
//! `WuiViewRenderer.swift`'s `renderViewToRGBA`, ported.
//!
//! The rendered leaf is laid out at its measured size, then captured once
//! into premultiplied RGBA8 — through the central native+GPU capture under
//! `gpu_surface`, the plain offscreen bitmap path in a native-only build —
//! the `CustomViewRenderer` contract.

use waterui_backend_core::Environment;
use waterui_core::AnyView;
use waterui_core::layout::ProposalSize;
use waterui_core::view_renderer::{
    CustomViewRenderer, RenderError, RenderResult, RenderSize, ViewRenderer,
};

use crate::contract::{NativeLeaf, Renderer};

/// The backend's `CustomViewRenderer`: a `'static` `Renderer` carrying the
/// dispatcher, an environment clone and the main-thread proof.
struct AppleViewRenderer {
    renderer: Renderer,
    mtm: cocoa_ui::MainThreadMarker,
}

impl core::fmt::Debug for AppleViewRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AppleViewRenderer").finish_non_exhaustive()
    }
}

impl CustomViewRenderer for AppleViewRenderer {
    #[expect(
        clippy::future_not_send,
        reason = "view rendering runs on the main thread; the future borrows the non-Send native leaf and `MainThreadMarker` across the capture await"
    )]
    async fn render_to_rgba(
        &self,
        view: AnyView,
        size: RenderSize,
    ) -> Result<RenderResult, RenderError> {
        let leaf = self.renderer.render(view);
        let (pixels, width, height) =
            capture_leaf_to_rgba(&leaf, self.renderer.context().env(), size, self.mtm).await?;
        Ok(RenderResult {
            rgba_data: pixels,
            width,
            height,
        })
    }
}

/// Why the platform capture produced no bitmap.
#[cfg(not(feature = "gpu_surface"))]
#[derive(Debug, thiserror::Error)]
enum CaptureError {
    /// Core Graphics refused the destination bitmap context.
    #[error("could not create a {width}x{height} RGBA bitmap context")]
    BitmapContext { width: usize, height: usize },
    /// `AppKit` gave the view no bitmap to cache its display into.
    #[cfg(target_os = "macos")]
    #[error("the view provided no bitmap representation to cache its display into")]
    NoBitmapRep,
    /// The cached display bitmap could not be read as an image.
    #[cfg(target_os = "macos")]
    #[error("the cached display bitmap has no CGImage")]
    NoImage,
    /// No connected window scene can host the offscreen capture window.
    #[cfg(target_os = "ios")]
    #[error("no UIWindowScene is connected to host the capture window")]
    NoWindowScene,
}

/// The `RenderError` a central-capture failure answers.
///
/// A failed GPU surface and a readback target the device refused are
/// failures of the GPU work behind the capture, so they report
/// [`RenderError::Gpu`]; a missing window scene is the platform withholding
/// a resource the capture needs, so it reports [`RenderError::Capture`].
#[cfg(feature = "gpu_surface")]
fn render_error(failure: crate::capture_image::CaptureFailure) -> RenderError {
    use crate::capture_image::CaptureFailure;
    match failure {
        CaptureFailure::GpuSurfaceFailed(_)
        | CaptureFailure::CompositionFailed { .. }
        | CaptureFailure::TargetAllocation { .. } => RenderError::Gpu(Box::new(failure)),
        #[cfg(target_os = "ios")]
        CaptureFailure::NoWindowScene => RenderError::Capture(Box::new(failure)),
    }
}

/// The environment's `ViewRenderer`, replacing
/// `waterui_env_install_view_renderer`.
///
/// Installs the native snapshot renderer into this environment.
///
/// # Panics
///
/// On a failure to confirm the main thread.
pub fn install_service(env: &mut Environment) {
    let mtm = cocoa_ui::MainThreadMarker::new().expect("main thread");
    let renderer =
        crate::contract::RenderContext::new(env, crate::dispatch::dispatcher(env), mtm).renderer();
    env.insert(ViewRenderer::new(AppleViewRenderer { renderer, mtm }));
}

/// Captures `leaf` into premultiplied RGBA8 under `size`'s proposal —
/// `captureViewToRGBA`.
#[expect(
    clippy::future_not_send,
    reason = "the capture borrows the main-thread leaf across its await"
)]
#[cfg_attr(
    not(feature = "gpu_surface"),
    expect(
        clippy::unused_async,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the synchronous fallback keeps the async contract; a clamped positive extent fits"
    )
)]
async fn capture_leaf_to_rgba(
    leaf: &NativeLeaf,
    #[cfg_attr(
        not(feature = "gpu_surface"),
        expect(
            unused_variables,
            reason = "only the central GPU capture reads the environment"
        )
    )]
    env: &Environment,
    size: RenderSize,
    mtm: cocoa_ui::MainThreadMarker,
) -> Result<(alloc::vec::Vec<u8>, u32, u32), RenderError> {
    let view = leaf.view();
    let proposed = cocoa_ui::Size::new(f64::from(size.width), f64::from(size.height));

    // Measure with the proposed size so stretchable views render correctly.
    cocoa_ui::view::set_frame(
        view,
        cocoa_ui::Rect::new(0.0, 0.0, proposed.width, proposed.height),
    );
    cocoa_ui::view::layout_immediately(view);
    let measured = cocoa_ui::view::fitting_size(view);
    let actual = cocoa_ui::Size::new(
        if measured.width.is_finite() && measured.width > 0.0 {
            measured.width
        } else {
            proposed.width
        },
        if measured.height.is_finite() && measured.height > 0.0 {
            measured.height
        } else {
            proposed.height
        },
    );

    // The render boundary's offer is the proposal the view was measured
    // under — delivered so a container subtree does not infer a different
    // proposal from the stamped frame.
    crate::proposal::deliver(
        view,
        ProposalSize {
            width: Some(size.width),
            height: Some(size.height),
        },
    );
    cocoa_ui::view::set_frame(
        view,
        cocoa_ui::Rect::new(0.0, 0.0, actual.width, actual.height),
    );
    cocoa_ui::view::layout_immediately(view);

    let scale = cocoa_ui::view::backing_scale_factor(view);
    #[cfg(feature = "gpu_surface")]
    {
        // The central capture owns the native+GPU composite and its
        // window/lifecycle — a first-frame wait here could deadlock under
        // link-only delivery on the screenless capture window.
        let captured = crate::capture_image::capture_rgba(view, env, scale, mtm)
            .await
            .map_err(render_error)?;
        Ok((captured.pixels, captured.width, captured.height))
    }
    #[cfg(not(feature = "gpu_surface"))]
    {
        let pixel_width = usize::try_from((actual.width * scale).ceil().max(1.0) as u64)
            .unwrap_or(usize::MAX)
            .min((usize::MAX - 3) / 4);
        let pixel_height = usize::try_from((actual.height * scale).ceil().max(1.0) as u64)
            .unwrap_or(usize::MAX)
            .min((usize::MAX - 3) / 4);
        capture::capture(view, actual, scale, pixel_width, pixel_height, mtm)
            .map_err(|error| RenderError::Capture(Box::new(error)))
    }
}

#[cfg(all(target_os = "macos", not(feature = "gpu_surface")))]
mod capture {
    use alloc::vec::Vec;

    use super::CaptureError;

    /// `AppKit`: offscreen borderless window, `cacheDisplay` bitmap — the
    /// `AppKit` half of `captureViewToRGBA`.
    #[allow(clippy::cast_possible_truncation)]
    pub fn capture(
        view: &cocoa_ui::PlatformView,
        actual: cocoa_ui::Size,
        scale: f64,
        pixel_width: usize,
        pixel_height: usize,
        mtm: cocoa_ui::MainThreadMarker,
    ) -> Result<(Vec<u8>, u32, u32), CaptureError> {
        let mut pixels = alloc::vec![0u8; pixel_width * pixel_height * 4];
        let context = cocoa_ui::bitmap::bitmap_context(&mut pixels, pixel_width, pixel_height)
            .ok_or(CaptureError::BitmapContext {
                width: pixel_width,
                height: pixel_height,
            })?;
        cocoa_ui::bitmap::scale_to_pixels(&context, scale);

        let window = cocoa_ui::bitmap::make_offscreen_window(mtm, actual);
        cocoa_ui::view::ensure_layer_backed(view);
        let content_view = window.contentView().expect("offscreen window content");
        cocoa_ui::view::add_subview(&content_view, view);
        cocoa_ui::bitmap::show_capture_window(&window);
        cocoa_ui::bitmap::force_text_fields_display(view);
        let drawn = cocoa_ui::view::bitmap_rep_for_caching_display(view)
            .ok_or(CaptureError::NoBitmapRep)
            .and_then(|rep| {
                cocoa_ui::view::cache_display(view, &rep);
                let image = rep.CGImage().ok_or(CaptureError::NoImage)?;
                cocoa_ui::bitmap::draw_image(
                    &context,
                    &image,
                    cocoa_ui::Rect::new(0.0, 0.0, actual.width, actual.height),
                );
                Ok(())
            });
        cocoa_ui::bitmap::close_capture_window(&window);
        drawn?;
        Ok((pixels, pixel_width as u32, pixel_height as u32))
    }
}

#[cfg(all(target_os = "ios", not(feature = "gpu_surface")))]
mod capture {
    use alloc::vec::Vec;

    use super::CaptureError;

    /// `UIKit`: offscreen `UIWindow`, `layer.render` into the context — the
    /// `UIKit` half of `captureViewToRGBA`.
    #[allow(clippy::cast_possible_truncation)]
    pub fn capture(
        view: &cocoa_ui::PlatformView,
        actual: cocoa_ui::Size,
        scale: f64,
        pixel_width: usize,
        pixel_height: usize,
        mtm: cocoa_ui::MainThreadMarker,
    ) -> Result<(Vec<u8>, u32, u32), CaptureError> {
        let mut pixels = alloc::vec![0u8; pixel_width * pixel_height * 4];
        let context = cocoa_ui::bitmap::bitmap_context(&mut pixels, pixel_width, pixel_height)
            .ok_or(CaptureError::BitmapContext {
                width: pixel_width,
                height: pixel_height,
            })?;
        cocoa_ui::bitmap::scale_to_pixels(&context, scale);

        let scene = cocoa_ui::bitmap::any_window_scene().ok_or(CaptureError::NoWindowScene)?;
        let window = cocoa_ui::bitmap::make_offscreen_window(mtm, &scene, actual);
        cocoa_ui::bitmap::show_capture_window(&window, view, actual);

        cocoa_ui::bitmap::begin_layer_flip(&context, actual.height);
        let layer = cocoa_ui::view::layer(view).expect("a UIKit view is always layer-backed");
        cocoa_ui::bitmap::with_uikit_context(&context, || {
            cocoa_ui::bitmap::render_layer(&layer, &context);
        });
        cocoa_ui::bitmap::end_layer_flip(&context);
        cocoa_ui::bitmap::close_capture_window(&window);
        Ok((pixels, pixel_width as u32, pixel_height as u32))
    }
}
