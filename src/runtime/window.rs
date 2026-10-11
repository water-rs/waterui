//! Module defining the `Window` struct for UI windows.
//!
//! # Window Backgrounds
//!
//! A window's background is what lies behind its content: the theme's
//! background colour, a solid colour (translucent through its alpha), or a
//! [`Material`]. A material window background is that window's material. The
//! mainline backends realize it at the window — within-window levels as a
//! backdrop treatment of the window behind its content, behind-window levels
//! by letting the desktop show through the window; a backend that does not
//! realize materials draws the window opaque in the theme's background
//! colour. See [`WindowBackground::Material`] for the per-platform
//! realizations.
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::window::{Window, WindowState};
//!
//! // Semi-transparent colored window
//! let tinted = Window::new("Tinted", binding::<WindowState>(WindowState::default()), || text!("Hello"))
//!     .background(Color::srgb(0, 0, 0).with_opacity(0.8));
//!
//! // Frosted glass window: the desktop shows through, blurred on macOS;
//! // on Hydrolysis tinted, and blurred where the compositor
//! // supports it
//! let frosted = Window::new("Frosted", binding::<WindowState>(WindowState::default()), || text!("Hello"))
//!     .background(Material::UltraThin);
//! ```

use std::{fmt::Debug, future::Future, rc::Rc, sync::Arc};

use nami::{Binding, Computed, Signal, SignalExt as _, impl_constant, signal::IntoComputed};
use suiteki::Str;
use waterui_core::handler::{AnyViewBuilder, Handler, ViewBuilder, boxed_action};
use waterui_core::{AnyView, Dynamic, Environment, View, flatten_signal};
use waterui_graphics::{Color, color::WorkingColor};
use waterui_layout::{Point, Rect, Size};

pub use super::window_close::CloseReply;
use super::window_close::{CloseRequest, Question};

use crate::app::{application_identifier, application_name};
#[cfg(feature = "snackbar")]
use crate::snackbar::SnackbarManager;
use crate::{
    ViewExt, background::Material, component::label::LabelDisplayMode,
    prelude::FullScreenOverlayManager, theme::color::Background,
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
    /// How each platform honours `false`:
    ///
    /// - **macOS and Windows:** the close button is disabled.
    /// - **Linux (X11 and Wayland):** the button stays drawn and enabled,
    ///   but the close request it sends is ignored.
    /// - **iOS, Android and web:** there is no window close.
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
    /// window is shown, including a switch between [`WindowBackground::Opaque`],
    /// a translucent [`WindowBackground::Color`] and a
    /// [`WindowBackground::Material`]. Backends realize what
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
    /// The window's own icon, or `None` for the application icon.
    ///
    /// Reactive: backends observe the binding and re-apply the icon to a
    /// shown window, so an app can switch icons at runtime — following a
    /// theme change, say — through [`WindowHandle::set_icon`] or its own
    /// binding. `None`, the default, keeps the application icon the `water`
    /// CLI stages next to the asset bundle, and setting the binding back to
    /// `None` restores it.
    ///
    /// Platform support: hydrolysis/winit's `set_window_icon` realizes the
    /// icon on X11, and on Windows alongside `set_taskbar_icon` (the
    /// taskbar and Alt-Tab read the separate big icon). Wayland and macOS
    /// are unsupported — winit 0.30 cannot set a window icon there — and
    /// Hydrolysis on Android has no per-window icon. `AppKit` and `UIKit`
    /// identify an application by one icon: unsupported there, not faked
    /// through the Dock. The attribute does not cross the C ABI.
    pub icon: Binding<Option<WindowIcon>>,
    /// The close-request machine [`Self::on_close_request`] installs into,
    /// shared by `Rc` with [`WindowHandle`] and armed by the backend when the
    /// window is realized.
    close_request: CloseRequest,
}

/// A window's own icon: straight-alpha sRGB RGBA8 pixels and their size.
///
/// Cloning shares the pixels, so a [`Binding`] snapshot is cheap. Platforms
/// pick or scale the size they show; a square image of 32 or 64 pixels suits
/// title bars and taskbars.
#[derive(Clone)]
pub struct WindowIcon {
    width: u32,
    height: u32,
    rgba: Arc<[u8]>,
}

