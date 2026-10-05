#[cfg(any(feature = "android-jni", test))]
use core::ptr::NonNull;
#[cfg(not(target_vendor = "apple"))]
use core::ptr::null_mut;
use std::rc::Rc;

use nami::{Computed, SignalExt as _, signal::IntoComputed as _};
use waterui::window::{
    Activation, Monitor, MonitorSelector, ResolvedWindowBackground, UserAttention, Window,
    WindowBackground, WindowLevel, WindowManager, WindowPlacement, WindowState, WindowStyle,
    resolve_background,
};
use waterui::{AnyView, Str};
use waterui_graphics::WorkingColor;
use waterui_layout::{Rect, Size};

use crate::components::layout::WuiRect;

#[cfg(feature = "c-api")]
use crate::ffi_binding;
use crate::{
    IntoFFI, IntoRust, WuiAnyView, WuiEnv,
    closure::ForeignCallbackContext,
    reactive::{WuiBinding, WuiComputed},
};

/// FFI-compatible representation of [`WindowStyle`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiWindowStyle {
    /// Standard window with title bar and controls.
    Titled = 0,
    /// Borderless window without title bar.
    Borderless = 1,
    /// Window where content extends into the title bar area.
    FullSizeContentView = 2,
}

/// FFI mirror of [`MonitorSelector`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiMonitorSelector {
    /// The platform's primary display.
    Primary = 0,
    /// The display under the pointer when the window is shown.
    Pointer = 1,
    /// The display holding this application's focused window (`Primary` when none).
    Focused = 2,
}

impl From<WuiMonitorSelector> for MonitorSelector {
    fn from(selector: WuiMonitorSelector) -> Self {
        match selector {
            WuiMonitorSelector::Primary => Self::Primary,
            WuiMonitorSelector::Pointer => Self::Pointer,
            WuiMonitorSelector::Focused => Self::Focused,
        }
    }
}

impl From<MonitorSelector> for WuiMonitorSelector {
    fn from(selector: MonitorSelector) -> Self {
        match selector {
            MonitorSelector::Primary => Self::Primary,
            MonitorSelector::Pointer => Self::Pointer,
            MonitorSelector::Focused => Self::Focused,
        }
    }
}

/// FFI mirror of [`Activation`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiActivation {
    /// Showing the window activates the app and focuses the window.
    OnShow = 0,
    /// Showing does not take focus; a click on the window does.
    OnClick = 1,
    /// The window never takes keyboard focus or activates the app.
    Never = 2,
}

impl From<WuiActivation> for Activation {
    fn from(activation: WuiActivation) -> Self {
        match activation {
            WuiActivation::OnShow => Self::OnShow,
            WuiActivation::OnClick => Self::OnClick,
            WuiActivation::Never => Self::Never,
        }
    }
}

impl From<Activation> for WuiActivation {
    fn from(activation: Activation) -> Self {
        match activation {
            Activation::OnShow => Self::OnShow,
            Activation::OnClick => Self::OnClick,
            Activation::Never => Self::Never,
        }
    }
}

/// FFI mirror of [`Monitor`], built by the native backend that resolved the
/// placement's selector.
///
/// `name` is a borrowed NUL-terminated UTF-8 string (null when the platform
/// reports no name): the native caller keeps it alive for the duration of the
/// `place` call only — the Rust side copies what it needs before returning.
#[repr(C)]
#[derive(Debug)]
pub struct WuiMonitor {
    /// Bounds in the global logical coordinate space.
    pub frame: WuiRect,
    /// `frame` minus what the desktop reserves (menu bar, dock, panels, taskbar).
    pub visible_frame: WuiRect,
    /// Physical pixels per logical point.
    pub scale_factor: f64,
    /// The platform's name for the display, or null.
    pub name: *const core::ffi::c_char,
}

impl WuiMonitor {
    /// Converts the borrowed monitor the native backend built into the
    /// [`Monitor`] the `place` closure expects. Only called while the backend's
    /// `name` pointer is still valid.
    fn as_rust(&self) -> Monitor {
        let rect = |r: &WuiRect| {
            Rect::new(
                waterui_layout::Point::new(r.origin.x, r.origin.y),
                Size::new(r.size.width, r.size.height),
            )
        };
        Monitor {
            frame: rect(&self.frame),
            visible_frame: rect(&self.visible_frame),
            scale_factor: self.scale_factor,
            name: if self.name.is_null() {
                None
            } else {
                // SAFETY: the caller contract gives `name` a NUL-terminated
                // UTF-8 string valid for this call.
                let bytes = unsafe { core::ffi::CStr::from_ptr(self.name) }.to_bytes();
                Some(Str::from(String::from_utf8_lossy(bytes).into_owned()))
            },
        }
    }
}

