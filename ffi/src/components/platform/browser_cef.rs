//! C ABI for the optional CEF runtime used by native renderers.
//!
//! A CEF page is drawn and driven through the generic GPU surface: the view
//! [`gpu_view_with_input`] builds reports `wants_input_events`, so the pointer,
//! keyboard, scroll and composition events a backend already routes to
//! interactive GPU surfaces — `waterui_gpu_surface_send_input_event` — reach
//! Chromium with nothing CEF-specific crossing the ABI. What this module adds is
//! the part the generic surface cannot know: which semantic view the page came
//! from, kept alive for as long as the surface is.

use std::{any::Any, fmt};

#[cfg(any(feature = "webview-cef", feature = "cef-header"))]
use waterui_browser_cef::CefWebViewHandle;
use waterui_browser_cef::{
    CefPageHandle, CefRuntime, CefRuntimeConfiguration, gpu_view_with_input,
};
#[cfg(any(feature = "chromium", feature = "cef-header"))]
use waterui_chromium::{ChromiumView, PageMode};
use waterui_core::Environment;
use waterui_graphics::gpu_surface::GpuSurface;
#[cfg(any(feature = "webview-cef", feature = "cef-header"))]
use waterui_webview::WebView;

use crate::IntoFFI;
#[cfg(any(feature = "webview-cef", feature = "cef-header"))]
use crate::IntoRust;
#[cfg(any(feature = "webview-cef", feature = "cef-header"))]
use crate::WuiAnyView;
use crate::components::visual::gpu_surface::WuiGpuSurface;

/// Installs one process-owned CEF runtime and the selected public controllers.
pub(crate) fn configure_environment(env: &mut Environment) {
    let runtime = CefRuntime::initialize(CefRuntimeConfiguration::packaged());
    #[cfg(any(feature = "webview-cef", feature = "cef-header"))]
    env.insert(runtime.webview_controller());
    #[cfg(any(feature = "chromium", feature = "cef-header"))]
    env.insert(runtime.chromium_controller());
    env.insert(runtime.clone());
    // Chromium's browser-process loop is the engine crate's to drive: it has to
    // run whether or not a surface is being drawn, and it is paced by the
    // deadline CEF itself asks for.
    runtime.start_message_pump();
}

/// GPU surface plus the semantic view it draws.
#[repr(C)]
#[derive(Debug)]
pub struct WuiCefSurface {
    /// GPU presenter consumed by `WaterUI`'s native GPU surface host. It takes
    /// its own input: the host routes surface input events to it like to any
    /// other GPU view that asks for them.
    pub gpu_surface: WuiGpuSurface,
    /// Opaque state retained until [`waterui_cef_surface_drop`].
    pub state: *mut WuiCefSurfaceState,
}

/// Opaque CEF state the native backend owns for the surface's lifetime.
///
/// Keeps the semantic view the page was created from alive until
/// [`waterui_cef_surface_drop`]: that view owns the subscriptions behind
/// `can_go_back` / `can_go_forward`, the event signal and the URL binding, so
/// dropping it early would leave a live page whose reactive state had gone dead.
pub struct WuiCefSurfaceState {
    _source: Box<dyn Any>,
}

impl fmt::Debug for WuiCefSurfaceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The type-erased source view has no Debug representation, so only the
        // state's identity is reported.
        f.debug_struct("WuiCefSurfaceState").finish_non_exhaustive()
    }
}

fn surface(page: CefPageHandle, source: impl Any) -> WuiCefSurface {
    let gpu_surface = GpuSurface::new(gpu_view_with_input(page)).into_ffi();
    WuiCefSurface {
        gpu_surface,
        state: Box::into_raw(Box::new(WuiCefSurfaceState {
            _source: Box::new(source),
        })),
    }
}

#[cfg(any(feature = "chromium", feature = "cef-header"))]
impl IntoFFI for ChromiumView {
    type FFI = WuiCefSurface;

    fn into_ffi(self) -> Self::FFI {
        assert_eq!(
            self.page().mode(),
            PageMode::Visible,
            "headless Chromium pages cannot be rendered as ChromiumView"
        );
        let page = self
            .page()
            .handle()
            .downcast_ref::<CefPageHandle>()
            .unwrap_or_else(|| {
                panic!("Apple ChromiumView handle does not use the selected CEF runtime")
            })
            .clone();
        surface(page, self)
    }
}

#[cfg(any(feature = "chromium", feature = "cef-header"))]
ffi_view!(ChromiumView, WuiCefSurface, chromium, any());

/// Consumes a standard `WebView` whose selected engine is CEF.
///
/// # Safety
///
/// `view` must be a valid owning `WuiAnyView` containing `Native<WebView>`.
///
/// # Panics
///
/// Panics when the web view's engine handle was not produced by the CEF runtime
/// this build selected.
#[cfg(any(feature = "webview-cef", feature = "cef-header"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_force_as_cef_webview(view: *mut WuiAnyView) -> WuiCefSurface {
    // SAFETY: the caller contract makes `view` an owning `WuiAnyView` handle,
    // which is exactly what `IntoRust` reclaims here; it is consumed once.
    let any: waterui::AnyView = unsafe { IntoRust::into_rust(view) };
    // SAFETY: the caller contract states the handle holds `Native<WebView>`, so
    // that is the concrete type the erased view was built from.
    let view = unsafe { *any.downcast_unchecked::<waterui_core::Native<WebView>>() }.into_inner();
    let page = view
        .handle()
        .downcast_ref::<CefWebViewHandle>()
        .unwrap_or_else(|| panic!("Apple WebView handle does not use the selected CEF runtime"))
        .page()
        .clone();
    surface(page, view)
}

/// Drops the semantic view retained behind a CEF surface.
///
/// # Safety
///
/// `state` must be returned by a CEF force-as function and consumed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_drop(state: *mut WuiCefSurfaceState) {
    // SAFETY: the caller contract makes `state` the pointer a force-as function
    // handed out from `Box::into_raw`, consumed exactly once here.
    drop(unsafe { Box::from_raw(state) });
}

/// Installs the CEF-compatible `NSApplication` subclass before `AppKit` starts.
#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub extern "C" fn waterui_cef_prepare_macos_application() {
    waterui_browser_cef::initialize_macos_application();
}

/// Runs one packaged CEF helper subprocess and returns its exit status.
#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub extern "C" fn waterui_cef_run_packaged_subprocess() -> i32 {
    waterui_browser_cef::run_packaged_subprocess()
}