impl WindowIcon {
    /// An icon of `width × height` pixels, `rgba` holding them row by row,
    /// top to bottom, four bytes each — red, green, blue and straight
    /// (non-premultiplied) alpha.
    ///
    /// # Panics
    ///
    /// Panics when the icon has no pixels, or when `rgba` is not exactly
    /// `width * height * 4` bytes long.
    #[must_use]
    pub fn new(width: u32, height: u32, rgba: impl Into<Arc<[u8]>>) -> Self {
        let rgba = rgba.into();
        assert!(
            width > 0 && height > 0,
            "WindowIcon::new: a window icon needs at least one pixel, got {width}x{height}"
        );
        let expected = usize::try_from(width)
            .ok()
            .and_then(|width| width.checked_mul(usize::try_from(height).ok()?))
            .and_then(|pixels| pixels.checked_mul(4))
            .unwrap_or_else(|| {
                panic!("WindowIcon::new: a {width}x{height} icon does not fit in memory")
            });
        assert_eq!(
            rgba.len(),
            expected,
            "WindowIcon::new: a {width}x{height} icon takes {expected} RGBA8 bytes, got {}",
            rgba.len()
        );
        Self {
            width,
            height,
            rgba,
        }
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The pixels, `width * height * 4` bytes of straight-alpha RGBA, row by
    /// row from the top.
    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

impl PartialEq for WindowIcon {
    fn eq(&self, other: &Self) -> bool {
        // Snapshots of one binding share their pixels, so the common
        // comparison — has the icon changed since the last sync? — never
        // reads them.
        self.width == other.width
            && self.height == other.height
            && (Arc::ptr_eq(&self.rgba, &other.rgba) || self.rgba == other.rgba)
    }
}

impl Eq for WindowIcon {}

impl Debug for WindowIcon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowIcon")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Converts a fixed icon or an application-owned icon binding into the
/// reactive value carried by a [`Window`].
pub trait IntoWindowIcon {
    /// Converts this value into a window-icon binding.
    fn into_window_icon(self) -> Binding<Option<WindowIcon>>;
}

impl IntoWindowIcon for WindowIcon {
    fn into_window_icon(self) -> Binding<Option<WindowIcon>> {
        Binding::container(Some(self))
    }
}

impl IntoWindowIcon for Option<WindowIcon> {
    fn into_window_icon(self) -> Binding<Option<WindowIcon>> {
        Binding::container(self)
    }
}

impl IntoWindowIcon for Binding<Option<WindowIcon>> {
    fn into_window_icon(self) -> Binding<Option<WindowIcon>> {
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

/// The background style of a window: what lies behind its content.
///
/// # Platform Support
///
/// [`Opaque`](Self::Opaque) and [`Color`](Self::Color):
///
/// - **macOS**: `NSWindow.backgroundColor` and `isOpaque`.
/// - **Android**: `Window.setBackgroundDrawable()`.
/// - **Linux (GTK)**: window CSS background.
/// - **Windows (`WinUI`)**: the content root's background brush.
/// - **Hydrolysis**: the surface clear colour and composite alpha mode.
///
/// [`Material`](Self::Material) is described on the variant.
#[derive(Debug, Clone, Default)]
pub enum WindowBackground {
    /// Opaque background in the theme's [`Background`] colour.
    #[default]
    Opaque,
    /// Solid color background (can be semi-transparent via alpha).
    Color(Color),
    /// The window's material: every level is allowed, and each backend
    /// realizes the level's own blending, as it does for a view's
    /// [`Material`] background.
    ///
    /// Within-window levels ([`Material::Regular`], [`Material::Thick`],
    /// [`Material::UltraThick`]) are a backdrop treatment over the window's
    /// opaque theme [`Background`]. Behind-window levels
    /// ([`Material::UltraThin`], [`Material::Thin`]) make the window
    /// translucent so the desktop shows through it.
    ///
    /// - **macOS**: an `NSVisualEffectView` filling the window behind its
    ///   content, with the level's blending mode; for a behind-window level
    ///   the window is non-opaque with a clear background, so the desktop
    ///   shows through blurred.
    /// - **iOS**: a `UIVisualEffectView` filling the window behind its
    ///   content, over the theme background — nothing lies behind an iOS
    ///   window.
    /// - **Hydrolysis on a desktop**: a within-window level mounts the
    ///   window's content over the level's backdrop treatment of the opaque
    ///   theme background. A behind-window level makes the window and its
    ///   surface transparent and composites the level's colour treatment as
    ///   the closest source-over tint under the content; blurring the desktop
    ///   is the compositor's job. The desktop is blurred where the platform's
    ///   blur-behind is wired — an `NSVisualEffectView` behind the window on
    ///   macOS, the DWM's acrylic system backdrop on Windows 11 22H2 and
    ///   later, `_KDE_NET_WM_BLUR_BEHIND_REGION` under a window manager that
    ///   honours it (`KWin`) on X11, and `ext-background-effect-v1` on
    ///   Wayland, whose compositor applies blur by its own policy — and
    ///   shows through tinted but unblurred elsewhere: older Windows, other
    ///   X11 window managers and Wayland compositors not advertising the
    ///   global.
    /// - **Hydrolysis on Android**: a within-window level is realized as on a
    ///   desktop. Window transparency is not realized on Android yet, so a
    ///   behind-window level renders as an opaque window
    ///   (water-rs/waterui#1966).
    /// - **Android (the Kotlin runtime)** and the experimental backends realize
    ///   no material: the window is drawn opaque in the theme's background
    ///   colour. Hydrolysis is the Android backend that realizes it
    ///   (water-rs/waterui#1899).
    Material(Material),
}

impl From<Color> for WindowBackground {
    fn from(color: Color) -> Self {
        Self::Color(color)
    }
}

impl From<Material> for WindowBackground {
    fn from(material: Material) -> Self {
        Self::Material(material)
    }
}

impl From<WindowBackground> for Binding<WindowBackground> {
    fn from(background: WindowBackground) -> Self {
        Self::container(background)
    }
}

/// A window background resolved for the frame a backend realizes: the
/// concrete colour to paint behind the content, or the material to realize
/// at the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResolvedWindowBackground {
    /// Paint this colour behind the window's content. An opacity below one
    /// asks for a translucent window. [`WindowBackground::Opaque`] resolves
    /// to the theme's [`Background`] colour.
    Color(WorkingColor),
    /// Realize this material at the window. See
    /// [`WindowBackground::Material`].
    Material(Material),
}

/// Resolves a reactive window background to what a backend realizes behind
/// the window's content.
///
/// The result follows both a change of the background itself — including a
/// switch between [`WindowBackground::Opaque`], [`WindowBackground::Color`]
/// and [`WindowBackground::Material`] — and a change of the colour it
/// currently resolves to, such as a theme switch.
#[must_use]
pub fn resolve_background<S>(
    background: &S,
    env: &Environment,
) -> Computed<ResolvedWindowBackground>
where
    S: Signal<Output = WindowBackground>,
{
    let env = env.clone();
    flatten_signal(background.map(move |background| {
        let color = match background {
            WindowBackground::Opaque => Color::new(Background),
            WindowBackground::Color(color) => color,
            WindowBackground::Material(material) => {
                return Computed::constant(ResolvedWindowBackground::Material(material));
            }
        };
        color
            .resolve(&env)
            .map(ResolvedWindowBackground::Color)
            .into_computed()
    }))
}

/// Input type for `Window::background()` method.
///
/// Accepts a `Color`, a [`Material`], a [`WindowBackground`], or a
/// `Binding<WindowBackground>` the app keeps to change the background later;
/// a colour and a material each become the matching [`WindowBackground`].
#[derive(Debug)]
pub struct WindowBackgroundInput(Binding<WindowBackground>);

impl From<WindowBackground> for WindowBackgroundInput {
    fn from(background: WindowBackground) -> Self {
        Self(background.into())
    }
}

impl From<Binding<WindowBackground>> for WindowBackgroundInput {
    fn from(background: Binding<WindowBackground>) -> Self {
        Self(background)
    }
}

impl From<Color> for WindowBackgroundInput {
    fn from(color: Color) -> Self {
        WindowBackground::Color(color).into()
    }
}

impl From<Material> for WindowBackgroundInput {
    fn from(material: Material) -> Self {
        WindowBackground::Material(material).into()
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
        // Overlay and snackbar state are created inside the builder so each
        // built instance owns its own managers: a `Dynamic` is single-consumer
        // and cannot be mounted by more than one scene.
        let content = AnyViewBuilder::new(move || {
            let (overlay_manager, overlay_view) = FullScreenOverlayManager::new();
            let content = content
                .build()
                .overlay(overlay_view)
                .with(overlay_manager.clone())
                .state(&overlay_manager);
            #[cfg(feature = "snackbar")]
            let (snackbar_manager, snackbar_view) = SnackbarManager::new();
            #[cfg(feature = "snackbar")]
            let content = content
                .overlay(snackbar_view)
                .with(snackbar_manager.clone())
                .state(&snackbar_manager);
            AnyView::new(content)
        });

        let close_request = CloseRequest::new(&state);
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
            level: Computed::constant(WindowLevel::Normal),
            attention: Binding::container(None),
            resize_increments: None,
            icon: Binding::container(None),
            close_request,
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

    /// Set the window's own icon.
    ///
    /// Takes a [`WindowIcon`] for a fixed icon, an `Option<WindowIcon>`, or a
    /// `Binding<Option<WindowIcon>>` the app keeps to change the icon after
    /// the window is shown. See [`Self::icon`] for the default and platform
    /// support notes.
    #[must_use]
    pub fn icon(mut self, icon: impl IntoWindowIcon) -> Self {
        self.icon = icon.into_window_icon();
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
    /// Accepts a `Color` for solid backgrounds, a `Material` for the window's
    /// material, or a [`WindowBackground`] or a `Binding<WindowBackground>` to
    /// change the background after the window is shown — between a colour,
    /// [`WindowBackground::Opaque`] and a material alike.
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
    /// // Frosted glass window: the desktop shows through, blurred on macOS;
    /// // on Hydrolysis tinted, and blurred where the compositor
    /// // supports it
    /// let frosted = Window::new("Frosted", binding::<WindowState>(WindowState::default()), || text!("Hello"))
    ///     .background(Material::UltraThin);
    /// ```
    #[must_use]
    pub fn background(mut self, background: impl Into<WindowBackgroundInput>) -> Self {
        self.background = background.into().0;
        self
    }

    /// What a backend realizes behind the window's content, following the
    /// reactive [`Self::background`]. See [`resolve_background`].
    #[must_use]
    pub fn resolved_background(&self, env: &Environment) -> Computed<ResolvedWindowBackground> {
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

    /// Asks `handler` before the window closes.
    ///
    /// Every close request — the title-bar close button, the window
    /// manager's close (X11 `WM_DELETE_WINDOW`, Wayland
    /// `xdg_toplevel.close`, `WM_CLOSE`), a Close Window menu command,
    /// `performClose:` or [`WindowHandle::request_close`] — runs the handler
    /// and applies its [`CloseReply`]: [`Close`](CloseReply::Close) writes
    /// `state =` [`WindowState::Closed`] and the normal teardown runs,
    /// [`Cancel`](CloseReply::Cancel) leaves the window open and writes
    /// nothing. A request arriving while a question is already open is
    /// dropped — the machine asks one question per window.
    ///
    /// Writing `state` to [`WindowState::Closed`] yourself, or
    /// [`WindowHandle::close`], is not a request and never runs the handler;
    /// a programmatic close while a question is open drops its future,
    /// cancelling it. `closable == false` drops a request before the handler
    /// runs.
    ///
    /// The handler extracts from the environment the window renders under —
    /// a `State`, a service — and its future runs on the runner's local
    /// executor, so the reply never arrives inside a platform delegate call.
    ///
    /// Platform support: Hydrolysis desktop and `AppKit`. Hydrolysis Android,
    /// `UIKit` and web have no window close request — the handler never runs
    /// there, and [`WindowHandle::request_close`] panics.
    #[must_use]
    pub fn on_close_request<H, Args, Fut>(self, handler: H) -> Self
    where
        H: Handler<Args, Fut>,
        Fut: Future<Output = CloseReply> + 'static,
    {
        let mut action = boxed_action(handler);
        self.close_request
            .set_hook(Box::new(move |env| Box::pin(action(env)) as Question));
        self
    }

    /// Arms close requests with the environment the window renders under —
    /// a backend calls it when it realizes the window, so
    /// [`WindowHandle::request_close`] can file before any platform request
    /// arrives.
    #[doc(hidden)]
    pub fn arm_close_requests(&self, env: &Environment) {
        self.close_request.arm(env, self.closable);
    }

    /// Files a user close request — the one entry point a backend calls when
    /// the platform asks to close the window. `env` is the environment the
    /// window renders under; the call arms the machine with it, so the
    /// handler extracts from it. `closable == false` drops the request before
    /// the handler runs.
    #[doc(hidden)]
    pub fn request_close(&self, env: &Environment) {
        self.arm_close_requests(env);
        self.close_request.request();
    }

    /// Whether an [`on_close_request`](Self::on_close_request) handler is
    /// installed — the synchronous verdict a delegate like
    /// `windowShouldClose:` needs.
    #[doc(hidden)]
    #[must_use]
    pub fn has_close_handler(&self) -> bool {
        self.close_request.has_handler()
    }

    /// Get a handle to control the window after showing it.
    #[must_use]
    pub fn handle(&self) -> WindowHandle {
        WindowHandle {
            frame: self.frame.clone(),
            state: self.state.clone(),
            attention: self.attention.clone(),
            style: self.style.clone(),
            background: self.background.clone(),
            icon: self.icon.clone(),
            close_request: self.close_request.clone(),
        }
    }

    /// Show the window on screen.
    ///
    /// The window opens independently of any view's lifetime, which makes
    /// this the way to show a window that must outlive every other window —
    /// for example one reopened under
    /// [`LastWindowPolicy::StayResident`](crate::app::LastWindowPolicy::StayResident)
    /// after the last window closed. For window presentation tied to a
    /// mounted view, see [`conditional_window`].
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
/// window is never presented. The presentation lives only as long as the
/// window hosting that view and ends when the host closes; a window that
/// must outlive every other window — for example one reopened under
/// [`LastWindowPolicy::StayResident`](crate::app::LastWindowPolicy::StayResident)
/// after the last window closed — is opened with [`Window::show`] instead.
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
    style: Binding<WindowStyle>,
    background: Binding<WindowBackground>,
    icon: Binding<Option<WindowIcon>>,
    close_request: CloseRequest,
}

impl WindowHandle {
    /// Files a close request the window's
    /// [`on_close_request`](Window::on_close_request) handler decides, as the
    /// title-bar close button would. With no handler installed the window
    /// closes immediately; `closable == false` drops the request.
    /// [`Self::close`] closes without asking.
    ///
    /// # Panics
    ///
    /// When the window has not been realized yet, and on the platforms with
    /// no window close request — Hydrolysis Android, `UIKit` and web.
    pub fn request_close(&self) {
        self.close_request.request();
    }

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
    pub fn set_icon(&self, icon: impl Into<Option<WindowIcon>>) {
        self.icon.set(icon.into());
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use nami::{Binding, Signal};
    use waterui_core::{Environment, plugin::Plugin as _};
    use waterui_graphics::{
        Color,
        color::{WorkingColor, working::from_linear_srgb},
    };

    use super::{
        ResolvedWindowBackground, Window, WindowBackground, WindowIcon, WindowState,
        resolve_background,
    };
    use crate::background::Material;
    use crate::theme::{
        ColorSettings, Theme,
        color::{Accent, Background},
    };

    fn color(resolved: ResolvedWindowBackground) -> WorkingColor {
        match resolved {
            ResolvedWindowBackground::Color(color) => color,
            ResolvedWindowBackground::Material(material) => {
                panic!("expected a colour, resolved Material::{material:?}")
            }
        }
    }

    /// `Window::background(Material)` stores the material as the window's
    /// background instead of wrapping the content.
    #[test]
    fn a_material_is_the_window_background() {
        let window = Window::new("", Binding::container(WindowState::Normal), || ())
            .background(Material::Thin);
        assert!(matches!(
            window.background.snapshot(),
            WindowBackground::Material(Material::Thin)
        ));
        assert_eq!(
            window.resolved_background(&Environment::new()).snapshot(),
            ResolvedWindowBackground::Material(Material::Thin)
        );
    }

    /// The resolved background follows a replacement of the background
    /// itself — colour to colour, to a material, to `Opaque` — and, while a
    /// colour is set, a theme change of the colour it resolves through.
    #[test]
    fn resolved_background_follows_a_replaced_background() {
        let green = from_linear_srgb([0.0, 1.0, 0.0], 1.0);
        let red = from_linear_srgb([1.0, 0.0, 0.0], 1.0);
        let accent = Binding::container(green);
        let mut env = Environment::new();
        Theme::new()
            .colors(
                ColorSettings::new()
                    .background(from_linear_srgb([0.5, 0.5, 0.5], 1.0))
                    .accent(accent.clone()),
            )
            .install(&mut env);

        let background = Binding::container(WindowBackground::Color(Color::srgb(255, 0, 0)));
        let resolved = resolve_background(&background, &env);
        // Components are linear Display P3 red, green, blue, alpha: sRGB red
        // lands near 0.82 in the wider P3 gamut.
        assert!(color(resolved.snapshot()).components[0] > 0.8);

        let seen = Rc::new(RefCell::new(Vec::new()));
        let _guard = resolved.watch({
            let seen = seen.clone();
            move |ctx| seen.borrow_mut().push(ctx.into_value())
        });
        let last = || *seen.borrow().last().expect("a change was delivered");

        background.set(WindowBackground::Color(
            Color::srgb(0, 0, 255).with_opacity(0.5),
        ));
        let blue = color(last());
        assert!(blue.components[2] > 0.9 && blue.components[0] < 0.05);
        assert!((blue.components[3] - 0.5).abs() < 1e-6);
        assert!((color(resolved.snapshot()).components[3] - 0.5).abs() < 1e-6);

        background.set(WindowBackground::Material(Material::UltraThin));
        assert_eq!(
            last(),
            ResolvedWindowBackground::Material(Material::UltraThin)
        );
        assert_eq!(
            resolved.snapshot(),
            ResolvedWindowBackground::Material(Material::UltraThin)
        );

        background.set(WindowBackground::Opaque);
        assert_eq!(
            color(last()),
            Color::new(Background).resolve(&env).snapshot(),
            "`Opaque` resolves to the theme background"
        );

        // A theme change while a colour is set reaches the resolved colour.
        background.set(WindowBackground::Color(Color::new(Accent)));
        assert_eq!(color(last()), green);
        accent.set(red);
        assert_eq!(color(last()), red);
        assert_eq!(color(resolved.snapshot()), red);
    }

    /// The icon validates its pixel buffer against its size.
    #[test]
    #[should_panic(expected = "a 2x1 icon takes 8 RGBA8 bytes, got 4")]
    fn window_icon_rejects_a_short_buffer() {
        let _ = WindowIcon::new(2, 1, vec![0_u8; 4]);
    }

    /// An icon without pixels is rejected rather than handed to a platform.
    #[test]
    #[should_panic(expected = "at least one pixel")]
    fn window_icon_rejects_an_empty_icon() {
        let _ = WindowIcon::new(0, 4, Vec::<u8>::new());
    }

    /// Icons compare by their pixels, whether or not they share them.
    #[test]
    fn window_icons_compare_by_pixels() {
        let icon = WindowIcon::new(1, 1, vec![1_u8, 2, 3, 4]);
        assert_eq!(icon, icon.clone());
        assert_eq!(icon, WindowIcon::new(1, 1, vec![1_u8, 2, 3, 4]));
        assert_ne!(icon, WindowIcon::new(1, 1, vec![1_u8, 2, 3, 5]));
    }
}
