//! Offscreen capture through [`AppleOffscreenRenderer`] — `WuiViewRenderer.swift`'s
//! `renderViewToRGBA`, ported onto [`ViewRenderer`].
//!
//! The rendered leaf mounts under a fresh [`ContentLedger`], joins its
//! capture window and is laid out under the request's proposal; the
//! capture then awaits the content the leaf's views owe, lays the leaf out
//! again at the size that content gives it and captures it once into
//! premultiplied RGBA8 — through the central native+GPU capture under
//! `gpu_surface` (with pure-native `native-test` trees on macOS and Mac
//! Catalyst routed through `crate::capture`'s in-process `Core Animation`
//! render server), or through `crate::capture` directly in a native-only
//! build.

use cocoa_ui::PlatformView;
use cocoa_ui::bitmap::set_window_color_scheme;
#[cfg(all(
    feature = "gpu_surface",
    feature = "native-test",
    any(target_os = "macos", catalyst)
))]
use cocoa_ui::capture::collect_capturables;
use cocoa_ui::view::window as view_window;
use waterui_backend_core::{Environment, View};
use waterui_capture::ContentLedger;
use waterui_capture::view_renderer::{RenderError, RenderRequest, RenderResult, ViewRenderer};
use waterui_core::env::With;
use waterui_core::layout::{ProposalSize, Size};

#[cfg(any(
    not(feature = "gpu_surface"),
    all(feature = "native-test", any(target_os = "macos", catalyst))
))]
use crate::capture;
use crate::contract::NativeLeaf;

/// The Apple backend's [`ViewRenderer`]: renders a typed view under a
/// fresh [`ContentLedger`] into premultiplied RGBA8 on the main thread.
#[derive(Debug, Clone)]
pub struct AppleOffscreenRenderer {
    mtm: cocoa_ui::MainThreadMarker,
}

impl AppleOffscreenRenderer {
    /// Creates an offscreen renderer on the main thread `mtm` proves.
    #[must_use]
    pub const fn new(mtm: cocoa_ui::MainThreadMarker) -> Self {
        Self { mtm }
    }
}

impl ViewRenderer for AppleOffscreenRenderer {
    #[expect(
        clippy::future_not_send,
        reason = "view rendering runs on the main thread; the future borrows the non-Send native leaf and `MainThreadMarker` across the capture await"
    )]
    async fn render_to_rgba<V: View>(
        &self,
        view: V,
        env: &Environment,
        request: RenderRequest,
    ) -> Result<RenderResult, RenderError> {
        let ledger = ContentLedger::new();
        let leaf = crate::dispatch::render(With::new(view, ledger.clone()), env, self.mtm);
        let (pixels, width, height) =
            capture_leaf_to_rgba(&leaf, env, &ledger, request, self.mtm).await?;
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
        | CaptureFailure::GpuRuntimeUnavailable
        | CaptureFailure::GpuUnrecoverable(_)
        | CaptureFailure::CompositionFailed { .. }
        | CaptureFailure::TargetAllocation { .. } => RenderError::Gpu(Box::new(failure)),
        #[cfg(target_os = "ios")]
        CaptureFailure::NoWindowScene => RenderError::Capture(Box::new(failure)),
    }
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
    apply_env_color_scheme(view, env);
    lay_out(view, request.size());
    // The capture window cannot present, so the surfaces inside never
    // attach: the drive keeps their frames — and the content their scenes
    // settle — moving from here until the terminal capture. Declared
    // after `host` so the scopes it opened end before the leaf unhosts.
    #[cfg(feature = "gpu_surface")]
    let drive = crate::capture_image::CaptureDrive::begin(view, env, mtm);
    crate::content::await_content(ledger, view, request.content_deadline()).await?;
    lay_out(view, request.size());

    #[cfg(all(
        feature = "gpu_surface",
        feature = "native-test",
        any(target_os = "macos", catalyst)
    ))]
    let captured = {
        let mut has_capturable = false;
        collect_capturables(view, &mut |_| has_capturable = true);
        if has_capturable {
            drive
                .capture_rgba()
                .await
                .map(|captured| (captured.pixels, captured.width, captured.height))
                .map_err(render_error)
        } else {
            drop(drive);
            capture::capture_presented(view)
                .await
                .map(RenderResult::into_parts)
                .map_err(|error| RenderError::Capture(Box::new(error)))
        }
    };
    #[cfg(all(
        feature = "gpu_surface",
        not(all(feature = "native-test", any(target_os = "macos", catalyst)))
    ))]
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
    let captured = capture::capture_presented(view)
        .await
        .map(RenderResult::into_parts)
        .map_err(|error| RenderError::Capture(Box::new(error)));
    drop(host);
    captured
}

