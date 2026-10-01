//! Module defining the `Window` struct for UI windows.
//!
//! # Window Backgrounds
//!
//! Windows support solid color backgrounds. For blur effects, use `Material`:
//! `Material` is delegated to platform backends as `MaterialBackground` metadata
//! and is best-effort (quality and behavior may vary by platform).
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::window::{Window, WindowState};
//!
//! // Semi-transparent colored window
//! let tinted = Window::new("Tinted", binding::<WindowState>(WindowState::default()), || text!("Hello"))
//!     .background(Color::srgb(0, 0, 0).with_opacity(0.8));
//!
//! // Frosted glass window (opaque window + material blur on content)
//! let frosted = Window::new("Frosted", binding::<WindowState>(WindowState::default()), || text!("Hello"))
//!     .background(Material::Regular);
//! ```

use std::{fmt::Debug, rc::Rc};

use nami::{Binding, Computed, Signal, SignalExt as _, impl_constant, signal::IntoComputed};
use suiteki::Str;
use waterui_core::handler::{AnyViewBuilder, ViewBuilder};
use waterui_core::{AnyView, Dynamic, Environment, IgnorableMetadata, View};
use waterui_graphics::Color;
use waterui_layout::{Point, Rect, Size};

use crate::app::{application_identifier, application_name};
#[cfg(feature = "snackbar")]
use crate::snackbar::SnackbarManager;
use crate::{
    ViewExt,
    background::{Material, MaterialBackground},
    component::label::LabelDisplayMode,
    prelude::FullScreenOverlayManager,
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
    /// Explicit minimum content size the window can be resized down to.
    ///
    /// When `None` (the default), the backend derives the minimum from the
    /// content's own layout: each root-view axis is measured independently with
    /// a zero proposal, so the window can never be resized smaller than its
    /// content's minimum — the same negotiation every [`Layout`] container
    /// performs on its children, applied at the window boundary.
    ///
    /// Platform support: enforced by desktop backends (hydrolysis/winit,
    /// macOS `NSWindow.contentMinSize`/`contentMaxSize`). iOS/iPadOS has no
    /// per-window size control today (the Apple backend's multi-window support
    /// is macOS-only); Android's single-Activity model has no per-window
    /// runtime size limits; embedded (dew) displays are fixed-size — those
    /// targets ignore this.
    ///
    /// [`Layout`]: waterui_core::layout::Layout
    pub min_size: Option<Computed<Size>>,
    /// Explicit maximum content size the window can be resized up to.
    ///
    /// When `None` (the default), self-drawn desktop backends derive the maximum
    /// from the content's own layout by measuring each axis with an infinite
    /// proposal. A layout that accepts infinity leaves that axis unbounded.
    /// Native backends may require an explicit maximum. Same platform support
    /// notes as [`Self::min_size`].
    pub max_size: Option<Computed<Size>>,
    /// The identity the desktop groups this window under.
    ///
    /// Window-manager rules, desktop-file matching, startup notification and
    /// dock or taskbar grouping all key off this value: the X11 `WM_CLASS`
    /// (both its instance and class part) and the Wayland `xdg_toplevel`
    /// `app_id`.
    ///
    /// When `None` (the default), the window carries the application's own
    /// identifier, [`application_identifier`](crate::app::application_identifier);
    /// when that is empty too, the platform's default applies (the executable
    /// name under X11 and Wayland).
    ///
    /// Platform support: X11 and Wayland through the hydrolysis backend.
    /// macOS, iOS, Android and Windows identify an application by its bundle
    /// or package, not per window, and ignore this.
    pub app_id: Option<Str>,
    /// The per-window instance name inside the desktop identity.
    ///
    /// `app_id` names the application class — the X11 `WM_CLASS` class part
    /// and the Wayland `app_id` — while `instance_name` names this window's
    /// instance inside it, the `WM_CLASS` `res_name` window managers like
    /// i3/sway/awesome match on for per-window rules.
    ///
    /// When `None` (the default), the instance name is the window's
    /// resolved [`app_id`](Self::app_id).
    ///
    /// Platform support: X11 through the hydrolysis backend. Wayland carries
    /// only `app_id`, and the bundled platforms ignore this.
    pub instance_name: Option<Str>,
    /// Which monitor the window is placed on, and the frame it takes there.
    ///
    /// When `Some`, the backend resolves [`WindowPlacement::monitor`] each
    /// time the window is shown, calls [`WindowPlacement::place`] with the
    /// resolved [`Monitor`], and writes the returned rect into
    /// [`frame`](Self::frame) before the window becomes visible — placement
    /// therefore re-picks its monitor on every show, including after monitor
    /// changes and for a window that was closed and shown again.
    ///
    /// When `None` (the default), the window takes [`frame`](Self::frame)'s
    /// initial value, positioned where the platform puts it.
    ///
    /// Platform support: hydrolysis (winit) and GTK. Wayland compositors do
    /// not expose global pointer position or absolute window positioning, so
    /// only the size part of the returned rect applies there.
    pub placement: Option<WindowPlacement>,
    /// How showing and clicking the window affects keyboard focus and app
    /// activation. See [`Activation`] for the per-platform notes.
    pub activation: Activation,
    /// Where the window stacks relative to other applications' windows.
    ///
    /// Platform support: hydrolysis/winit, macOS (`NSWindow.level`), GTK
    /// where the compositor honours keep-above. Mobile platforms and
    /// embedded displays have no stacking between applications and ignore
    /// this.
    pub level: Computed<WindowLevel>,
    /// The window's request for the user's attention, if one is pending.
    ///
    /// Setting it asks the platform to draw the user to the window — a
    /// flashing taskbar entry, a bouncing dock icon, the window manager's
    /// demands-attention hint. The request lasts until the user focuses the
    /// window, at which point the backend sets it back to `None`; setting it to
    /// `None` withdraws it earlier.
    ///
    /// Platform support: hydrolysis/winit (X11, Wayland activation, Windows,
    /// macOS), macOS (`NSApp.requestUserAttention`). Others ignore it.
    pub attention: Binding<Option<UserAttention>>,
    /// The steps the window's content size moves in while the user resizes
    /// it, such as one character cell of a terminal.
    ///
    /// When `None` (the default), the window resizes continuously.
    ///
    /// Platform support: hydrolysis/winit (X11 `WM_NORMAL_HINTS`, macOS),
    /// macOS (`NSWindow.contentResizeIncrements`). Others ignore it.
    pub resize_increments: Option<Computed<Size>>,
}

