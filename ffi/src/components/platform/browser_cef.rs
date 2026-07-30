//! C ABI for the optional CEF runtime used by native renderers.

use std::any::Any;
#[cfg(not(target_os = "macos"))]
use std::cell::Cell;
#[cfg(not(target_os = "macos"))]
use std::rc::Rc;
#[cfg(not(target_os = "macos"))]
use std::time::Instant;

#[cfg(not(target_os = "macos"))]
use executor_core::spawn_local;
#[cfg(target_os = "macos")]
use waterui_browser_cef::CefMacOsMessagePump;
#[cfg(any(feature = "webview-cef", feature = "cef-header"))]
use waterui_browser_cef::CefWebViewHandle;
use waterui_browser_cef::{
    CefInputModifiers, CefKeyInput, CefPageHandle, CefPointerButton, CefRuntime,
    CefRuntimeConfiguration, CefTextRange, CefViewport, gpu_view,
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
use crate::{IntoFFI, IntoRust, WuiStr};

#[cfg(not(target_os = "macos"))]
struct CefPumpLifetime(Rc<Cell<bool>>);

#[cfg(not(target_os = "macos"))]
impl Drop for CefPumpLifetime {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

/// Installs one process-owned CEF runtime and the selected public controllers.
pub(crate) fn configure_environment(env: &mut Environment) {
    let runtime = CefRuntime::initialize(CefRuntimeConfiguration::packaged());
    #[cfg(any(feature = "webview-cef", feature = "cef-header"))]
    env.insert(runtime.webview_controller());
    #[cfg(any(feature = "chromium", feature = "cef-header"))]
    env.insert(runtime.chromium_controller());
    env.insert(runtime.clone());

    #[cfg(target_os = "macos")]
    env.insert(CefMacOsMessagePump::install(runtime));
    #[cfg(not(target_os = "macos"))]
    {
        let active = Rc::new(Cell::new(true));
        env.insert(CefPumpLifetime(Rc::clone(&active)));
        spawn_local(async move {
            while active.get() {
                let deadline = runtime.pump().instant();
                async_io::Timer::at(deadline.max(Instant::now())).await;
            }
        })
        .detach();
    }
}

/// GPU surface plus retained CEF input and semantic state.
#[repr(C)]
pub struct WuiCefSurface {
    /// GPU presenter consumed by WaterUI's native GPU surface host.
    pub gpu_surface: WuiGpuSurface,
    /// Opaque input state retained until [`waterui_cef_surface_drop`].
    pub state: *mut WuiCefSurfaceState,
}

/// Modifier snapshot for CEF pointer and keyboard input.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WuiCefInputModifiers {
    /// Shift key.
    pub shift: bool,
    /// Control key.
    pub control: bool,
    /// Alt or Option key.
    pub alt: bool,
    /// Command key.
    pub command: bool,
    /// Primary pointer button.
    pub primary_button: bool,
    /// Middle pointer button.
    pub middle_button: bool,
    /// Secondary pointer button.
    pub secondary_button: bool,
}

impl From<WuiCefInputModifiers> for CefInputModifiers {
    fn from(value: WuiCefInputModifiers) -> Self {
        Self {
            shift: value.shift,
            control: value.control,
            alt: value.alt,
            command: value.command,
            primary_button: value.primary_button,
            middle_button: value.middle_button,
            secondary_button: value.secondary_button,
        }
    }
}

/// Pointer buttons supported by CEF windowless rendering.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum WuiCefPointerButton {
    /// Primary pointer button.
    Primary,
    /// Middle pointer button.
    Middle,
    /// Secondary pointer button.
    Secondary,
}

impl From<WuiCefPointerButton> for CefPointerButton {
    fn from(value: WuiCefPointerButton) -> Self {
        match value {
            WuiCefPointerButton::Primary => Self::Primary,
            WuiCefPointerButton::Middle => Self::Middle,
            WuiCefPointerButton::Secondary => Self::Secondary,
        }
    }
}

/// Editing operations forwarded to Chromium's focused frame.
#[repr(C)]
#[derive(Clone, Copy)]
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

pub struct WuiCefSurfaceState {
    page: CefPageHandle,
    viewport: CefViewport,
    _source: Box<dyn Any>,
}

fn surface(page: CefPageHandle, source: impl Any) -> WuiCefSurface {
    let viewport = CefViewport::new();
    let gpu_surface = GpuSurface::new(gpu_view(page.clone(), viewport.clone())).into_ffi();
    WuiCefSurface {
        gpu_surface,
        state: Box::into_raw(Box::new(WuiCefSurfaceState {
            page,
            viewport,
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

/// Consumes a standard WebView whose selected engine is CEF.
///
/// # Safety
///
/// `view` must be a valid owning `WuiAnyView` containing `Native<WebView>`.
#[cfg(any(feature = "webview-cef", feature = "cef-header"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_force_as_cef_webview(view: *mut WuiAnyView) -> WuiCefSurface {
    let any: waterui::AnyView = unsafe { IntoRust::into_rust(view) };
    let view = unsafe { *any.downcast_unchecked::<waterui_core::Native<WebView>>() }.into_inner();
    let page = view
        .handle()
        .downcast_ref::<CefWebViewHandle>()
        .unwrap_or_else(|| panic!("Apple WebView handle does not use the selected CEF runtime"))
        .page()
        .clone();
    surface(page, view)
}

fn borrow_state<'a>(state: *const WuiCefSurfaceState) -> &'a WuiCefSurfaceState {
    unsafe { crate::borrow_ffi(state) }
}

/// Updates focus for a CEF surface.
///
/// # Safety
///
/// `state` must be a live state returned by a CEF force-as function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_set_focus(
    state: *const WuiCefSurfaceState,
    focused: bool,
) {
    borrow_state(state).page.set_focus(focused);
}

/// Requests one compositor frame for a visible CEF surface.
///
/// # Safety
///
/// `state` must be a live state returned by a CEF force-as function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_request_frame(state: *const WuiCefSurfaceState) {
    borrow_state(state).page.request_frame();
}

/// Updates the logical browser viewport and device scale.
///
/// # Safety
///
/// `state` must be a live state returned by a CEF force-as function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_set_viewport(
    state: *const WuiCefSurfaceState,
    width: u32,
    height: u32,
    scale: f64,
) {
    let state = borrow_state(state);
    state.viewport.set_scale(scale);
    assert!(
        scale <= f64::from(f32::MAX),
        "CEF viewport scale exceeds f32"
    );
    state.page.set_viewport(width, height, scale as f32);
}

/// Navigates the CEF surface backward.
///
/// # Safety
///
/// `state` must be a live state returned by a CEF force-as function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_go_back(state: *const WuiCefSurfaceState) {
    borrow_state(state).page.go_back();
}

