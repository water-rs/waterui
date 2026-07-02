use core::ptr::null_mut;

use waterui::Str;
use waterui::window::{Window, WindowBackground, WindowManager, WindowState, WindowStyle};
use waterui_layout::{Rect, Size};

use crate::{
    IntoFFI, IntoRust, WuiAnyView, WuiEnv,
    color::WuiColor,
    ffi_binding,
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

// Generate FFI binding functions for WindowState
ffi_binding!(WindowState, WuiWindowState, window_state);

// JNI primitive support for WindowState (enum treated as jint)
#[cfg(feature = "android-jni")]
impl crate::jni::JniPrimitive for WindowState {
    type Jni = jni::sys::jint;
    fn to_jni(self) -> Self::Jni {
        WuiWindowState::from(self) as Self::Jni
    }
    fn from_jni(val: Self::Jni) -> Self {
        match val {
            x if x == WuiWindowState::Normal as Self::Jni => WindowState::Normal,
            x if x == WuiWindowState::Closed as Self::Jni => WindowState::Closed,
            x if x == WuiWindowState::Minimized as Self::Jni => WindowState::Minimized,
            x if x == WuiWindowState::Fullscreen as Self::Jni => WindowState::Fullscreen,
            _ => panic!("invalid WindowState JNI value: {val}"),
        }
    }
}

// Generate JNI read/set for WindowState binding
crate::jni_binding_primitive!(WindowState, window_state);

use crate::reactive::{WuiWatcher, WuiWatcherMetadata};

/// Creates a watcher for WindowState from native callbacks.
///
/// # Safety
/// All function pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_new_watcher_window_state(
    data: *mut (),
    call: unsafe extern "C" fn(*mut (), WuiWindowState, *mut WuiWatcherMetadata),
    drop: unsafe extern "C" fn(*mut ()),
) -> *mut WuiWatcher<WindowState> {
    use alloc::boxed::Box;
    let watcher = unsafe { WuiWatcher::new(data, call, drop) };
    Box::into_raw(Box::new(watcher))
}

/// FFI-compatible representation of [`WindowBackground`].
///
/// Only supports Opaque and Color. Material blur effects are handled
/// via `MaterialBackground` metadata on the window content.
#[repr(C)]
pub enum WuiWindowBackground {
    /// Opaque system default background.
    Opaque,
    /// Solid color background (can be semi-transparent via alpha).
    /// Native must resolve the color using the environment.
    Color { color: *mut WuiColor },
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

impl IntoFFI for Window {
    type FFI = WuiWindow;

    fn into_ffi(self) -> Self::FFI {
        let content = self.build_content();
        // NOTE: Toolbars transfer ownership of an `AnyView` pointer to native.
        // Only enable this on backends that actually consume (or drop) the pointer.
        let toolbar = if cfg!(any(target_vendor = "apple", target_os = "android")) {
            self.toolbar.into_ffi()
        } else {
            null_mut()
        };

        WuiWindow {
            title: self.title.into_ffi(),
            closable: self.closable,
            resizable: self.resizable,
            // Frame bindings are currently not consumed by native backends, and leaking
            // them across FFI is easy to do accidentally. Until there is end-to-end support,
            // do not transfer ownership.
            frame: null_mut(),
            content: content.into_ffi(),
            state: self.state.into_ffi(),
            // Toolbars are currently not rendered by native backends. Avoid leaking AnyView
            // pointers across FFI by not transferring ownership.
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
/// Native code should use the global environment to render the window content.
///
/// # Parameters
/// - `WuiWindow`: The window configuration to show
pub type WindowShowFn = unsafe extern "C" fn(WuiWindow);

/// FFI-compatible WindowManager implementation.
struct FFIWindowManager {
    show_fn: WindowShowFn,
}

// Safety: The function pointer is thread-safe to send as it points to static code.
unsafe impl Send for FFIWindowManager {}
unsafe impl Sync for FFIWindowManager {}

impl FFIWindowManager {
    fn show(&self, window: Window) {
        let ffi_window = window.into_ffi();
        unsafe {
            (self.show_fn)(ffi_window);
        }
    }
}

/// Installs a WindowManager into the environment from a native function pointer.
///
/// Native backends call this during initialization to register their window
/// management implementation. When `Window` views are rendered, the provided
/// callback will be invoked to create and display native windows.
///
/// Note: Native code should use its global environment to render window content,
/// as the environment cannot be safely passed through the callback.
///
/// # Safety
///
/// The caller must ensure that:
/// - `env` is a valid pointer to a `WuiEnv`
/// - `show_fn` is a valid function pointer that can handle `WuiWindow` and create native windows
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_install_window_manager(
    env: *mut WuiEnv,
    show_fn: WindowShowFn,
) {
    let env_ref = unsafe { &mut *env };

    let ffi_manager = FFIWindowManager { show_fn };

    let manager = WindowManager::new(move |window| {
        ffi_manager.show(window);
    });

    env_ref.insert(manager);
}