/// Native invocation of [`WindowPlacement::place`]: the backend fills a
/// [`WuiMonitor`] for the resolved selector and receives the frame to write.
pub type WuiPlaceFn =
    unsafe extern "C" fn(context: *const (), monitor: *const WuiMonitor) -> WuiRect;

/// FFI mirror of [`WindowPlacement`]: the selector plus the `place` closure as
/// the usual context/call/drop triple.
#[repr(C)]
#[derive(Debug)]
pub struct WuiWindowPlacement {
    /// Which monitor the backend resolves before calling `call`.
    pub monitor: WuiMonitorSelector,
    /// The `place` closure's context, registered with `call` and `drop`.
    pub context: *mut (),
    /// Resolved monitor in, window frame out.
    pub call: WuiPlaceFn,
    /// Releases `context` exactly once when the window record is disposed.
    pub drop: unsafe extern "C" fn(*mut ()),
}

/// Calls a `place` closure held in a `WuiWindowPlacement`'s context.
///
/// # Safety
/// `data` is the `Box::into_raw` of the `Rc<dyn Fn(&Monitor) -> Rect>` the
/// conversion stored, and `monitor` points at a valid `WuiMonitor` whose `name`
/// lives for the call.
unsafe extern "C" fn placement_call(data: *const (), monitor: *const WuiMonitor) -> WuiRect {
    // SAFETY: upheld by the caller contract on `placement_into_ffi`.
    let place = unsafe { &*(data.cast::<Rc<dyn Fn(&Monitor) -> Rect>>()) };
    // SAFETY: `monitor` points at the valid `WuiMonitor` the caller passed.
    let monitor = unsafe { (*monitor).as_rust() };
    place(&monitor).into_ffi()
}

/// Releases a `place` closure boxed by `placement_into_ffi`.
///
/// # Safety
/// `data` is the pointer `placement_into_ffi` produced, released exactly once.
unsafe extern "C" fn placement_drop(data: *mut ()) {
    // SAFETY: `data` is the `Box::into_raw` of the `place` closure.
    unsafe { drop(Box::from_raw(data.cast::<Rc<dyn Fn(&Monitor) -> Rect>>())) };
}

/// Moves a [`WindowPlacement`] into its FFI triple, or null for `None`.
fn placement_into_ffi(placement: Option<WindowPlacement>) -> *mut WuiWindowPlacement {
    let Some(placement) = placement else {
        return core::ptr::null_mut();
    };
    let context = Box::into_raw(Box::new(placement.place)).cast::<()>();
    Box::into_raw(Box::new(WuiWindowPlacement {
        monitor: placement.monitor.into(),
        context,
        call: placement_call,
        drop: placement_drop,
    }))
}

/// Disposes an FFI placement: releases the context through its `drop`, then
/// the record itself.
///
/// # Safety
/// `placement` is a pointer produced by `placement_into_ffi` (or null), not
/// already released.
#[cfg(any(feature = "android-jni", test))]
unsafe fn dispose_placement(placement: *mut WuiWindowPlacement) {
    if placement.is_null() {
        return;
    }
    // SAFETY: the record and its context are the ones `placement_into_ffi`
    // registered; each is released exactly once.
    unsafe {
        ((*placement).drop)((*placement).context);
        drop(Box::from_raw(placement));
    }
}

impl From<WindowStyle> for WuiWindowStyle {
    fn from(style: WindowStyle) -> Self {
        match style {
            WindowStyle::Titled => Self::Titled,
            WindowStyle::Borderless => Self::Borderless,
            WindowStyle::FullSizeContentView => Self::FullSizeContentView,
        }
    }
}

impl IntoFFI for WindowStyle {
    type FFI = WuiWindowStyle;

    fn into_ffi(self) -> Self::FFI {
        self.into()
    }
}

// Native backends read and observe the style; only Rust writes it.
crate::ffi_computed!(WindowStyle, WuiWindowStyle, window_style);

