//! C ABI for the optional CEF runtime used by native renderers.

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

#[cfg(any(feature = "webview-cef", feature = "cef-header"))]
use crate::WuiAnyView;
use crate::components::visual::gpu_surface::WuiGpuSurface;
use crate::{IntoFFI, IntoRust};

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

/// GPU surface plus retained CEF input and semantic state.
#[repr(C)]
#[derive(Debug)]
pub struct WuiCefSurface {
    /// GPU presenter consumed by `WaterUI`'s native GPU surface host.
    pub gpu_surface: WuiGpuSurface,
    /// Opaque input state retained until [`waterui_cef_surface_drop`].
    pub state: *mut WuiCefSurfaceState,
}

/// Editing operations forwarded to Chromium's focused frame.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub enum WuiCefEditCommand {
    /// Undo.
    Undo,
    /// Redo.
    Redo,
    /// Cut.
    Cut,
    /// Copy.
    Copy,
    /// Paste.
    Paste,
    /// Select all.
    SelectAll,
}

/// Opaque CEF state the native backend owns for the surface's lifetime.
///
/// Keeps the page handle every input and navigation entry point addresses, plus
/// the semantic view the page was created from, alive until
/// [`waterui_cef_surface_drop`].
pub struct WuiCefSurfaceState {
    page: CefPageHandle,
    _source: Box<dyn Any>,
}

impl fmt::Debug for WuiCefSurfaceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Neither the CEF page handle nor the type-erased source view has a
        // Debug representation, so only the state's identity is reported.
        f.debug_struct("WuiCefSurfaceState").finish_non_exhaustive()
    }
}

fn surface(page: CefPageHandle, source: impl Any) -> WuiCefSurface {
    let gpu_surface = GpuSurface::new(gpu_view_with_input(page.clone())).into_ffi();
    WuiCefSurface {
        gpu_surface,
        state: Box::into_raw(Box::new(WuiCefSurfaceState {
            page,
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

/// Reborrows the state handle every CEF surface entry point receives.
///
/// # Safety
///
/// `state` must be a live state returned by a CEF force-as function that stays
/// alive and unaliased for the duration of the returned `'a` borrow.
const unsafe fn borrow_state<'a>(state: *const WuiCefSurfaceState) -> &'a WuiCefSurfaceState {
    // SAFETY: this function's own contract is exactly `borrow_ffi`'s: a live,
    // initialized state that outlives the borrow it hands back.
    unsafe { crate::borrow_ffi(state) }
}

/// Navigates the CEF surface backward.
///
/// # Safety
///
/// `state` must be a live state returned by a CEF force-as function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_go_back(state: *const WuiCefSurfaceState) {
    // SAFETY: the caller contract stated above is `borrow_state`'s: a live CEF
    // surface state, borrowed only for this call.
    unsafe { borrow_state(state) }.page.go_back();
}

/// Navigates the CEF surface forward.
///
/// # Safety
///
/// `state` must be a live state returned by a CEF force-as function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_go_forward(state: *const WuiCefSurfaceState) {
    // SAFETY: the caller contract stated above is `borrow_state`'s: a live CEF
    // surface state, borrowed only for this call.
    unsafe { borrow_state(state) }.page.go_forward();
}

/// Executes an editing command in Chromium's focused frame.
///
/// # Safety
///
/// `state` must be a live state returned by a CEF force-as function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_edit(
    state: *const WuiCefSurfaceState,
    command: WuiCefEditCommand,
) {
    // SAFETY: the caller contract stated above is `borrow_state`'s: a live CEF
    // surface state, borrowed only for this call.
    let page = &unsafe { borrow_state(state) }.page;
    match command {
        WuiCefEditCommand::Undo => page.undo(),
        WuiCefEditCommand::Redo => page.redo(),
        WuiCefEditCommand::Cut => page.cut(),
        WuiCefEditCommand::Copy => page.copy(),
        WuiCefEditCommand::Paste => page.paste(),
        WuiCefEditCommand::SelectAll => page.select_all(),
    }
}

/// Drops retained CEF input and semantic state.
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
