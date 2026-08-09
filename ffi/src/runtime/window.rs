#[cfg(any(feature = "android-jni", test))]
use core::ptr::NonNull;
#[cfg(not(target_vendor = "apple"))]
use core::ptr::null_mut;

use waterui::window::{Window, WindowBackground, WindowManager, WindowState, WindowStyle};
use waterui::{AnyView, Str};
use waterui_layout::{Rect, Size};

#[cfg(feature = "c-api")]
use crate::ffi_binding;
use crate::{
    IntoFFI, IntoRust, WuiAnyView, WuiEnv,
    closure::ForeignCallbackContext,
    color::WuiColor,
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

impl From<WindowStyle> for WuiWindowStyle {
    fn from(style: WindowStyle) -> Self {
        match style {
            WindowStyle::Titled => Self::Titled,
            WindowStyle::Borderless => Self::Borderless,
            WindowStyle::FullSizeContentView => Self::FullSizeContentView,
        }
    }
}

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
}

impl From<WindowState> for WuiWindowState {
    fn from(state: WindowState) -> Self {
        match state {
            WindowState::Normal => Self::Normal,
            WindowState::Closed => Self::Closed,
            WindowState::Minimized => Self::Minimized,
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
            Self::Fullscreen => WindowState::Fullscreen,
        }
    }
}

// Generate C FFI binding functions and the native watcher for WindowState.
#[cfg(feature = "c-api")]
ffi_binding!(WindowState, WuiWindowState, window_state);
crate::ffi_watcher!(WindowState, WuiWindowState, window_state);

/// FFI-compatible representation of [`WindowBackground`].
///
/// Only supports Opaque and Color. Material blur effects are handled
/// via `MaterialBackground` metadata on the window content.
#[repr(C)]
#[derive(Debug)]
pub enum WuiWindowBackground {
    /// Opaque system default background.
    Opaque,
    /// Solid color background (can be semi-transparent via alpha).
    /// Native must resolve the color using the environment.
    Color {
        /// Pointer to the reactive color to resolve and apply as the window background.
        color: *mut WuiColor,
    },
}

impl From<WindowBackground> for WuiWindowBackground {
    fn from(bg: WindowBackground) -> Self {
        match bg {
            WindowBackground::Opaque => Self::Opaque,
            WindowBackground::Color(color) => Self::Color {
                color: color.into_ffi(),
            },
        }
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
    /// The visual style of the window.
    pub style: WuiWindowStyle,
    /// The background style of the window.
    pub background: WuiWindowBackground,
    /// Explicit minimum content size, or null to derive the minimum from the
    /// content's layout (the root view measured at a zero proposal).
    pub min_size: *mut WuiComputed<Size>,
    /// Explicit maximum content size, or null for an unconstrained window.
    pub max_size: *mut WuiComputed<Size>,
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
        unsafe {
            drop(alloc::boxed::Box::from_raw(self.0.as_ptr()));
        }
    }
}

#[cfg(any(feature = "android-jni", test))]
impl WuiWindowBackground {
    fn into_android_owned_color(self) -> Option<OwnedFfiHandle<WuiColor>> {
        match self {
            Self::Opaque => None,
            Self::Color { color } => Some(OwnedFfiHandle::required(
                color,
                "WuiWindow.background.color",
            )),
        }
    }
}

#[cfg(any(feature = "android-jni", test))]
impl WuiWindow {
    /// Retains the only window property consumed by Android's root activity and
    /// releases every other Rust-owned FFI handle.
    pub(crate) fn into_android_content(self) -> OwnedFfiHandle<WuiAnyView> {
        let Self {
            title,
            closable: _,
            resizable: _,
            frame,
            content,
            state,
            toolbar,
            style: _,
            background,
            min_size,
            max_size,
        } = self;

        let unused_handles = (
            OwnedFfiHandle::required(title, "WuiWindow.title"),
            OwnedFfiHandle::optional(frame),
            OwnedFfiHandle::required(state, "WuiWindow.state"),
            OwnedFfiHandle::optional(toolbar),
            background.into_android_owned_color(),
            OwnedFfiHandle::optional(min_size),
            OwnedFfiHandle::optional(max_size),
        );
        let content = OwnedFfiHandle::required(content, "WuiWindow.content");
        drop(unused_handles);
        content
    }

    /// Releases a window which Android cannot represent.
    pub(crate) fn dispose_android(self) {
        drop(self.into_android_content());
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
        // Apple consumes the toolbar as native window chrome. Other backends,
        // including Android, drop the Rust view without allocating an FFI handle.
        let toolbar = toolbar_into_ffi(self.toolbar);

        WuiWindow {
            title: self.title.into_ffi(),
            closable: self.closable,
            resizable: self.resizable,
            frame: self.frame.into_ffi(),
            content: content.into_ffi(),
            state: self.state.into_ffi(),
            toolbar,
            style: self.style.into(),
            background: self.background.into(),
            min_size: self.min_size.into_ffi(),
            max_size: self.max_size.into_ffi(),
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
    let env = unsafe { crate::borrow_ffi_mut(env) };

    let ffi_manager = FFIWindowManager {
        context: unsafe { ForeignCallbackContext::new(context, drop_context) },
        show_fn,
    };

    let manager = WindowManager::new(move |window| {
        ffi_manager.show(window);
    });

    env.insert(manager);
}