/// FFI-compatible representation of [`WindowState`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiWindowState {
    /// The window is in its normal state.
    Normal = 0,
    /// The window is closed.
    Closed = 1,
    /// The window is minimized.
    Minimized = 2,
    /// The window is maximized to fullscreen.
    Fullscreen = 3,
    /// The window fills the screen's work area, keeping its chrome and
    /// the system's panels.
    Maximized = 4,
}

impl From<WindowState> for WuiWindowState {
    fn from(state: WindowState) -> Self {
        match state {
            WindowState::Normal => Self::Normal,
            WindowState::Closed => Self::Closed,
            WindowState::Minimized => Self::Minimized,
            WindowState::Maximized => Self::Maximized,
            WindowState::Fullscreen => Self::Fullscreen,
        }
    }
}

impl IntoFFI for WindowState {
    type FFI = WuiWindowState;

    fn into_ffi(self) -> Self::FFI {
        self.into()
    }
}

impl IntoRust for WuiWindowState {
    type Rust = WindowState;

    unsafe fn into_rust(self) -> Self::Rust {
        match self {
            Self::Normal => WindowState::Normal,
            Self::Closed => WindowState::Closed,
            Self::Minimized => WindowState::Minimized,
            Self::Maximized => WindowState::Maximized,
            Self::Fullscreen => WindowState::Fullscreen,
        }
    }
}

// Generate C FFI binding functions and the native watcher for WindowState.
#[cfg(feature = "c-api")]
ffi_binding!(WindowState, WuiWindowState, window_state);
crate::ffi_watcher!(WindowState, WuiWindowState, window_state);

/// FFI-compatible representation of [`WindowLevel`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiWindowLevel {
    /// The window stacks with other windows as focus moves between them.
    Normal = 0,
    /// The window stays above other applications' normal windows.
    AlwaysOnTop = 1,
}

impl From<WindowLevel> for WuiWindowLevel {
    fn from(level: WindowLevel) -> Self {
        match level {
            WindowLevel::Normal => Self::Normal,
            WindowLevel::AlwaysOnTop => Self::AlwaysOnTop,
        }
    }
}

impl IntoFFI for WindowLevel {
    type FFI = WuiWindowLevel;

    fn into_ffi(self) -> Self::FFI {
        self.into()
    }
}

impl IntoRust for WuiWindowLevel {
    type Rust = WindowLevel;

    unsafe fn into_rust(self) -> Self::Rust {
        match self {
            Self::Normal => WindowLevel::Normal,
            Self::AlwaysOnTop => WindowLevel::AlwaysOnTop,
        }
    }
}

// Generate C FFI computed functions and the native watcher for WindowLevel.
#[cfg(feature = "c-api")]
crate::ffi_computed!(WindowLevel, WuiWindowLevel, window_level);

/// FFI-compatible representation of `Option<UserAttention>` (see
/// [`UserAttention`]).
///
/// `None` is a variant of the enum itself so that watching the attention
/// binding reports a withdrawn request without a second out-of-band channel.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiUserAttention {
    /// No attention request is pending.
    None = 0,
    /// Something the user may want to look at: a finished task, a mention.
    Informational = 1,
    /// Something the user must act on.
    Critical = 2,
}

impl From<UserAttention> for WuiUserAttention {
    fn from(attention: UserAttention) -> Self {
        match attention {
            UserAttention::Informational => Self::Informational,
            UserAttention::Critical => Self::Critical,
        }
    }
}

impl IntoFFI for Option<UserAttention> {
    type FFI = WuiUserAttention;

    fn into_ffi(self) -> Self::FFI {
        self.map_or(WuiUserAttention::None, Into::into)
    }
}

impl IntoRust for WuiUserAttention {
    type Rust = Option<UserAttention>;

    unsafe fn into_rust(self) -> Self::Rust {
        match self {
            Self::None => None,
            Self::Informational => Some(UserAttention::Informational),
            Self::Critical => Some(UserAttention::Critical),
        }
    }
}

// Generate C FFI binding functions and the native watcher for the attention
// binding.
#[cfg(feature = "c-api")]
ffi_binding!(Option<UserAttention>, WuiUserAttention, user_attention);
crate::ffi_watcher!(Option<UserAttention>, WuiUserAttention, user_attention);

