//! Module defining the `Window` struct for UI windows.
//!
//! # Window Backgrounds
//!
//! Windows support solid color backgrounds. For blur effects, use `Material`:
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//! use waterui::window::Window;
//!
//! // Semi-transparent colored window
//! Window::new("Tinted", content)
//!     .background(Color::black().with_alpha(0.8));
//!
//! // Frosted glass window (opaque window + material blur on content)
//! Window::new("Frosted", content)
//!     .background(Material::Regular);
//! ```

use std::{fmt::Debug, rc::Rc};

use nami::{Binding, Computed, impl_constant, signal::IntoComputed};
use waterui_core::{AnyView, Environment, IgnorableMetadata, View};
use waterui_graphics::Color;
use waterui_layout::{Point, Rect, Size, stack::zstack};
use waterui_str::Str;

use crate::{ViewExt, background::{Material, MaterialBackground}, prelude::FullScreenOverlayManager, snackbar::SnackbarManager};

/// Represents a window in the UI.
#[derive(Debug)]
pub struct Window {
    /// The title of the window.
    ///
    /// Notice that it may not be displayed on all platforms.
    pub title: Computed<Str>,
    /// Whether the window is closable.
    ///
    /// Notice that it may not be supported on all platforms.
    pub closable: bool,
    /// Whether the window is resizable.
    ///
    /// Notice that it may not be supported on all platforms.
    pub resizable: bool,
    /// The frame of the window.
    ///
    /// Notice that it may not be supported on all platforms.
    pub frame: Binding<Rect>,
    /// The content of the window.
    pub content: AnyView,
    /// The current state of the window.
    pub state: Binding<WindowState>,
    /// Optional toolbar content for the window.
    ///
    /// Notice that it may not be supported on all platforms.
    pub toolbar: Option<AnyView>,
    /// The visual style of the window.
    ///
    /// Notice that it may not be supported on all platforms.
    pub style: WindowStyle,
    /// The background style of the window.
    ///
    /// Use this to create transparent or frosted glass windows.
    /// Notice that it may not be supported on all platforms.
    pub background: WindowBackground,
}

/// The state of a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowState {
    /// The window is in its normal state.
    #[default]
    Normal,
    /// The window is closed.
    Closed,
    /// The window is minimized.
    Minimized,
    /// The window is maximized to fullscreen.
    Fullscreen,
}

/// The visual style of a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowStyle {
    /// Standard window with title bar and controls.
    #[default]
    Titled,
    /// Borderless window without title bar.
    Borderless,
    /// Window where content extends into the title bar area.
    ///
    /// On macOS, this corresponds to `NSWindow.StyleMask.fullSizeContentView`.
    FullSizeContentView,
}

/// The background style of a window (FFI level).
///
/// This only supports opaque or solid color backgrounds.
/// For blur effects, use `Material` which wraps content with `MaterialBackground` metadata.
///
/// # Platform Support
///
/// - **macOS/iOS**: Supports both opaque and colored backgrounds.
/// - **Android**: Supports colored backgrounds via `Window.setBackgroundDrawable()`.
#[derive(Debug, Clone, Default)]
pub enum WindowBackground {
    /// Opaque system default background.
    #[default]
    Opaque,
    /// Solid color background (can be semi-transparent via alpha).
    Color(Color),
}

/// Input type for `Window::background()` method.
///
/// Allows setting window background via `Color` or `Material`.
/// When `Material` is used, the window becomes opaque and the content
/// is wrapped with a `MaterialBackground` metadata for native blur effects.
#[derive(Debug)]
pub enum WindowBackgroundInput {
    /// A solid color background.
    Color(Color),
    /// A material blur effect (wraps content, window stays opaque).
    Material(Material),
}

impl From<Color> for WindowBackgroundInput {
    fn from(color: Color) -> Self {
        Self::Color(color)
    }
}

impl From<Material> for WindowBackgroundInput {
    fn from(material: Material) -> Self {
        Self::Material(material)
    }
}

/// Manages the display of windows.
#[derive(Clone)]
pub struct WindowManager(Rc<dyn Fn(Window)>);

impl Debug for WindowManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowManager").finish()
    }
}

impl WindowManager {
    /// Create a new `WindowManager` with the specified show function.
    pub fn new<F: 'static + Fn(Window)>(show: F) -> Self {
        Self(Rc::new(show))
    }

    /// Show a window using the window manager.
    pub fn show(&self, window: Window) {
        (self.0)(window);
    }
}

impl_constant!(WindowState);
impl_constant!(WindowStyle);