/// A connected display, as the backend resolved it for a window's placement.
///
/// A backend hands this to [`WindowPlacement::place`] after resolving the
/// placement's [`MonitorSelector`]; it is a per-resolution snapshot, not a
/// live handle — monitor geometry read elsewhere stays authoritative in the
/// backend that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    /// Bounds in the global logical coordinate space window frames use.
    pub frame: Rect,
    /// `frame` minus what the desktop reserves (menu bar, dock, panels, taskbar).
    /// Equal to `frame` where the platform reports no work area.
    pub visible_frame: Rect,
    /// Physical pixels per logical point.
    pub scale_factor: f64,
    /// The platform's name for the display, when it reports one.
    pub name: Option<Str>,
}

/// Which monitor a window is placed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MonitorSelector {
    /// The platform's primary display (macOS: the one carrying the menu bar).
    #[default]
    Primary,
    /// The display under the pointer when the window is shown.
    Pointer,
    /// The display holding this application's focused window.
    /// Resolves as `Primary` when the application has no focused window.
    Focused,
}

/// Where a window is placed.
///
/// The backend resolves `monitor` each time the window is shown, calls `place`
/// with the result, and writes the returned rect into `Window::frame` before
/// the window becomes visible.
pub struct WindowPlacement {
    /// The monitor a backend resolves before calling `place`.
    pub monitor: MonitorSelector,
    /// Computes the window frame from the resolved monitor's geometry.
    ///
    /// Backends call it once per show, immediately before the window becomes
    /// visible; `place` answers in the same global logical space
    /// [`Monitor::frame`] is expressed in.
    pub place: Rc<dyn Fn(&Monitor) -> Rect>,
}

impl Debug for WindowPlacement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowPlacement")
            .field("monitor", &self.monitor)
            .finish_non_exhaustive()
    }
}

/// How showing and clicking a window affects keyboard focus and app activation.
///
/// This is for windows that must appear without stealing what the user is
/// doing — a drop-down terminal is the canonical case: it overlays focused
/// work and takes the keyboard only when its policy says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Activation {
    /// Showing the window activates the app and focuses the window.
    #[default]
    OnShow,
    /// Showing does not take focus; a click on the window does.
    OnClick,
    /// The window never takes keyboard focus or activates the app.
    Never,
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
    /// The window fills the screen's work area, keeping its chrome and
    /// the system's panels.
    Maximized,
    /// The window is maximized to fullscreen.
    Fullscreen,
}

/// Where a window stacks relative to other applications' windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowLevel {
    /// The window stacks with other windows as focus moves between them.
    #[default]
    Normal,
    /// The window stays above other applications' normal windows.
    AlwaysOnTop,
}