/// The colour the Kotlin Android runtime paints behind a window's content,
/// following `background`.
///
/// The Kotlin Android runtime realizes no material: a
/// [`WindowBackground::Material`] draws the window opaque in the theme's
/// background colour, as [`WindowBackground::Opaque`] does. Hydrolysis is the
/// Android backend that realizes a material window background
/// (water-rs/waterui#1899).
fn android_window_background(
    background: &Computed<WindowBackground>,
    env: &waterui::Environment,
) -> Computed<WorkingColor> {
    let without_material = background.map(|background| match background {
        WindowBackground::Material(_) => WindowBackground::Opaque,
        background => background,
    });
    resolve_background(&without_material, env)
        .map(|resolved| match resolved {
            ResolvedWindowBackground::Color(color) => color,
            ResolvedWindowBackground::Material(_) => {
                unreachable!("every material was mapped to an opaque background above")
            }
        })
        .into_computed()
}

/// Resolves a window's reactive background to the colour the Kotlin Android
/// runtime paints behind the window's content, consuming `background`.
///
/// The returned signal follows both a change of the background — including a
/// switch between opaque and a translucent colour — and a change of the colour
/// it resolves to. A colour whose opacity is below one asks for a translucent
/// window.
///
/// The Kotlin Android runtime realizes no material, so a
/// [`WindowBackground::Material`] resolves to the theme's background colour,
/// drawing the window opaque; Hydrolysis is the Android backend that realizes
/// a material window background (water-rs/waterui#1899).
///
/// # Safety
///
/// `background` must be the owning `WuiWindow.background` handle, consumed by
/// this call and not used afterwards; `env` must be a valid `WuiEnv` borrowed
/// for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_resolve_window_background(
    background: *mut WuiComputed<WindowBackground>,
    env: *const WuiEnv,
) -> *mut WuiComputed<WorkingColor> {
    // SAFETY: the caller contract makes `background` an owning handle reclaimed
    // exactly once here, and `env` a valid borrow for the call.
    unsafe {
        let background = Box::from_raw(background).0;
        android_window_background(&background, &*env).into_ffi()
    }
}

/// FFI-compatible representation of a window.
#[repr(C)]
#[derive(Debug)]
pub struct WuiWindow {
    /// The title of the window.
    pub title: *mut WuiComputed<Str>,
    /// Whether the window is closable.
    pub closable: bool,
    /// Whether the window is resizable.
    pub resizable: bool,
    /// The frame of the window.
    pub frame: *mut WuiBinding<Rect>,
    /// The content of the window.
    pub content: *mut WuiAnyView,
    /// The current state of the window.
    pub state: *mut WuiBinding<WindowState>,
    /// Optional toolbar content (null if none).
    pub toolbar: *mut WuiAnyView,
    /// The visual style of the window, observed so a change after the window
    /// is shown is re-applied.
    pub style: *mut WuiComputed<WindowStyle>,
    /// The window's reactive background. Resolve it with
    /// `waterui_resolve_window_background`, which consumes it.
    pub background: *mut WuiComputed<WindowBackground>,
    /// Explicit minimum content size, or null to derive the minimum from the
    /// content's layout (the root view measured at a zero proposal).
    pub min_size: *mut WuiComputed<Size>,
    /// Explicit maximum content size, or null for an unconstrained window.
    pub max_size: *mut WuiComputed<Size>,
    /// Monitor selection plus the `place` callback, or null for the
    /// platform's default placement.
    pub placement: *mut WuiWindowPlacement,
    /// How showing and clicking the window affects focus and app activation.
    pub activation: WuiActivation,
    /// Where the window stacks relative to other applications' windows.
    pub level: *mut WuiComputed<WindowLevel>,
    /// The window's request for the user's attention. The backend sets it back
    /// to `None` when the window gains focus.
    pub attention: *mut WuiBinding<Option<UserAttention>>,
    /// The steps the window's content size moves in while the user resizes it,
    /// or null for continuous resizing.
    pub resize_increments: *mut WuiComputed<Size>,
}

/// A uniquely owned pointer produced by [`IntoFFI`].
///
/// Keeping the ownership in a type lets Android discard unsupported window
/// properties without scattering raw `Box::from_raw` calls across the app
/// projection path.
#[cfg(any(feature = "android-jni", test))]
pub(crate) struct OwnedFfiHandle<T>(NonNull<T>);

