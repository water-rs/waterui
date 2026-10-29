//! The `view_renderer` service: the environment's `ViewRenderer` —
//! `WuiViewRenderer.swift`'s `renderViewToRGBA`, ported.
//!
//! The rendered leaf mounts under a fresh [`ContentLedger`], joins its
//! capture window and is laid out under the request's proposal; the
//! capture then awaits the content the leaf's views owe, lays the leaf out
//! again at the size that content gives it and captures it once into
//! premultiplied RGBA8 — through the central native+GPU capture under
//! `gpu_surface`, the plain offscreen bitmap path in a native-only build —
//! the `CustomViewRenderer` contract.

use waterui_backend_core::Environment;
use waterui_core::env::With;
use waterui_core::layout::{ProposalSize, Size};
use waterui_core::view_renderer::{
    CustomViewRenderer, RenderError, RenderRequest, RenderResult, ViewRenderer,
};
use waterui_core::{AnyView, ContentLedger};

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
        request: RenderRequest,
    ) -> Result<RenderResult, RenderError> {
        let ledger = ContentLedger::new();
        let leaf = self
            .renderer
            .render(AnyView::new(With::new(view, ledger.clone())));
        let (pixels, width, height) = capture_leaf_to_rgba(
            &leaf,
            self.renderer.context().env(),
            &ledger,
            request,
            self.mtm,
        )
        .await?;
        Ok(RenderResult::new(pixels, width, height))
    }
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

/// Captures `leaf` into premultiplied RGBA8 under `request`'s proposal once
/// the content `ledger` records has arrived — `captureViewToRGBA`.
///
/// The leaf joins its capture window before it is measured: native controls
/// build their parts — and report their real size — only inside a window.
/// It is laid out under the proposal, the capture awaits the owed content,
/// and the leaf is measured and laid out again, since content that arrived
/// — a decoded photo — changes the size it fits.
#[expect(
    clippy::future_not_send,
    reason = "the capture borrows the main-thread leaf across its await"
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
    ledger: &ContentLedger,
    request: RenderRequest,
    mtm: cocoa_ui::MainThreadMarker,
) -> Result<(alloc::vec::Vec<u8>, u32, u32), RenderError> {
    let view = leaf.view();
    let proposed = cocoa_ui::Size::new(
        f64::from(request.size().width),
        f64::from(request.size().height),
    );
    cocoa_ui::view::set_frame(
        view,
        cocoa_ui::Rect::new(0.0, 0.0, proposed.width, proposed.height),
    );
    let host = host::CaptureHost::attach(view, proposed, mtm)?;
    lay_out(view, request.size());
    // The capture window cannot present, so the surfaces inside never
    // attach: the drive keeps their frames — and the content their scenes
    // settle — moving from here until the terminal capture. Declared
    // after `host` so the scopes it opened end before the leaf unhosts.
    #[cfg(feature = "gpu_surface")]
    let drive = crate::capture_image::CaptureDrive::begin(view, env, mtm);
    crate::content::await_content(ledger, view, request.content_deadline()).await?;
    lay_out(view, request.size());

    #[cfg(feature = "gpu_surface")]
    let captured = {
        // The central capture owns the native+GPU composite and its
        // lifecycle — a first-frame wait here could deadlock under
        // link-only delivery on the screenless capture window.
        drive
            .capture_rgba()
            .await
            .map(|captured| (captured.pixels, captured.width, captured.height))
            .map_err(render_error)
    };
    #[cfg(not(feature = "gpu_surface"))]
    let captured = crate::capture::capture_presented(view)
        .await
        .map(|result| (result.rgba_data, result.width, result.height))
        .map_err(|error| RenderError::Capture(Box::new(error)));
    drop(host);
    captured
}

/// Measures `view` under the proposal `size` and lays it out at the size it
/// fits — the full proposal on an axis it does not size.
fn lay_out(view: &cocoa_ui::PlatformView, size: Size) {
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
        crate::proposal::Placement::platform_hosted(ProposalSize {
            width: Some(size.width),
            height: Some(size.height),
        }),
    );
    cocoa_ui::view::set_frame(
        view,
        cocoa_ui::Rect::new(0.0, 0.0, actual.width, actual.height),
    );
    cocoa_ui::view::layout_immediately(view);
}

