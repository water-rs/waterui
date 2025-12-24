use core::ptr::null_mut;

use waterui::Str;
use waterui::background::Material;
use waterui::window::{Window, WindowBackground, WindowState, WindowStyle};
use waterui_layout::Rect;

use crate::{
    IntoFFI, WuiAnyView, color::WuiColor,
    reactive::{WuiBinding, WuiComputed},
};

/// FFI-safe representation of a material blur style.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum WuiMaterial {
    UltraThin = 0,
    Thin = 1,
    Regular = 2,
    Thick = 3,
    UltraThick = 4,
}

impl IntoFFI for Material {
    type FFI = WuiMaterial;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Material::UltraThin => WuiMaterial::UltraThin,
            Material::Thin => WuiMaterial::Thin,
            Material::Regular => WuiMaterial::Regular,
            Material::Thick => WuiMaterial::Thick,
            Material::UltraThick => WuiMaterial::UltraThick,
        }
    }
}

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

/// FFI-compatible representation of [`WindowBackground`].
#[repr(C)]
pub enum WuiWindowBackground {
    /// Opaque system default background.
    Opaque,
    /// Fully transparent window (no background).
    Transparent,
    /// Solid color background (can be semi-transparent via alpha).
    /// Native must resolve the color using the environment.
    Color { color: *mut WuiColor },
    /// Material blur effect.
    Material { material: WuiMaterial },
}

impl From<WindowBackground> for WuiWindowBackground {
    fn from(bg: WindowBackground) -> Self {
        match bg {
            WindowBackground::Opaque => Self::Opaque,
            WindowBackground::Transparent => Self::Transparent,
            WindowBackground::Color(color) => Self::Color {
                color: color.into_ffi(),
            },
            WindowBackground::Material(material) => Self::Material {
                material: material.into_ffi(),
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
}

impl IntoFFI for Window {
    type FFI = WuiWindow;

    fn into_ffi(self) -> Self::FFI {
        WuiWindow {
            title: self.title.into_ffi(),
            closable: self.closable,
            resizable: self.resizable,
            frame: self.frame.into_ffi(),
            content: self.content.into_ffi(),
            state: self.state.into_ffi(),
            toolbar: self.toolbar.map_or(null_mut(), IntoFFI::into_ffi),
            style: self.style.into(),
            background: self.background.into(),
        }
    }
}