#[cfg(any(feature = "android-jni", test))]
impl<T> OwnedFfiHandle<T> {
    #[track_caller]
    pub(crate) fn required(pointer: *mut T, field: &'static str) -> Self {
        let pointer = NonNull::new(pointer).unwrap_or_else(|| panic!("{field} must not be null"));
        Self(pointer)
    }

    fn optional(pointer: *mut T) -> Option<Self> {
        NonNull::new(pointer).map(Self)
    }

    pub(crate) const fn as_ptr(&self) -> *mut T {
        self.0.as_ptr()
    }

    pub(crate) const fn into_raw(self) -> *mut T {
        let pointer = self.as_ptr();
        core::mem::forget(self);
        pointer
    }
}

#[cfg(any(feature = "android-jni", test))]
impl<T> Drop for OwnedFfiHandle<T> {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the boxed value `self.0` points at, and `Drop`
        // runs once.
        unsafe {
            drop(alloc::boxed::Box::from_raw(self.0.as_ptr()));
        }
    }
}

/// The window properties Android's root activity realizes.
#[cfg(any(feature = "android-jni", test))]
pub(crate) struct WuiAndroidWindow {
    /// The root content view.
    pub(crate) content: OwnedFfiHandle<WuiAnyView>,
    /// The resolved background colour, applied with `setBackgroundDrawable`.
    pub(crate) background: OwnedFfiHandle<WuiComputed<WorkingColor>>,
}

#[cfg(any(feature = "android-jni", test))]
impl WuiWindow {
    /// Retains the window properties Android's root activity consumes —
    /// resolving the background in `env` — and releases every other
    /// Rust-owned FFI handle.
    pub(crate) fn into_android_window(self, env: &waterui::Environment) -> WuiAndroidWindow {
        let Self {
            title,
            closable: _,
            resizable: _,
            frame,
            content,
            state,
            toolbar,
            style,
            background,
            min_size,
            max_size,
            placement,
            activation: _,
            level,
            attention,
            resize_increments,
        } = self;
        // SAFETY: `placement` is the pointer `placement_into_ffi` produced for
        // this window, not yet released.
        unsafe { dispose_placement(placement) };

        // Android's single-Activity model has no window states: the `state`
        // binding is dropped, so a `Maximized` write maps to Normal — the
        // window is the activity, always filling the screen. `level`,
        // `attention` and `resize_increments` are dropped the same way; they
        // have no Android meaning.
        let unused_handles = (
            OwnedFfiHandle::required(title, "WuiWindow.title"),
            OwnedFfiHandle::optional(frame),
            OwnedFfiHandle::required(state, "WuiWindow.state"),
            OwnedFfiHandle::optional(toolbar),
            OwnedFfiHandle::required(style, "WuiWindow.style"),
            OwnedFfiHandle::optional(min_size),
            OwnedFfiHandle::optional(max_size),
            OwnedFfiHandle::required(level, "WuiWindow.level"),
            OwnedFfiHandle::required(attention, "WuiWindow.attention"),
            OwnedFfiHandle::optional(resize_increments),
        );
        let background = OwnedFfiHandle::required(background, "WuiWindow.background");
        // SAFETY: `background` is this window's owning handle; taking it out
        // of the wrapper hands the one release to the resolved signal.
        let background = unsafe { Box::from_raw(background.into_raw()) }.0;
        let background = OwnedFfiHandle::required(
            android_window_background(&background, env).into_ffi(),
            "resolved window background",
        );
        let content = OwnedFfiHandle::required(content, "WuiWindow.content");
        drop(unused_handles);
        WuiAndroidWindow {
            content,
            background,
        }
    }

    /// Releases a window which Android cannot represent.
    pub(crate) fn dispose_android(self, env: &waterui::Environment) {
        drop(self.into_android_window(env));
    }
}

#[cfg(target_vendor = "apple")]
fn toolbar_into_ffi(toolbar: Option<AnyView>) -> *mut WuiAnyView {
    toolbar.into_ffi()
}

#[cfg(not(target_vendor = "apple"))]
fn toolbar_into_ffi(_toolbar: Option<AnyView>) -> *mut WuiAnyView {
    null_mut()
}

impl IntoFFI for Window {
    type FFI = WuiWindow;