/// The capture window the leaf joins before it is measured.
#[cfg(feature = "gpu_surface")]
mod host {
    use waterui_core::view_renderer::RenderError;

    /// The central capture's re-host: the leaf moves into a never-ordered
    /// (`AppKit`) or offscreen (`UIKit`) window and returns to its parent
    /// on drop.
    pub struct CaptureHost {
        /// The re-host, `None` when the leaf already had a window.
        _host: Option<crate::capture_image::WindowHost>,
    }

    impl CaptureHost {
        /// Attaches `view`, already framed at the proposal, to a capture
        /// window sized from it.
        pub fn attach(
            view: &cocoa_ui::PlatformView,
            _proposed: cocoa_ui::Size,
            mtm: cocoa_ui::MainThreadMarker,
        ) -> Result<Self, RenderError> {
            crate::capture_image::WindowHost::attach_if_windowless(
                &cocoa_ui::view::retain_base(view),
                mtm,
            )
            .map(|host| Self { _host: host })
            .map_err(super::render_error)
        }
    }
}

/// The capture window the leaf joins before it is measured.
#[cfg(all(target_os = "macos", not(feature = "gpu_surface")))]
mod host {
    use waterui_core::view_renderer::RenderError;

    /// `AppKit`: a window that never orders in hosts the leaf for
    /// `crate::capture`'s bitmap readback — the `AppKit` half of
    /// `captureViewToRGBA`.
    pub struct CaptureHost {
        /// The window, alive until the capture is read.
        _window: cocoa_ui::Retained<cocoa_ui::objc2_app_kit::NSWindow>,
    }

    impl CaptureHost {
        /// Attaches `view` to a never-ordered window sized `proposed`.
        #[expect(
            clippy::unnecessary_wraps,
            reason = "only UIKit's window-scene lookup can fail; AppKit's offscreen window always attaches"
        )]
        pub fn attach(
            view: &cocoa_ui::PlatformView,
            proposed: cocoa_ui::Size,
            mtm: cocoa_ui::MainThreadMarker,
        ) -> Result<Self, RenderError> {
            let window = cocoa_ui::bitmap::capture_window(mtm, proposed);
            let content = window.contentView().expect("offscreen window content");
            cocoa_ui::view::add_subview(&content, view);
            Ok(Self { _window: window })
        }
    }
}

/// The capture window the leaf joins before it is measured.
#[cfg(all(target_os = "ios", not(feature = "gpu_surface")))]
mod host {
    use waterui_core::view_renderer::RenderError;

    use crate::capture::CaptureError;

    /// `UIKit`: an offscreen `UIWindow` in a connected scene hosts the
    /// leaf while `crate::capture` renders its layer — the `UIKit` half of
    /// `captureViewToRGBA`. Hidden again on drop.
    pub struct CaptureHost(cocoa_ui::Retained<cocoa_ui::bitmap::CaptureWindow>);

    impl CaptureHost {
        /// Attaches `view` to an offscreen window sized `proposed`.
        pub fn attach(
            view: &cocoa_ui::PlatformView,
            proposed: cocoa_ui::Size,
            mtm: cocoa_ui::MainThreadMarker,
        ) -> Result<Self, RenderError> {
            let scene = cocoa_ui::bitmap::any_window_scene()
                .ok_or_else(|| RenderError::Capture(Box::new(CaptureError::NoWindowScene)))?;
            let window = cocoa_ui::bitmap::make_offscreen_window(mtm, &scene, proposed);
            cocoa_ui::bitmap::show_capture_window(&window, view, proposed);
            Ok(Self(window))
        }
    }

    impl Drop for CaptureHost {
        fn drop(&mut self) {
            cocoa_ui::bitmap::close_capture_window(&self.0);
        }
    }
}
