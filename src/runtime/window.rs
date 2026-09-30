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
use waterui_core::{AnyView, Dynamic, Environment, IgnorableMetadata, View, flatten_signal};
use waterui_graphics::{Color, color::ResolvedColor, peniko::ImageData};
use waterui_layout::{Point, Rect, Size};

use crate::app::{application_identifier, application_name};
#[cfg(feature = "snackbar")]
use crate::snackbar::SnackbarManager;
use crate::{
    ViewExt,
    background::{Material, MaterialBackground},
    component::label::LabelDisplayMode,
    prelude::FullScreenOverlayManager,
    theme::color::Background,
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
    /// Reactive: backends observe the binding and re-apply the style when it
    /// changes after the window is shown, so an app can toggle decorations at
    /// runtime through [`WindowHandle::set_style`] or its own binding.
    ///
    /// Notice that it may not be supported on all platforms.
    pub style: Binding<WindowStyle>,
    /// The background style of the window.
    ///
    /// Use this to create transparent or frosted glass windows. Reactive:
    /// backends re-apply the background when the binding changes after the
    /// window is shown, including a switch between [`WindowBackground::Opaque`]
    /// and a translucent [`WindowBackground::Color`]. Backends paint what
    /// [`Window::resolved_background`] resolves it to.
    ///
    /// Notice that it may not be supported on all platforms.
    pub background: Binding<WindowBackground>,
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
    /// The window's own icon, or `None` for the application icon.
    ///
    /// Reactive: backends apply a change to a shown window, so an app can
    /// switch icons at runtime (with its theme, say). `None` — the default —
    /// keeps the icon the `water` CLI stages for the application.
    ///
    /// Platform support: hydrolysis on X11 and Windows (winit's
    /// `set_window_icon`), GTK through the toplevel's icon list, and `WinUI`
    /// through `AppWindow`'s icon. `AppKit`, `UIKit` and Android identify an
    /// application by one icon and have no per-window icon, and Wayland's
    /// winit toplevel carries none: this is unsupported there, and those
    /// platforms keep showing the application icon.
    pub icon: Binding<Option<ImageData>>,
}

/// Conversion into the reactive icon a [`Window`] carries: decoded pixels for
/// a fixed icon, or a binding the app keeps to change the icon later (`None`
/// keeps the application icon).
pub trait IntoWindowIcon {
    /// Converts into the window's icon binding.
    fn into_window_icon(self) -> Binding<Option<ImageData>>;
}

impl IntoWindowIcon for ImageData {
    fn into_window_icon(self) -> Binding<Option<ImageData>> {
        Binding::container(Some(self))
    }
}