impl Window {
    /// Create a new window instance with the specified title and content.
    ///
    /// Notice that would not show this window immediately
    #[must_use]
    pub fn new(title: impl IntoComputed<Str>, content: impl View) -> Self {
        let default_frame = Rect::new(Point::zero(), Size::new(800.0, 600.0));
        let (overlay_manager, overlay_view) = FullScreenOverlayManager::new();
        let (snackbar_manager, snackbar_view) = SnackbarManager::new();

        Self {
            title: title.into_computed(),
            closable: true,
            resizable: true,
            frame: Binding::container(default_frame),
            content: AnyView::new(
                zstack((content, overlay_view, snackbar_view))
                    .with(overlay_manager)
                    .with(snackbar_manager),
            ),
            state: Binding::default(),
            toolbar: None,
            style: WindowStyle::default(),
            background: WindowBackground::default(),
        }
    }

    /// Set whether the window is resizable.
    #[must_use]
    pub const fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Set the toolbar content for the window.
    #[must_use]
    pub fn toolbar(mut self, toolbar: impl View) -> Self {
        self.toolbar = Some(AnyView::new(toolbar));
        self
    }

    /// Set the visual style of the window.
    #[must_use]
    pub const fn style(mut self, style: WindowStyle) -> Self {
        self.style = style;
        self
    }

    /// Set the background style of the window.
    ///
    /// Accepts either a `Color` for solid backgrounds or a `Material` for blur effects.
    /// When using `Material`, the window stays opaque and the content is wrapped with
    /// a native blur effect on supported platforms (Apple).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    /// use waterui::window::Window;
    ///
    /// // Semi-transparent colored window
    /// Window::new("Tinted", content)
    ///     .background(Color::black().with_alpha(0.8));
    ///
    /// // Frosted glass window (opaque + material blur on content)
    /// Window::new("Frosted", content)
    ///     .background(Material::Regular);
    /// ```
    #[must_use]
    pub fn background(mut self, background: impl Into<WindowBackgroundInput>) -> Self {
        match background.into() {
            WindowBackgroundInput::Color(color) => {
                self.background = WindowBackground::Color(color);
            }
            WindowBackgroundInput::Material(material) => {
                // Keep window opaque, wrap content with MaterialBackground metadata
                self.background = WindowBackground::Opaque;
                self.content = AnyView::new(IgnorableMetadata::new(
                    self.content,
                    MaterialBackground(material),
                ));
            }
        }
        self
    }

    /// Set the state binding for the window.
    ///
    /// Use this to connect an external state binding to the window,
    /// allowing you to track when the window is closed by the native close button.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    /// use waterui::window::{Window, WindowState};
    ///
    /// let state = binding(WindowState::Normal);
    ///
    /// // Window with external state binding
    /// Window::new("My Window", content)
    ///     .with_state(state.clone());
    ///
    /// // Watch for close
    /// watch(state, |s| {
    ///     if s == WindowState::Closed {
    ///         // Window was closed
    ///     }
    /// });
    /// ```
    #[must_use]
    pub fn with_state(mut self, state: Binding<WindowState>) -> Self {
        self.state = state;
        self
    }

    /// Set the title of the window.
    #[must_use]
    pub fn title(mut self, title: impl IntoComputed<Str>) -> Self {
        self.title = title.into_computed();
        self
    }

    /// Get a handle to control the window after showing it.
    #[must_use]
    pub fn handle(&self) -> WindowHandle {
        WindowHandle {
            frame: self.frame.clone(),
            state: self.state.clone(),
        }
    }

    /// Show the window on screen.
    ///
    /// # Panics
    ///
    /// Panics if `WindowManager` is not found in the environment.
    pub fn show(self, env: &Environment) {
        env.get::<WindowManager>()
            .expect("WindowManager not found in environment")
            .show(self);
    }
}

// Implement View for Window to allow reactive window display.
// When a Window is rendered as a View, it shows itself via WindowManager.
impl View for Window {
    fn body(self, env: &Environment) -> impl View {
        self.show(env);
        // Return empty view - the window is shown separately
        ()
    }
}


/// A handle to control a window after it has been shown.
#[derive(Debug, Clone)]
pub struct WindowHandle {
    frame: Binding<Rect>,
    state: Binding<WindowState>,
}

impl WindowHandle {
    /// Close the window.
    pub fn close(self) {
        self.state.set(WindowState::Closed);
    }

    /// Minimize the window.
    pub fn minimize(&self) {
        self.state.set(WindowState::Minimized);
    }

    /// Maximize the window to fullscreen.
    pub fn fullscreen(&self) {
        self.state.set(WindowState::Fullscreen);
    }

    /// Restore the window to its normal state.
    pub fn restore(&self) {
        self.state.set(WindowState::Normal);
    }

    /// Set the frame of the window.
    pub fn set_frame(&self, frame: Rect) {
        self.frame.set(frame);
    }
}