    fn into_ffi(self) -> Self::FFI {
        let content = self.build_content();
        // Resolved before the window is taken apart, because it reads the
        // declared title alongside the application's name.
        let title = self.display_title();
        // Apple consumes the toolbar as native window chrome. Other backends,
        // including Android, drop the Rust view without allocating an FFI handle.
        let toolbar = toolbar_into_ffi(self.toolbar);

        WuiWindow {
            // Every backend is handed the title to show, not the raw
            // declaration, so none of them repeats the rule.
            title: title.into_ffi(),
            closable: self.closable,
            resizable: self.resizable,
            frame: self.frame.into_ffi(),
            content: content.into_ffi(),
            state: self.state.into_ffi(),
            toolbar,
            style: self.style.computed().into_ffi(),
            background: self.background.computed().into_ffi(),
            min_size: self.min_size.into_ffi(),
            max_size: self.max_size.into_ffi(),
            placement: placement_into_ffi(self.placement),
            activation: self.activation.into(),
            level: self.level.into_ffi(),
            attention: self.attention.into_ffi(),
            resize_increments: self.resize_increments.into_ffi(),
        }
    }
}

// =============================================================================
// WindowManager FFI - Environment Service Installation
// =============================================================================

/// Type alias for the native window show function.
///
/// This function is called by Rust when a `Window` view needs to be shown.
/// The native implementation should create and display the window.
/// # Parameters
/// - context: The native window-manager owner
/// - `WuiWindow`: The window configuration to show
pub type WindowShowFn = unsafe extern "C" fn(context: *mut (), window: WuiWindow);

/// FFI-compatible `WindowManager` implementation.
struct FFIWindowManager {
    context: ForeignCallbackContext,
    show_fn: WindowShowFn,
}

impl FFIWindowManager {
    fn show(&self, window: Window) {
        let ffi_window = window.into_ffi();
        // SAFETY: `show_fn` and the context are one registration, kept alive by
        // `self`; the window is handed to backend ownership.
        unsafe {
            (self.show_fn)(self.context.data(), ffi_window);
        }
    }
}

/// Installs a `WindowManager` into the environment from a native function pointer.
///
/// Native backends call this during initialization to register their window
/// management implementation. When `Window` views are rendered, the provided
/// callback will be invoked to create and display native windows.
///
/// # Safety
///
/// The caller must ensure that:
/// - `env` is a valid pointer to a `WuiEnv`
/// - `context` remains valid until `drop_context` releases it
/// - `show_fn` is valid for `context` and can create native windows
/// - `drop_context` releases `context` exactly once
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_install_window_manager(
    env: *mut WuiEnv,
    context: *mut (),
    show_fn: WindowShowFn,
    drop_context: unsafe extern "C" fn(*mut ()),
) {
    // SAFETY: the caller contract requires `env` to be a valid handle, alive and not
    // otherwise borrowed for this call; the exclusive borrow ends here.
    let env = unsafe { crate::borrow_ffi_mut(env) };

    let ffi_manager = FFIWindowManager {
        // SAFETY: the caller contract requires `context` and `drop_context` to be one
        // registration from the backend.
        context: unsafe { ForeignCallbackContext::new(context, drop_context) },
        show_fn,
    };

    let manager = WindowManager::new(move |window| {
        ffi_manager.show(window);
    });

    env.insert(manager);
}

#[cfg(test)]
mod tests {
    use nami::{Binding, Signal as _, SignalExt as _};
    use waterui::background::Material;
    use waterui::theme::{ColorSettings, Theme};
    use waterui::window::WindowBackground;
    use waterui_core::plugin::Plugin as _;
    use waterui_graphics::{Color, color::working::from_linear_srgb};

    use super::android_window_background;

    /// A colour passes through; a material, which the Kotlin Android runtime
    /// does not realize, resolves as an opaque background does.
    #[test]
    fn the_kotlin_runtime_draws_a_material_window_background_opaque() {
        let mut env = waterui::Environment::new();
        Theme::new()
            .colors(ColorSettings::new().background(from_linear_srgb([0.5, 0.5, 0.5], 1.0)))
            .install(&mut env);
        let background = Binding::container(WindowBackground::Color(
            Color::srgb(0, 0, 0).with_opacity(0.5),
        ));
        let resolved = android_window_background(&background.computed(), &env);
        assert!((resolved.snapshot().components[3] - 0.5).abs() < 1e-6);
        background.set(WindowBackground::Material(Material::Thin));
        let material = resolved.snapshot();
        background.set(WindowBackground::Opaque);
        assert_eq!(material, resolved.snapshot());
    }
}