impl IntoWindowIcon for Binding<Option<ImageData>> {
    fn into_window_icon(self) -> Binding<Option<ImageData>> {
        self
    }
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

/// The background style of a window.
///
/// This only supports opaque or solid color backgrounds.
/// For blur effects, use `Material` which wraps content with `MaterialBackground` metadata.
///
/// # Platform Support
///
/// - **macOS**: `NSWindow.backgroundColor` and `isOpaque`.
/// - **Android**: `Window.setBackgroundDrawable()`.
/// - **Linux (GTK)**: window CSS background.
/// - **Windows (`WinUI`)**: the content root's background brush.
/// - **Hydrolysis**: the surface clear colour and composite alpha mode.
#[derive(Debug, Clone, Default)]
pub enum WindowBackground {
    /// Opaque background in the theme's [`Background`] colour.
    #[default]
    Opaque,
    /// Solid color background (can be semi-transparent via alpha).
    Color(Color),
}

impl WindowBackground {
    /// The colour this background paints: the theme's [`Background`] colour
    /// for [`Self::Opaque`], the declared colour otherwise.
    #[must_use]
    pub fn color(&self) -> Color {
        match self {
            Self::Opaque => Color::new(Background),
            Self::Color(color) => color.clone(),
        }
    }
}

impl From<Color> for WindowBackground {
    fn from(color: Color) -> Self {
        Self::Color(color)
    }
}

impl From<WindowBackground> for Binding<WindowBackground> {
    fn from(background: WindowBackground) -> Self {
        Self::container(background)
    }
}

/// Resolves a reactive window background to the concrete colour a backend
/// paints behind the window's content.
///
/// The result follows both a change of the background itself — including a
/// switch between [`WindowBackground::Opaque`] and [`WindowBackground::Color`]
/// — and a change of the colour it currently resolves to, such as a theme
/// switch. A colour whose opacity is below one asks for a translucent window.
#[must_use]
pub fn resolve_background<S>(background: &S, env: &Environment) -> Computed<ResolvedColor>
where
    S: Signal<Output = WindowBackground>,
{
    let env = env.clone();
    flatten_signal(background.map(move |background| background.color().resolve(&env)))
}

/// Input type for `Window::background()` method.
///
/// Allows setting window background via a `Color`, a [`WindowBackground`], a
/// `Binding<WindowBackground>` the app keeps to change it later, or a
/// `Material`. When `Material` is used, the window becomes opaque and the
/// content is wrapped with a `MaterialBackground` metadata for native blur
/// effects.
#[derive(Debug)]
pub enum WindowBackgroundInput {
    /// A reactive background: a fixed colour or `Opaque`, or a binding.
    Background(Binding<WindowBackground>),
    /// A material blur effect (wraps content, window stays opaque).
    Material(Material),
}

impl From<WindowBackground> for WindowBackgroundInput {
    fn from(background: WindowBackground) -> Self {
        Self::Background(background.into())
    }
}

impl From<Binding<WindowBackground>> for WindowBackgroundInput {
    fn from(background: Binding<WindowBackground>) -> Self {
        Self::Background(background)
    }
}

impl From<Color> for WindowBackgroundInput {
    fn from(color: Color) -> Self {
        WindowBackground::Color(color).into()
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

impl From<WindowStyle> for Binding<WindowStyle> {
    fn from(style: WindowStyle) -> Self {
        Self::container(style)
    }
}

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
            style: Binding::container(WindowStyle::default()),
            background: Binding::container(WindowBackground::default()),
            min_size: None,
            max_size: None,
            app_id: None,
            instance_name: None,
            placement: None,
            activation: Activation::default(),
            icon: Binding::container(None),
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

    /// Set the window's own icon: decoded pixels, or a
    /// `Binding<Option<ImageData>>` to change it after the window is shown.
    ///
    /// See [`Self::icon`] for the default and platform support notes.
    #[must_use]
    pub fn icon(mut self, icon: impl IntoWindowIcon) -> Self {
        self.icon = icon.into_window_icon();
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
    ///
    /// Takes a [`WindowStyle`] for a fixed style or a `Binding<WindowStyle>`
    /// the app keeps to change the style after the window is shown.
    #[must_use]
    pub fn style(mut self, style: impl Into<Binding<WindowStyle>>) -> Self {
        self.style = style.into();
        self
    }

    /// Set the background style of the window.
    ///
    /// Accepts a `Color` for solid backgrounds, a [`WindowBackground`] or a
    /// `Binding<WindowBackground>` to change the background after the window
    /// is shown, or a `Material` for blur effects. When using `Material`, the
    /// window stays opaque and the content is wrapped with
    /// `MaterialBackground` metadata handled by the native backend on a
    /// best-effort basis.
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
            WindowBackgroundInput::Background(background) => {
                self.background = background;
            }
            WindowBackgroundInput::Material(material) => {
                // Keep window opaque, wrap content with MaterialBackground metadata
                self.background = Binding::container(WindowBackground::Opaque);
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

    /// The colour a backend paints behind the window's content, following the
    /// reactive [`Self::background`]. See [`resolve_background`].
    #[must_use]
    pub fn resolved_background(&self, env: &Environment) -> Computed<ResolvedColor> {
        resolve_background(&self.background, env)
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
            style: self.style.clone(),
            background: self.background.clone(),
            icon: self.icon.clone(),
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
    style: Binding<WindowStyle>,
    background: Binding<WindowBackground>,
    icon: Binding<Option<ImageData>>,
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

    /// Set the visual style of the window; the backend re-applies it to the
    /// shown window.
    pub fn set_style(&self, style: WindowStyle) {
        self.style.set(style);
    }

    /// Set the background of the window; the backend re-applies it to the
    /// shown window.
    pub fn set_background(&self, background: impl Into<WindowBackground>) {
        self.background.set(background.into());
    }

    /// Set the window's own icon, or `None` for the application icon; the
    /// backend re-applies it to the shown window. See [`Window::icon`] for
    /// platform support.
    pub fn set_icon(&self, icon: Option<ImageData>) {
        self.icon.set(icon);
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use nami::{Binding, Signal};
    use waterui_core::Environment;
    use waterui_graphics::Color;

    use super::{WindowBackground, resolve_background};

    /// The resolved background follows a replacement of the background
    /// itself, not only a change of the colour it started with.
    #[test]
    fn resolved_background_follows_a_replaced_background() {
        let env = Environment::new();
        let background = Binding::container(WindowBackground::Color(Color::srgb(255, 0, 0)));
        let resolved = resolve_background(&background, &env);
        assert!(resolved.snapshot().red > 0.99);

        let seen = Rc::new(RefCell::new(Vec::new()));
        let _guard = resolved.watch({
            let seen = seen.clone();
            move |ctx| seen.borrow_mut().push(ctx.into_value())
        });
        background.set(WindowBackground::Color(
            Color::srgb(0, 0, 255).with_opacity(0.5),
        ));

        let seen = seen.borrow();
        let last = seen.last().expect("the replaced background was delivered");
        assert!(last.blue > 0.99 && last.red < 0.01);
        assert!((last.opacity - 0.5).abs() < 1e-6);
        assert!((resolved.snapshot().opacity - 0.5).abs() < 1e-6);
    }
}