/// Navigates the CEF surface forward.
///
/// # Safety
///
/// `state` must be a live state returned by a CEF force-as function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_go_forward(state: *const WuiCefSurfaceState) {
    borrow_state(state).page.go_forward();
}

/// Sends pointer movement in surface-local logical coordinates.
///
/// # Safety
///
/// `state` must be a live state returned by a CEF force-as function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_pointer_move(
    state: *const WuiCefSurfaceState,
    x: f64,
    y: f64,
    modifiers: WuiCefInputModifiers,
) {
    borrow_state(state)
        .page
        .pointer_move(x, y, modifiers.into());
}

/// Sends one pointer button transition.
///
/// # Safety
///
/// `state` must be a live state returned by a CEF force-as function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_pointer_button(
    state: *const WuiCefSurfaceState,
    pressed: bool,
    button: WuiCefPointerButton,
    x: f64,
    y: f64,
    modifiers: WuiCefInputModifiers,
) {
    borrow_state(state)
        .page
        .pointer_button(pressed, button.into(), x, y, modifiers.into());
}

/// Sends a CEF wheel event.
///
/// # Safety
///
/// `state` must be a live state returned by a CEF force-as function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_scroll(
    state: *const WuiCefSurfaceState,
    x: f64,
    y: f64,
    delta_x: f64,
    delta_y: f64,
    modifiers: WuiCefInputModifiers,
) {
    borrow_state(state)
        .page
        .scroll(x, y, delta_x, delta_y, modifiers.into());
}

/// Sends one native key transition.
///
/// `character` is a Unicode scalar value, or zero when the key has no text.
///
/// # Safety
///
/// `state` must be a live state returned by a CEF force-as function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_key(
    state: *const WuiCefSurfaceState,
    pressed: bool,
    native_keycode: u32,
    keyval: u32,
    character: u32,
    modifiers: WuiCefInputModifiers,
) {
    let character = (character != 0).then(|| {
        char::from_u32(character)
            .unwrap_or_else(|| panic!("CEF key character is not a Unicode scalar: {character}"))
    });
    borrow_state(state).page.key(
        pressed,
        CefKeyInput {
            native_keycode,
            keyval,
            character,
        },
        modifiers.into(),
    );
}

/// Commits text to the focused Chromium editor.
///
/// # Safety
///
/// `state` and `text` must be valid owning FFI values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_commit_text(
    state: *const WuiCefSurfaceState,
    text: WuiStr,
    replacement_start: u32,
    replacement_end: u32,
) {
    let text: waterui::Str = unsafe { text.into_rust() };
    borrow_state(state).page.commit_text(
        text.as_str(),
        optional_cef_range(replacement_start, replacement_end),
    );
}

/// Updates active IME composition text and its UTF-16 selection.
///
/// # Safety
///
/// `state` and `text` must be valid owning FFI values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_set_composition(
    state: *const WuiCefSurfaceState,
    text: WuiStr,
    selection_start: u32,
    selection_end: u32,
    replacement_start: u32,
    replacement_end: u32,
) {
    let text: waterui::Str = unsafe { text.into_rust() };
    borrow_state(state).page.set_composition(
        text.as_str(),
        selection_start,
        selection_end,
        optional_cef_range(replacement_start, replacement_end),
    );
}

fn optional_cef_range(start: u32, end: u32) -> Option<CefTextRange> {
    if start == u32::MAX || end == u32::MAX {
        assert_eq!(
            (start, end),
            (u32::MAX, u32::MAX),
            "CEF replacement range must be entirely specified or entirely absent"
        );
        return None;
    }
    Some(CefTextRange::new(start, end))
}

/// Finishes active IME composition.
///
/// # Safety
///
/// `state` must be a live state returned by a CEF force-as function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_finish_composition(
    state: *const WuiCefSurfaceState,
    keep_selection: bool,
) {
    borrow_state(state).page.finish_composition(keep_selection);
}

/// Cancels active IME composition.
///
/// # Safety
///
/// `state` must be a live state returned by a CEF force-as function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_cef_surface_cancel_composition(state: *const WuiCefSurfaceState) {
    borrow_state(state).page.cancel_composition();
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
    let page = &borrow_state(state).page;
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
    drop(unsafe { Box::from_raw(state) });
}

/// Installs the CEF-compatible `NSApplication` subclass before AppKit starts.
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
