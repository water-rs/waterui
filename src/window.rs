//! Module defining the `Window` struct for UI windows.
//!
//! # Window Backgrounds
//!
//! Windows support solid color backgrounds. For blur effects, use `Material`:
//! `Material` is delegated to platform backends as `MaterialBackground` metadata
//! and is best-effort (quality and behavior may vary by platform).
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//! use waterui::window::Window;
//!
//! // Semi-transparent colored window
//! Window::new("Tinted", content)
//!     .background(Color::srgb(0, 0, 0).with_opacity(0.8));
//!
//! // Frosted glass window (opaque window + material blur on content)
//! Window::new("Frosted", content)
//!     .background(Material::Regular);
//! ```

use std::{fmt::Debug, rc::Rc};

use nami::{Binding, Computed, SignalExt, impl_constant, signal::IntoComputed};
use waterui_core::handler::{AnyViewBuilder, ViewBuilder};
use waterui_core::{AnyView, Environment, IgnorableMetadata, View};
use waterui_graphics::Color;
use waterui_layout::{Point, Rect, Size, stack::zstack};
use waterui_str::Str;

use crate::{
    ViewExt,
    background::{Material, MaterialBackground},
    component::label::LabelDisplayMode,
    prelude::FullScreenOverlayManager,
    snackbar::SnackbarManager,
};

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
    /// The root view builder for the window content.
    pub content: AnyViewBuilder<AnyView>,
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
///
/// `Default::default()` returns [`Self::Closed`]: a freshly initialized
/// state binding represents a window that has not yet been shown. Setting
/// the binding to [`Self::Normal`] is what triggers the window to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowState {
    /// The window is in its normal state.
    Normal,
    /// The window is closed.
    #[default]
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
/// - **Linux (GTK)**: Supports colored backgrounds via window CSS/background styling.
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
    /// Create a new window with the specified title, state binding, and content.
    ///
    /// `state` is required: every example calls `.with_state(...)` on every
    /// window in practice, so the parameter is positional. Use
    /// `binding::<WindowState>(default())` (which is [`WindowState::Closed`])
    /// to start the window closed; flip to [`WindowState::Normal`] to open it.
    ///
    /// Note: this does not show the window immediately. It is shown via
    /// [`Self::show`] or by being conditionally rendered with
    /// [`conditional_window`].
    #[must_use]
    pub fn new(
        title: impl IntoComputed<Str>,
        state: Binding<WindowState>,
        content: impl ViewBuilder,
    ) -> Self {
        let default_frame = Rect::new(Point::zero(), Size::new(800.0, 600.0));
        let (overlay_manager, overlay_view) = FullScreenOverlayManager::new();
        let (snackbar_manager, snackbar_view) = SnackbarManager::new();
        let content = AnyViewBuilder::new(move || {
            AnyView::new(
                zstack((content.build(), overlay_view.clone(), snackbar_view.clone()))
                    .with(overlay_manager.clone())
                    .with(snackbar_manager.clone()),
            )
        });

        Self {
            title: title.into_computed(),
            closable: true,
            resizable: true,
            frame: Binding::container(default_frame),
            content,
            state,
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
    ///
    /// Toolbar subtrees automatically install [`LabelDisplayMode::IconOnly`] so
    /// semantic labels adapt to compact window chrome while preserving their
    /// accessibility text. Individual labels can opt back into
    /// [`LabelDisplayMode::TitleOnly`] or [`LabelDisplayMode::TitleAndIcon`].
    #[must_use]
    pub fn toolbar(mut self, toolbar: impl View) -> Self {
        self.toolbar = Some(AnyView::new(toolbar.install(LabelDisplayMode::IconOnly)));
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
    /// `MaterialBackground` metadata handled by the native backend on a best-effort basis.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    /// use waterui::window::Window;
    ///
    /// // Semi-transparent colored window
    /// Window::new("Tinted", content)
    ///     .background(Color::srgb(0, 0, 0).with_opacity(0.8));
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
                let content = self.content;
                self.content = AnyViewBuilder::new(move || {
                    AnyView::new(IgnorableMetadata::new(
                        content.build(),
                        MaterialBackground(material),
                    ))
                });
            }
        }
        self
    }

    /// Builds the current window content tree.
    pub fn build_content(&self) -> AnyView {
        self.content.build()
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
    }
}

/// Shows a window only once per "open" cycle.
///
/// This helper avoids accidentally creating multiple native windows when the bound
/// [`WindowState`] changes (e.g. Normal → Minimized), which would re-run `Window`'s
/// `View::body()` and call `show()` again.
///
/// The window is shown when `state` transitions from `Closed` to any other state.
/// When `state` becomes `Closed` again (either programmatically or via the native
/// close button updating the binding), the helper resets and a subsequent open will
/// create a new window.
pub fn conditional_window<F>(state: &Binding<WindowState>, creator: F) -> impl View + use<F>
where
    F: Fn(Binding<WindowState>) -> Window + 'static,
{
    use core::cell::Cell;
    use std::rc::Rc;

    let shown = Rc::new(Cell::new(false));
    let creator = Rc::new(creator);
    let state = state.clone();

    state
        .clone()
        .map(move |s| {
            if s == WindowState::Closed {
                shown.set(false);
                AnyView::new(())
            } else if !shown.get() {
                shown.set(true);
                AnyView::new((creator.as_ref())(state.clone()))
            } else {
                AnyView::new(())
            }
        })
        .computed()
}

/// A handle to control a window after it has been shown.
#[derive(Debug, Clone)]
pub struct WindowHandle {
    frame: Binding<Rect>,
    state: Binding<WindowState>,
}

impl WindowHandle {
    /// Close the window.
    pub fn close(&self) {
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