impl_constant!(WindowLevel);

/// How urgently a window asks for the user's attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserAttention {
    /// Something the user may want to look at: a finished task, a mention.
    Informational,
    /// Something the user must act on. Platforms that distinguish the two
    /// keep drawing attention until the window is focused (macOS bounces the
    /// dock icon repeatedly).
    Critical,
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
        #[cfg(feature = "snackbar")]
        let (snackbar_manager, snackbar_view) = SnackbarManager::new();
        let content = AnyViewBuilder::new(move || {
            let overlay_manager = overlay_manager.clone();
            #[cfg(feature = "snackbar")]
            let snackbar_manager = snackbar_manager.clone();
            let content = content
                .build()
                .overlay(overlay_view.clone())
                .with(overlay_manager.clone())
                .state(&overlay_manager);
            #[cfg(feature = "snackbar")]
            let content = content
                .overlay(snackbar_view.clone())
                .with(snackbar_manager.clone())
                .state(&snackbar_manager);
            AnyView::new(content)
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
            min_size: None,
            max_size: None,
            app_id: None,
            instance_name: None,
            placement: None,
            activation: Activation::default(),
            level: Computed::constant(WindowLevel::Normal),
            attention: Binding::container(None),
            resize_increments: None,
        }
    }

    /// Set an explicit minimum content size for the window.
    ///
    /// Without one, the backend derives the minimum from the content's layout
    /// by measuring each axis independently at a zero proposal. See
    /// [`Self::min_size`] for platform support notes.
    #[must_use]
    pub fn min_size(mut self, size: impl IntoComputed<Size>) -> Self {
        self.min_size = Some(size.into_computed());
        self
    }

    /// Set an explicit maximum content size for the window.
    ///
    /// See [`Self::max_size`] for platform support notes.
    #[must_use]
    pub fn max_size(mut self, size: impl IntoComputed<Size>) -> Self {
        self.max_size = Some(size.into_computed());
        self
    }

    /// Set the identity the desktop groups this window under — its X11
    /// `WM_CLASS` and Wayland `app_id`.
    ///
    /// See [`Self::app_id`] for the default and platform support notes.
    #[must_use]
    pub fn app_id(mut self, app_id: impl Into<Str>) -> Self {
        self.app_id = Some(app_id.into());
        self
    }

    /// Set the window's instance name inside the desktop identity — the X11
    /// `WM_CLASS` instance part.
    ///
    /// See [`Self::instance_name`] for the default and platform support notes.
    #[must_use]
    pub fn instance_name(mut self, instance_name: impl Into<Str>) -> Self {
        self.instance_name = Some(instance_name.into());
        self
    }

    /// Set where the window stacks relative to other applications' windows.
    ///
    /// See [`Self::level`] for platform support notes.
    #[must_use]
    pub fn level(mut self, level: impl IntoComputed<WindowLevel>) -> Self {
        self.level = level.into_computed();
        self
    }

    /// Set the steps the window's content size moves in while the user
    /// resizes it.
    ///
    /// See [`Self::resize_increments`] for platform support notes.
    #[must_use]
    pub fn resize_increments(mut self, increments: impl IntoComputed<Size>) -> Self {
        self.resize_increments = Some(increments.into_computed());
        self
    }

    /// Set whether the window is resizable.
    #[must_use]
    pub const fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Place the window on the monitor `monitor` resolves to, at the frame
    /// `place` computes from that monitor's [`Monitor`].
    ///
    /// See [`Self::placement`] for the resolution contract and platform
    /// support notes.
    #[must_use]
    pub fn placement(
        mut self,
        monitor: MonitorSelector,
        place: impl Fn(&Monitor) -> Rect + 'static,
    ) -> Self {
        self.placement = Some(WindowPlacement {
            monitor,
            place: Rc::new(place),
        });
        self
    }

    /// Set how showing and clicking the window affects keyboard focus and
    /// app activation. See [`Activation`].
    #[must_use]
    pub const fn activation(mut self, activation: Activation) -> Self {
        self.activation = activation;
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
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::window::{Window, WindowState};
    ///
    /// // Semi-transparent colored window
    /// let tinted = Window::new("Tinted", binding::<WindowState>(WindowState::default()), || text!("Hello"))
    ///     .background(Color::srgb(0, 0, 0).with_opacity(0.8));
    ///
    /// // Frosted glass window (opaque + material blur on content)
    /// let frosted = Window::new("Frosted", binding::<WindowState>(WindowState::default()), || text!("Hello"))
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

    /// The title to put on the window, which is what a backend should show.
    ///
    /// A window that declares no title of its own — an empty one — is shown
    /// under the application's name, so that an application which never says
    /// what its window is called is still named after itself rather than after
    /// the framework. This is resolved here rather than in each backend so that
    /// every one of them titles a window the same way.
    ///
    /// The name is empty when nothing told the application what it is called,
    /// which leaves the decision to the platform: a bundle knows its own name
    /// and keeps whatever it had.
    #[must_use]
    pub fn display_title(&self) -> Computed<Str> {
        let application = application_name();
        self.title
            .map(move |declared| {
                if declared.is_empty() {
                    application.clone()
                } else {
                    declared
                }
            })
            .into_computed()
    }

    /// The identity to give the window, which is what a backend should set.
    ///
    /// A window that declares no [`app_id`](Self::app_id) of its own carries
    /// the application's identifier, resolved here so that every backend
    /// identifies a window the same way. The result is empty when neither was
    /// given, which leaves the decision to the platform.
    #[must_use]
    pub fn display_app_id(&self) -> Str {
        self.app_id.clone().unwrap_or_else(application_identifier)
    }

    /// The window's resolved instance name inside the desktop identity.
    ///
    /// A window that declares no [`instance_name`](Self::instance_name) of its
    /// own carries its resolved [`app_id`](Self::display_app_id), which keeps
    /// the platform default when that is empty too.
    #[must_use]
    pub fn display_instance_name(&self) -> Str {
        self.instance_name
            .clone()
            .unwrap_or_else(|| self.display_app_id())
    }

    /// Get a handle to control the window after showing it.
    #[must_use]
    pub fn handle(&self) -> WindowHandle {
        WindowHandle {
            frame: self.frame.clone(),
            state: self.state.clone(),
            attention: self.attention.clone(),
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

/// Owns the explicit presentation state for a conditionally created window.
///
/// Keep this value at the same semantic level as the window's [`WindowState`]
/// binding. Its retained `presented` binding prevents unrelated state changes
/// such as Normal → Minimized from creating duplicate native windows.
#[derive(Debug, Clone)]
pub struct WindowPresentation {
    state: Binding<WindowState>,
    presented: Binding<bool>,
}

impl WindowPresentation {
    /// Creates presentation state for a window controlled by `state`.
    #[must_use]
    pub fn new(state: &Binding<WindowState>) -> Self {
        Self {
            state: state.clone(),
            presented: Binding::container(false),
        }
    }

    /// Returns the window-state binding controlled by this presentation.
    #[must_use]
    pub fn state(&self) -> Binding<WindowState> {
        self.state.clone()
    }
}

/// Shows a window only once per "open" cycle.
///
/// The window is shown when the presentation's state transitions from
/// [`WindowState::Closed`] to any other state. Closing the window resets the
/// retained presentation binding so a subsequent open creates a new native
/// window.
///
/// The returned view is invisible and must be placed in the tree, or the
/// window is never presented.
pub fn conditional_window<F>(presentation: &WindowPresentation, creator: F) -> impl View + use<F>
where
    F: Fn(Binding<WindowState>) -> Window + 'static,
{
    let state = presentation.state.clone();
    let presented = presentation.presented.clone();

    Dynamic::watch(state.clone(), move |s| {
        if s == WindowState::Closed {
            presented.set(false);
            AnyView::new(())
        } else if !presented.snapshot() {
            presented.set(true);
            AnyView::new(creator(state.clone()))
        } else {
            AnyView::new(())
        }
    })
}

/// A handle to control a window after it has been shown.
#[derive(Debug, Clone)]
pub struct WindowHandle {
    frame: Binding<Rect>,
    state: Binding<WindowState>,
    attention: Binding<Option<UserAttention>>,
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

    /// Maximize the window to the screen's work area.
    pub fn maximize(&self) {
        self.state.set(WindowState::Maximized);
    }

    /// Make the window fullscreen.
    pub fn fullscreen(&self) {
        self.state.set(WindowState::Fullscreen);
    }

    /// Restore the window to its normal state.
    pub fn restore(&self) {
        self.state.set(WindowState::Normal);
    }

    /// Ask the platform to draw the user's attention to the window, until
    /// the user focuses it.
    pub fn request_attention(&self, urgency: UserAttention) {
        self.attention.set(Some(urgency));
    }

    /// Withdraw a pending request for the user's attention.
    pub fn cancel_attention(&self) {
        self.attention.set(None);
    }

    /// Set the frame of the window.
    pub fn set_frame(&self, frame: Rect) {
        self.frame.set(frame);
    }
}