/// Applies the `ColorScheme` installed in `env`, if any, onto `view`'s capture
/// window so native controls and glass materials resolve against light or dark
/// mode before layout and rasterization.
fn apply_env_color_scheme(view: &PlatformView, env: &Environment) {
    use cocoa_ui::ColorScheme as CocoaColorScheme;
    use waterui::theme::{ColorScheme, installed_color_scheme};
    use waterui_core::Signal as _;

    let Some(scheme_signal) = installed_color_scheme(env) else {
        return;
    };
    let scheme = match scheme_signal.snapshot() {
        ColorScheme::Light => CocoaColorScheme::Light,
        ColorScheme::Dark => CocoaColorScheme::Dark,
    };
    if let Some(window) = view_window(view) {
        set_window_color_scheme(&window, scheme);
    }
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
    use waterui_capture::view_renderer::RenderError;

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
    use waterui_capture::view_renderer::RenderError;

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
            let content = window
                .contentView()
                .expect("a window built by `capture_window` has a content view");
            cocoa_ui::view::add_subview(&content, view);
            Ok(Self { _window: window })
        }
    }
}

/// The capture window the leaf joins before it is measured.
#[cfg(all(target_os = "ios", not(feature = "gpu_surface")))]
mod host {
    #[cfg(all(feature = "native-test", catalyst))]
    use cocoa_ui::bitmap::scene_less_capture_window;
    use cocoa_ui::bitmap::{
        CaptureWindow, any_window_scene, close_capture_window, make_offscreen_window,
        show_capture_window,
    };
    #[cfg(all(feature = "native-test", catalyst))]
    use cocoa_ui::uikit::catalyst::BackgroundApplication;
    use cocoa_ui::{MainThreadMarker, PlatformView, Retained, Size};
    use waterui_capture::view_renderer::RenderError;

    #[cfg(not(all(feature = "native-test", catalyst)))]
    use crate::capture::CaptureError;

    /// `UIKit`: an offscreen `UIWindow` in a connected scene hosts the
    /// leaf while `crate::capture` renders its layer — the `UIKit` half of
    /// `captureViewToRGBA`. Hidden again on drop.
    pub struct CaptureHost(Retained<CaptureWindow>);

    impl CaptureHost {
        /// Attaches `view` to an offscreen window sized `proposed`.
        #[cfg_attr(
            all(feature = "native-test", catalyst),
            expect(
                clippy::unnecessary_wraps,
                reason = "non-Catalyst UIKit can fail with NoWindowScene; native-test Catalyst always attaches"
            )
        )]
        pub fn attach(
            view: &PlatformView,
            proposed: Size,
            mtm: MainThreadMarker,
        ) -> Result<Self, RenderError> {
            #[cfg(all(feature = "native-test", catalyst))]
            let window = any_window_scene().map_or_else(
                || {
                    let application = BackgroundApplication::bring_up(mtm);
                    scene_less_capture_window(mtm, &application, proposed)
                },
                |scene| make_offscreen_window(mtm, &scene, proposed),
            );
            #[cfg(not(all(feature = "native-test", catalyst)))]
            let window = {
                let scene = any_window_scene()
                    .ok_or_else(|| RenderError::Capture(Box::new(CaptureError::NoWindowScene)))?;
                make_offscreen_window(mtm, &scene, proposed)
            };
            show_capture_window(&window, view, proposed);
            Ok(Self(window))
        }
    }

    impl Drop for CaptureHost {
        fn drop(&mut self) {
            close_capture_window(&self.0);
        }
    }
}
