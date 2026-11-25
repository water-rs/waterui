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
//! use waterui::window::Window;
//!
//! // Semi-transparent colored window
//! let tinted = Window::new("Tinted", || text!("Hello"))
//!     .background(Color::srgb(0, 0, 0).with_opacity(0.8));
//!
//! // Frosted glass window: the desktop shows through, blurred on macOS;
//! // on Hydrolysis tinted, and blurred where the compositor
//! // supports it
//! let frosted = Window::new("Frosted", || text!("Hello"))
//!     .background(Material::UltraThin);
//! ```

use std::{
    cell::RefCell,
    fmt::{self, Debug},
    future::Future,
    rc::Rc,
    sync::Arc,
};

use nami::{Binding, Computed, Signal, SignalExt as _, impl_constant, signal::IntoComputed};
use suiteki::Str;
use waterui_core::backend::Backend;
use waterui_core::env::Installed;
use waterui_core::extract::State as EnvState;
use waterui_core::handler::{AnyViewBuilder, Handler, ViewBuilder, boxed_action};
use waterui_core::metadata::Metadata;
use waterui_core::plugin::Plugin;
use waterui_core::{AnyView, Environment, SerialDispatch, View, flatten_signal};
use waterui_graphics::{Color, color::WorkingColor};
use waterui_layout::{Point, Rect, Size};
use waterui_text::{IntoText, Text};

pub use super::window_close::CloseReply;
use super::window_close::{CloseRequest, Question, WeakCloseRequest};
use super::window_focus::FocusRequest;

use crate::app::{application_identifier, application_name};
use crate::{background::Material, component::label::LabelDisplayMode, theme::color::Background};

/// Represents a window in the UI.
///
/// `Window` is builder-only: configure it with [`Window::new`] and chained
/// setters, pass it to [`App::window`](crate::app::App::window) or
/// [`Window::show`], and extract [`WindowHandle`] as handler state inside its
/// content. Backends take a window apart with [`Window::into_parts`].
///
/// `C` builds the window's content; [`Window::new`] wraps the application's
/// builder in a [`WindowContent`], so the content keeps its type up to the
/// backend that realizes it. `T` holds the window's optional toolbar (`()`
/// until [`Window::toolbar`] attaches one), keeping the toolbar view type
/// intact up to the backend as well.
pub struct Window<C, T = ()> {
    title: Text,
    closable: bool,
    resizable: bool,
    frame: Binding<Rect>,
    content: C,
    state: Binding<WindowState>,
    toolbar: T,
    style: Binding<WindowStyle>,
    background: Binding<WindowBackground>,
    min_size: Option<Computed<Size>>,
    max_size: Option<Computed<Size>>,
    app_id: Option<Str>,
    instance_name: Option<Str>,
    placement: Option<WindowPlacement>,
    activation: Activation,
    level: Computed<WindowLevel>,
    attention: Binding<Option<UserAttention>>,
    resize_increments: Option<Computed<Size>>,
    icon: Binding<Option<WindowIcon>>,
    close_request: CloseRequest,
    focus_request: FocusRequest,
}

/// A [`Window`] taken apart for a backend runner, see [`Window::into_parts`].
pub struct WindowParts<C, T = ()> {
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
    pub content: C,
    /// The current state of the window.
    pub state: Binding<WindowState>,
    /// Optional toolbar content for the window.
    ///
    /// `()` when unset; [`Window::toolbar`] stores the toolbar view wrapped in
    /// [`LabelDisplayMode::IconOnly`]. Notice that it may not be supported on
    /// all platforms.
    pub toolbar: T,
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
    /// [`WindowParts::resolved_background`] resolves it to.
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
    /// macOS `NSWindow.contentMinSize`/`contentMaxSize`) and by hydrolysis on
    /// iPadOS, where the limits reach `UIWindowScene.sizeRestrictions` (an
    /// iPhone scene always fills the screen and has none). Android's single-Activity model has no
    /// per-window runtime size limits; embedded (dew) displays are fixed-size — those
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
    /// The close-request machine [`Window::on_close_request`] installs into,
    /// shared by `Rc` with [`WindowHandle`] and armed by the backend when the
    /// window is realized.
    close_request: CloseRequest,
    /// The focus machine [`WindowHandle::focus`] files into, shared by
    /// `Rc` with [`WindowHandle`] and armed by the backend when the window
    /// is realized.
    focus_request: FocusRequest,
}

/// A window whose content and toolbar types are explicitly erased via
/// [`Window::erase`] or [`Windows::erase`](crate::app::Windows::erase).
pub type AnyWindow = Window<AnyViewBuilder<AnyView>, Option<AnyView>>;

/// A window's declared toolbar: `()` when unset, or a view subtree whose
/// concrete type stays intact up to the backend.
pub trait WindowToolbar: 'static {
    /// The concrete view type of a configured toolbar.
    type View: View;

    /// Extracts the toolbar view if one was configured.
    fn into_view(self) -> Option<Self::View>;

    /// Erases a configured toolbar into [`AnyView`].
    fn erase(self) -> Option<AnyView>
    where
        Self: Sized,
    {
        self.into_view().map(AnyView::new)
    }
}

impl WindowToolbar for () {
    type View = ();

    fn into_view(self) -> Option<Self::View> {
        None
    }

    fn erase(self) -> Option<AnyView> {
        None
    }
}

/// Opaque wrapper for a window's configured toolbar subtree.
///
/// Returned by [`Window::toolbar`], preserving the toolbar view's concrete type
/// while scoping [`LabelDisplayMode::IconOnly`] into its environment.
#[derive(Debug)]
pub struct ToolbarContent<V>(Installed<V, LabelDisplayMode>);

impl<V: View> View for ToolbarContent<V> {
    fn body(self, _env: &Environment) -> impl View {
        self.0
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        self.0.dispatch(backend, env, cx)
    }
}

impl<V: View> WindowToolbar for ToolbarContent<V> {
    type View = Self;

    fn into_view(self) -> Option<Self::View> {
        Some(self)
    }
}

impl<V: View> WindowToolbar for Option<V> {
    type View = V;

    fn into_view(self) -> Option<Self::View> {
        self
    }
}

impl<C, T> Debug for Window<C, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Window")
            .field("title", &self.title)
            .field("closable", &self.closable)
            .field("resizable", &self.resizable)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl<C, T> Debug for WindowParts<C, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowParts")
            .field("title", &self.title)
            .field("closable", &self.closable)
            .field("resizable", &self.resizable)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// Wraps a window's content builder so each built root view installs the
/// window's own [`WindowHandle`] into its environment as handler state.
#[derive(Debug)]
pub struct WindowHandleContent<C> {
    content: C,
    handle: WindowHandle,
}

impl<C: ViewBuilder> ViewBuilder for WindowHandleContent<C> {
    type Output = WindowHandleView<C::Output>;

    fn build(&self) -> Self::Output {
        WindowHandleView {
            content: self.content.build(),
            handle: self.handle.clone(),
        }
    }
}

/// One built instance of a window's content with its [`WindowHandle`]
/// installed in the environment, see [`WindowHandleContent`].
#[derive(Debug)]
pub struct WindowHandleView<V> {
    content: V,
    handle: WindowHandle,
}

impl<V: View> View for WindowHandleView<V> {
    fn body(self, env: &Environment) -> impl View {
        Metadata::new(self.content, env.extending(EnvState(self.handle)))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        let env = env.extending(EnvState(self.handle));
        self.content.dispatch(backend, &env, cx)
    }
}

/// The content builder [`Window::new`] gives a window.
#[derive(Debug)]
pub struct WindowContent<B>(B);

impl<B: ViewBuilder> ViewBuilder for WindowContent<B> {
    type Output = WindowRoot<B::Output>;

    fn build(&self) -> Self::Output {
        WindowRoot(self.0.build())
    }
}

/// One built instance of a window's content, see [`WindowContent`].
#[derive(Debug)]
pub struct WindowRoot<V>(V);

impl<V: View> View for WindowRoot<V> {
    fn body(self, _env: &Environment) -> impl View {
        self.0
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        self.0.dispatch(backend, env, cx)
    }
}

/// Why pixels cannot make a [`WindowIcon`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WindowIconError {
    /// The icon has no pixels.
    #[error("a window icon needs at least one pixel, got {width}x{height}")]
    Empty {
        /// Requested width in pixels.
        width: u32,
        /// Requested height in pixels.
        height: u32,
    },
    /// The pixel count of the icon does not fit in memory.
    #[error("a {width}x{height} icon does not fit in memory")]
    TooLarge {
        /// Requested width in pixels.
        width: u32,
        /// Requested height in pixels.
        height: u32,
    },
    /// The buffer is not `width * height * 4` bytes long.
    #[error("a {width}x{height} icon takes {expected} RGBA8 bytes, got {actual}")]
    LengthMismatch {
        /// Requested width in pixels.
        width: u32,
        /// Requested height in pixels.
        height: u32,
        /// The buffer length the size requires.
        expected: usize,
        /// The buffer length supplied.
        actual: usize,
    },
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
    /// # Errors
    ///
    /// Returns a [`WindowIconError`] when the icon has no pixels, does not
    /// fit in memory, or `rgba` is not exactly `width * height * 4` bytes
    /// long.
    pub fn new(
        width: u32,
        height: u32,
        rgba: impl Into<Arc<[u8]>>,
    ) -> Result<Self, WindowIconError> {
        let rgba = rgba.into();
        if width == 0 || height == 0 {
            return Err(WindowIconError::Empty { width, height });
        }
        let expected = usize::try_from(width)
            .ok()
            .and_then(|width| width.checked_mul(usize::try_from(height).ok()?))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(WindowIconError::TooLarge { width, height })?;
        if rgba.len() != expected {
            return Err(WindowIconError::LengthMismatch {
                width,
                height,
                expected,
                actual: rgba.len(),
            });
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
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
        Binding::new(Some(self))
    }
}

impl IntoWindowIcon for Option<WindowIcon> {
    fn into_window_icon(self) -> Binding<Option<WindowIcon>> {
        Binding::new(self)
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
#[non_exhaustive]
pub struct Monitor {
    /// Bounds in the global logical coordinate space window frames use.
    frame: Rect,
    /// `frame` minus what the desktop reserves (menu bar, dock, panels, taskbar).
    /// Equal to `frame` where the platform reports no work area.
    visible_frame: Rect,
    /// Physical pixels per logical point.
    scale_factor: f64,
    /// The platform's name for the display, when it reports one.
    name: Option<Str>,
}

impl Monitor {
    /// Bounds in the global logical coordinate space window frames use.
    #[must_use]
    pub const fn frame(&self) -> Rect {
        self.frame
    }

    /// `frame` minus what the desktop reserves (menu bar, dock, panels, taskbar).
    /// Equal to `frame` where the platform reports no work area.
    #[must_use]
    pub const fn visible_frame(&self) -> Rect {
        self.visible_frame
    }

    /// Physical pixels per logical point.
    #[must_use]
    pub const fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// The platform's name for the display, when it reports one.
    #[must_use]
    pub const fn name(&self) -> Option<&Str> {
        self.name.as_ref()
    }
}

impl Monitor {
    /// A display spanning `frame`, of which `visible_frame` is free of
    /// desktop chrome, at `scale_factor` physical pixels per logical point,
    /// named `name` when the platform reports one.
    #[must_use]
    pub const fn new(
        frame: Rect,
        visible_frame: Rect,
        scale_factor: f64,
        name: Option<Str>,
    ) -> Self {
        Self {
            frame,
            visible_frame,
            scale_factor,
            name,
        }
    }
}

/// Which monitor a window is placed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends resolve each monitor selector variant by variant; a new selector must fail their build instead of falling into a wildcard arm"
)]
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
#[non_exhaustive]
pub struct WindowPlacement {
    /// The monitor a backend resolves before calling `place`.
    monitor: MonitorSelector,
    /// Computes the window frame from the resolved monitor's geometry.
    ///
    /// Backends call it once per show, immediately before the window becomes
    /// visible; `place` answers in the same global logical space
    /// [`Monitor::frame`] is expressed in.
    place: Rc<dyn Fn(&Monitor) -> Rect>,
}

impl WindowPlacement {
    /// The monitor a backend resolves before calling `place`.
    #[must_use]
    pub const fn monitor(&self) -> MonitorSelector {
        self.monitor
    }

    /// Computes the window frame from the resolved monitor's geometry.
    ///
    /// Backends call it once per show, immediately before the window becomes
    /// visible; `place` answers in the same global logical space
    /// [`Monitor::frame`] is expressed in.
    #[must_use]
    pub const fn place(&self) -> &Rc<dyn Fn(&Monitor) -> Rect> {
        &self.place
    }
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
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each activation policy variant by variant on every desktop platform"
)]
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
/// state binding represents a window that has not yet been shown. On a
/// window declared on [`App`](crate::app::App), setting the binding from
/// [`Self::Closed`] to [`Self::Normal`] (or any open state) creates the
/// platform window, writing [`Self::Normal`] while it is already open
/// restores and focuses it, and writing [`Self::Closed`] tears down the
/// platform window while keeping the declaration armed for future reopens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each window state variant by variant; a new window state must fail their build instead of falling into a wildcard arm"
)]
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
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each window level variant by variant; a new window level must fail their build instead of falling into a wildcard arm"
)]
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
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each attention request variant by variant; a new attention request must fail their build instead of falling into a wildcard arm"
)]
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
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each window style variant by variant; a new window style must fail their build instead of falling into a wildcard arm"
)]
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
#[non_exhaustive]
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
    /// - **Experimental backends** that do not implement material backgrounds
    ///   draw the window opaque in the theme's background colour.
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
        Self::new(background)
    }
}

/// A window background resolved for the frame a backend realizes: the
/// concrete colour to paint behind the content, or the material to realize
/// at the window.
#[derive(Debug, Clone, Copy, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each window background variant by variant; a new window background must fail their build instead of falling into a wildcard arm"
)]
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

/// Sends windows of content type `C` and toolbar type `T` to the runner
/// owning the paired [`WindowChannel`].
///
/// Created with [`WindowManager::new`]; pass the returned [`WindowChannel`] to
/// [`App::window`](crate::app::App::window), and show windows from handlers
/// through [`Self::show`] or [`Window::show`].
///
/// Extracts through `State<Self>`, so `.state(&manager)` on
/// [`App`](crate::app::App) or a view makes `manager: WindowManager<C, T>`
/// available to handlers; cloning `manager` into a closure works directly as
/// well.
#[waterui_macros::state]
pub struct WindowManager<C: 'static, T: 'static = ()>(Rc<WindowManagerInner<C, T>>);

impl<C: 'static, T: 'static> Clone for WindowManager<C, T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<C: 'static, T: 'static> Plugin for WindowManager<C, T> {}

impl<C: 'static, T: 'static> Debug for WindowManager<C, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowManager").finish_non_exhaustive()
    }
}

struct WindowManagerInner<C, T> {
    state: RefCell<WindowChannelState<C, T>>,
    /// Windows shown while a receiver call is in flight are delivered after it
    /// returns, in order, rather than re-entering the receiver.
    dispatch: SerialDispatch<Window<C, T>>,
}

enum WindowChannelState<C, T> {
    /// Connected to the runner's receiver.
    Connected(Rc<dyn Fn(Window<C, T>)>),
    /// Not yet connected; holds windows shown before the runner connected, in
    /// call order.
    Unconnected(Vec<Window<C, T>>),
    /// The paired [`WindowChannel`] was dropped without connecting.
    Disconnected,
}

impl<C: 'static, T: 'static> WindowManager<C, T> {
    /// Creates a typed window manager and the [`WindowChannel`] a runner
    /// receives its windows from.
    #[must_use]
    pub fn new() -> (Self, WindowChannel<C, T>) {
        let manager = Self(Rc::new(WindowManagerInner {
            state: RefCell::new(WindowChannelState::Unconnected(Vec::new())),
            dispatch: SerialDispatch::new(),
        }));
        let channel = WindowChannel(manager.clone());
        (manager, channel)
    }

    /// Shows `window` on the runner connected to this manager's
    /// [`WindowChannel`].
    ///
    /// Windows shown before the runner connects are queued in call order and
    /// opened when the application starts.
    ///
    /// # Contract
    ///
    /// Showing a window after the paired [`WindowChannel`] was dropped without
    /// connecting to a runner breaks the contract: a debug build panics, and a
    /// release build logs the error and shows nothing.
    pub fn show(&self, window: Window<C, T>)
    where
        C: ViewBuilder,
        T: WindowToolbar,
    {
        self.0.dispatch.deliver(window, |window| {
            let receiver = {
                let mut state = self.0.state.borrow_mut();
                match &mut *state {
                    WindowChannelState::Connected(receiver) => Rc::clone(receiver),
                    WindowChannelState::Unconnected(pending) => {
                        pending.push(window);
                        return;
                    }
                    WindowChannelState::Disconnected => {
                        crate::contract_violation!(
                            "WindowChannel was dropped without being connected to a runner, so the window is not shown"
                        );
                        return;
                    }
                }
            };
            receiver(window);
        });
    }
}

/// The runner's end of a [`WindowManager`]: a [`Windows`](crate::app::Windows)
/// source that carries `C` and `T`'s concrete types to the runner without
/// erasing to [`AnyWindow`].
pub struct WindowChannel<C: 'static, T: 'static = ()>(WindowManager<C, T>);

impl<C: 'static, T: 'static> Debug for WindowChannel<C, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowChannel").finish_non_exhaustive()
    }
}

impl<C: 'static, T: 'static> WindowChannel<C, T> {
    /// How many windows were queued on this channel before a runner connected
    /// it.
    #[must_use]
    pub fn queued_len(&self) -> usize {
        match &*self.0.0.state.borrow() {
            WindowChannelState::Unconnected(pending) => pending.len(),
            WindowChannelState::Connected(_) | WindowChannelState::Disconnected => 0,
        }
    }

    /// Connects this channel to `receiver` for windows shown while the
    /// application runs, and returns any windows already queued before the
    /// connection, in call order.
    pub fn connect(self, receiver: impl Fn(Window<C, T>) + 'static) -> Vec<Window<C, T>> {
        let mut state = self.0.0.state.borrow_mut();
        let queued = match &mut *state {
            WindowChannelState::Unconnected(pending) => core::mem::take(pending),
            WindowChannelState::Connected(_) | WindowChannelState::Disconnected => {
                unreachable!("WindowChannel is single-consumer and connects once")
            }
        };
        *state = WindowChannelState::Connected(Rc::new(receiver));
        queued
    }

    /// Consumes this channel without connecting a live receiver, returning any
    /// windows already queued on it.
    #[must_use]
    pub fn drain(self) -> Vec<Window<C, T>> {
        let mut state = self.0.0.state.borrow_mut();
        let queued = match &mut *state {
            WindowChannelState::Unconnected(pending) => core::mem::take(pending),
            WindowChannelState::Connected(_) | WindowChannelState::Disconnected => Vec::new(),
        };
        *state = WindowChannelState::Disconnected;
        queued
    }
}

impl<C: 'static, T: 'static> Drop for WindowChannel<C, T> {
    fn drop(&mut self) {
        let mut state = self.0.0.state.borrow_mut();
        if matches!(*state, WindowChannelState::Unconnected(_)) {
            *state = WindowChannelState::Disconnected;
        }
    }
}

impl_constant!(WindowState);
impl_constant!(WindowStyle);

impl From<WindowStyle> for Binding<WindowStyle> {
    fn from(style: WindowStyle) -> Self {
        Self::new(style)
    }
}

impl<B: ViewBuilder> Window<WindowContent<B>> {
    /// Create a new window with the specified title and content.
    ///
    /// Accepts any [`IntoText`] value for `title` so string literals resolve
    /// through the active translation catalog in the application's
    /// [`Environment`].
    ///
    /// The window owns a [`WindowState::Normal`] state binding by default so it
    /// opens immediately when declared on [`App`](crate::app::App). Chain
    /// [`Window::state`] to bind the window's lifetime to an external
    /// `Binding<WindowState>`.
    #[must_use]
    pub fn new(title: impl IntoText, content: B) -> Self {
        let default_frame = Rect::new(Point::zero(), Size::new(800.0, 600.0));
        let content = WindowContent(content);
        let state = Binding::new(WindowState::Normal);
        let close_request = CloseRequest::new(&state);
        Self {
            title: title.into_text(),
            closable: true,
            resizable: true,
            frame: Binding::new(default_frame),
            content,
            state,
            toolbar: (),
            style: Binding::new(WindowStyle::default()),
            background: Binding::new(WindowBackground::default()),
            min_size: None,
            max_size: None,
            app_id: None,
            instance_name: None,
            placement: None,
            activation: Activation::default(),
            level: Computed::constant(WindowLevel::Normal),
            attention: Binding::new(None),
            resize_increments: None,
            icon: Binding::new(None),
            close_request,
            focus_request: FocusRequest::new(),
        }
    }
}

impl<C, T> Window<C, T> {
    /// Bind this window's state to an external [`Binding<WindowState>`].
    ///
    /// By default [`Window::new`] owns an internal [`WindowState::Normal`]
    /// binding. Call this when the application reads or drives the window's
    /// state — for example, declaring a secondary window on
    /// [`App`](crate::app::App) starting [`WindowState::Closed`], flipping it
    /// to [`WindowState::Normal`] to open (or focus if already open), and
    /// returning it to [`WindowState::Closed`] to close while keeping the
    /// declaration armed to reopen later.
    #[must_use]
    pub fn state(mut self, state: &Binding<WindowState>) -> Self {
        self.close_request.set_state(state);
        self.state = state.clone();
        self
    }

    /// Set whether the window can be closed by the user.
    ///
    /// Defaults to `true`. See [`WindowParts::closable`] for how each platform
    /// honours `false`.
    #[must_use]
    pub const fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    /// Set the initial content size of the window.
    ///
    /// Defaults to `800×600` logical points. Preserves any origin already set
    /// on the window's frame binding.
    #[must_use]
    pub fn default_size(self, size: Size) -> Self {
        let origin = self.frame.snapshot().origin();
        self.frame.set(Rect::new(origin, size));
        self
    }

    /// Bind this window's frame to an external [`Binding<Rect>`].
    ///
    /// By default [`Window::new`] owns an `800×600` frame binding. Call this
    /// when external code reads or drives the window's position and size.
    #[must_use]
    pub fn frame(mut self, frame: &Binding<Rect>) -> Self {
        self.frame = frame.clone();
        self
    }

    /// Bind this window's attention request to an external
    /// [`Binding<Option<UserAttention>>`].
    ///
    /// See [`WindowParts::attention`] for platform support notes.
    #[must_use]
    pub fn attention(mut self, attention: &Binding<Option<UserAttention>>) -> Self {
        self.attention = attention.clone();
        self
    }

    /// Returns the window's current [`WindowState`] snapshot.
    #[must_use]
    pub fn current_state(&self) -> WindowState {
        self.state.snapshot()
    }

    /// Returns the window's current frame snapshot.
    #[must_use]
    pub fn current_frame(&self) -> Rect {
        self.frame.snapshot()
    }

    /// Takes this window apart for a backend runner, resolving its title
    /// against `env` so localized titles track the application's translation
    /// catalog and locale.
    ///
    /// Wraps `content` in [`WindowHandleContent`] with the finalized
    /// [`WindowHandle`] so every view and handler inside the window extracts
    /// its own `WindowHandle` from the environment on every backend.
    #[must_use]
    pub fn into_parts(self, env: &Environment) -> WindowParts<WindowHandleContent<C>, T> {
        let handle = self.handle();
        let Self {
            title,
            closable,
            resizable,
            frame,
            content,
            state,
            toolbar,
            style,
            background,
            min_size,
            max_size,
            app_id,
            instance_name,
            placement,
            activation,
            level,
            attention,
            resize_increments,
            icon,
            close_request,
            focus_request,
        } = self;
        let title = title
            .resolve(env)
            .content()
            .map(|styled| styled.to_plain())
            .computed();
        WindowParts {
            title,
            closable,
            resizable,
            frame,
            content: WindowHandleContent { content, handle },
            state,
            toolbar,
            style,
            background,
            min_size,
            max_size,
            app_id,
            instance_name,
            placement,
            activation,
            level,
            attention,
            resize_increments,
            icon,
            close_request,
            focus_request,
        }
    }

    /// The same window with its content builder replaced by `f(content)`.
    pub fn map_content<D>(self, f: impl FnOnce(C) -> D) -> Window<D, T> {
        self.map(f, |toolbar| toolbar)
    }

    /// The same window with its content builder replaced by `f(content)` and
    /// its toolbar replaced by `g(toolbar)`.
    pub fn map<D, U>(self, f: impl FnOnce(C) -> D, g: impl FnOnce(T) -> U) -> Window<D, U> {
        let Self {
            title,
            closable,
            resizable,
            frame,
            content,
            state,
            toolbar,
            style,
            background,
            min_size,
            max_size,
            app_id,
            instance_name,
            placement,
            activation,
            level,
            attention,
            resize_increments,
            icon,
            close_request,
            focus_request,
        } = self;
        Window {
            title,
            closable,
            resizable,
            frame,
            content: f(content),
            state,
            toolbar: g(toolbar),
            style,
            background,
            min_size,
            max_size,
            app_id,
            instance_name,
            placement,
            activation,
            level,
            attention,
            resize_increments,
            icon,
            close_request,
            focus_request,
        }
    }

    /// Set an explicit minimum content size for the window.
    ///
    /// Without one, the backend derives the minimum from the content's layout
    /// by measuring each axis independently at a zero proposal. See
    /// [`WindowParts::min_size`] for platform support notes.
    #[must_use]
    pub fn min_size(mut self, size: impl IntoComputed<Size>) -> Self {
        self.min_size = Some(size.into_computed());
        self
    }

    /// Set an explicit maximum content size for the window.
    ///
    /// See [`WindowParts::max_size`] for platform support notes.
    #[must_use]
    pub fn max_size(mut self, size: impl IntoComputed<Size>) -> Self {
        self.max_size = Some(size.into_computed());
        self
    }

    /// Set the identity the desktop groups this window under — its X11
    /// `WM_CLASS` and Wayland `app_id`.
    ///
    /// See [`WindowParts::app_id`] for the default and platform support notes.
    #[must_use]
    pub fn app_id(mut self, app_id: impl Into<Str>) -> Self {
        self.app_id = Some(app_id.into());
        self
    }

    /// Set the window's instance name inside the desktop identity — the X11
    /// `WM_CLASS` instance part.
    ///
    /// See [`WindowParts::instance_name`] for the default and platform support notes.
    #[must_use]
    pub fn instance_name(mut self, instance_name: impl Into<Str>) -> Self {
        self.instance_name = Some(instance_name.into());
        self
    }

    /// Set where the window stacks relative to other applications' windows.
    ///
    /// See [`WindowParts::level`] for platform support notes.
    #[must_use]
    pub fn level(mut self, level: impl IntoComputed<WindowLevel>) -> Self {
        self.level = level.into_computed();
        self
    }

    /// Set the steps the window's content size moves in while the user
    /// resizes it.
    ///
    /// See [`WindowParts::resize_increments`] for platform support notes.
    #[must_use]
    pub fn resize_increments(mut self, increments: impl IntoComputed<Size>) -> Self {
        self.resize_increments = Some(increments.into_computed());
        self
    }

    /// Set the window's own icon.
    ///
    /// Takes a [`WindowIcon`] for a fixed icon, an `Option<WindowIcon>`, or a
    /// `Binding<Option<WindowIcon>>` the app keeps to change the icon after
    /// the window is shown. See [`WindowParts::icon`] for the default and
    /// platform support notes.
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
    /// See [`WindowParts::placement`] for the resolution contract and platform
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
    pub fn toolbar<V: View>(self, toolbar: V) -> Window<C, ToolbarContent<V>> {
        self.map(
            |content| content,
            |_| ToolbarContent(Installed::new(toolbar, LabelDisplayMode::IconOnly)),
        )
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
    /// use waterui::window::Window;
    ///
    /// // Semi-transparent colored window
    /// let tinted = Window::new("Tinted", || text!("Hello"))
    ///     .background(Color::srgb(0, 0, 0).with_opacity(0.8));
    ///
    /// // Frosted glass window: the desktop shows through, blurred on macOS;
    /// // on Hydrolysis tinted, and blurred where the compositor
    /// // supports it
    /// let frosted = Window::new("Frosted", || text!("Hello"))
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
    pub fn build_content(&self) -> C::Output
    where
        C: ViewBuilder,
    {
        self.content.build()
    }

    /// The same window with its content and toolbar types erased, see
    /// [`AnyWindow`].
    pub fn erase(self) -> AnyWindow
    where
        C: ViewBuilder,
        T: WindowToolbar,
    {
        self.map(
            |content| AnyViewBuilder::new(move || AnyView::new(content.build())),
            WindowToolbar::erase,
        )
    }

    /// Set the title of the window.
    ///
    /// Accepts any [`IntoText`] value so string literals resolve through the
    /// active translation catalog in the application's [`Environment`].
    #[must_use]
    pub fn title(mut self, title: impl IntoText) -> Self {
        self.title = title.into_text();
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
    pub fn display_title(&self, env: &Environment) -> Computed<Str> {
        let application = application_name();
        self.title
            .resolve(env)
            .content()
            .map(move |declared| {
                let declared = declared.to_plain();
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
        // A handler whose parameters cannot be extracted does not run: the
        // failure is reported and the request is answered as if the window
        // had no handler, which closes it.
        self.close_request
            .set_hook(Box::new(move |env| match action(env) {
                Ok(question) => Box::pin(question) as Question,
                Err(error) => {
                    error.report();
                    Box::pin(core::future::ready(CloseReply::Close)) as Question
                }
            }));
        self
    }

    /// Arms close requests with the environment the window renders under —
    /// a backend calls it when it realizes the window, so
    /// [`WindowHandle::request_close`] can file before any platform request
    /// arrives.
    #[doc(hidden)]
    pub fn arm_close_requests(&self, env: &Environment) {
        let window_env = env.extending(EnvState(self.handle()));
        self.close_request.arm(&window_env, self.closable);
    }

    /// Arms focus requests with `hook` — the platform call
    /// [`WindowHandle::focus`] dispatches into. A backend calls it when it
    /// realizes the window; a platform that cannot activate a mapped window
    /// leaves the machine unarmed and the call panics.
    #[doc(hidden)]
    pub fn arm_focus_requests(&self, hook: impl FnMut() + 'static) {
        self.focus_request.arm(hook);
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
            close_request: self.close_request.downgrade(),
            focus_request: self.focus_request.clone(),
        }
    }

    /// Show the window on screen through `manager`.
    ///
    /// Every call opens a fresh platform window — the right tool for
    /// unbounded document or scratch windows, or windows opened imperatively
    /// from handlers after the last window closed under
    /// [`LastWindowPolicy::StayResident`](crate::app::LastWindowPolicy::StayResident).
    /// Pass `manager`'s paired [`WindowChannel`] to
    /// [`App::window`](crate::app::App::window) so the runner receives the
    /// window with its content type intact. For a single-instance secondary
    /// window that opens once and focuses on repeat, declare it directly on
    /// [`App::window`](crate::app::App::window) with [`Window::state`] instead.
    ///
    /// # Contract
    ///
    /// Showing a window after `manager`'s [`WindowChannel`] was dropped
    /// without connecting to a runner breaks the contract: a debug build
    /// panics, and a release build logs the error and shows nothing.
    pub fn show(self, manager: &WindowManager<C, T>)
    where
        C: ViewBuilder,
        T: WindowToolbar,
    {
        manager.show(self);
    }
}

impl<C, T> WindowParts<C, T> {
    /// The same parts with `content` replaced by `f(content)`.
    pub fn map_content<D>(self, f: impl FnOnce(C) -> D) -> WindowParts<D, T> {
        self.map(f, |toolbar| toolbar)
    }

    /// The same parts with `content` replaced by `f(content)` and `toolbar`
    /// replaced by `g(toolbar)`.
    pub fn map<D, U>(self, f: impl FnOnce(C) -> D, g: impl FnOnce(T) -> U) -> WindowParts<D, U> {
        let Self {
            title,
            closable,
            resizable,
            frame,
            content,
            state,
            toolbar,
            style,
            background,
            min_size,
            max_size,
            app_id,
            instance_name,
            placement,
            activation,
            level,
            attention,
            resize_increments,
            icon,
            close_request,
            focus_request,
        } = self;
        WindowParts {
            title,
            closable,
            resizable,
            frame,
            content: f(content),
            state,
            toolbar: g(toolbar),
            style,
            background,
            min_size,
            max_size,
            app_id,
            instance_name,
            placement,
            activation,
            level,
            attention,
            resize_increments,
            icon,
            close_request,
            focus_request,
        }
    }

    /// What a backend realizes behind the window's content, following the
    /// reactive [`Self::background`]. See [`resolve_background`].
    #[must_use]
    pub fn resolved_background(&self, env: &Environment) -> Computed<ResolvedWindowBackground> {
        resolve_background(&self.background, env)
    }

    /// The title to put on the window, see [`Window::display_title`].
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

    /// The identity to give the window, see [`Window::display_app_id`].
    #[must_use]
    pub fn display_app_id(&self) -> Str {
        self.app_id.clone().unwrap_or_else(application_identifier)
    }

    /// The window's resolved instance name, see [`Window::display_instance_name`].
    #[must_use]
    pub fn display_instance_name(&self) -> Str {
        self.instance_name
            .clone()
            .unwrap_or_else(|| self.display_app_id())
    }

    /// Arms close requests with `env` extended by this window's [`WindowHandle`].
    #[doc(hidden)]
    pub fn arm_close_requests(&self, env: &Environment) {
        let window_env = env.extending(EnvState(self.handle()));
        self.close_request.arm(&window_env, self.closable);
    }

    /// Arms focus requests with `hook`, see [`Window::arm_focus_requests`].
    #[doc(hidden)]
    pub fn arm_focus_requests(&self, hook: impl FnMut() + 'static) {
        self.focus_request.arm(hook);
    }

    /// Files a user close request, see [`Window::request_close`].
    #[doc(hidden)]
    pub fn request_close(&self, env: &Environment) {
        self.arm_close_requests(env);
        self.close_request.request();
    }

    /// Whether an [`Window::on_close_request`] handler is installed.
    #[doc(hidden)]
    #[must_use]
    pub fn has_close_handler(&self) -> bool {
        self.close_request.has_handler()
    }

    /// Get a handle to control this window.
    #[must_use]
    pub fn handle(&self) -> WindowHandle {
        WindowHandle {
            frame: self.frame.clone(),
            state: self.state.clone(),
            attention: self.attention.clone(),
            style: self.style.clone(),
            background: self.background.clone(),
            icon: self.icon.clone(),
            close_request: self.close_request.downgrade(),
            focus_request: self.focus_request.clone(),
        }
    }
}

/// A handle to control a window from its own content or from outside it.
///
/// Every backend installs a window's `WindowHandle` into the environment of
/// its content, toolbar, and [`Window::on_close_request`] handler, so any
/// handler inside the window can extract it directly:
///
/// ```rust
/// use waterui::prelude::*;
/// use waterui::window::WindowHandle;
///
/// fn controls() -> impl View {
///     button("Maximize").action(|window: WindowHandle| window.maximize())
/// }
/// ```
#[waterui_macros::state]
#[derive(Debug, Clone)]
pub struct WindowHandle {
    frame: Binding<Rect>,
    state: Binding<WindowState>,
    attention: Binding<Option<UserAttention>>,
    style: Binding<WindowStyle>,
    background: Binding<WindowBackground>,
    icon: Binding<Option<WindowIcon>>,
    close_request: WeakCloseRequest,
    focus_request: FocusRequest,
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

    /// Bring the window in front and give it keyboard focus — the classic
    /// "show me the existing settings window" request.
    ///
    /// Platform behaviour:
    ///
    /// - **macOS:** a miniaturized window deminiaturizes first, then
    ///   `makeKeyAndOrderFront:` and `NSApp.activate()` — the cooperative
    ///   activation macOS 14 introduced, which the system may still
    ///   decline.
    /// - **iOS:** `makeKeyAndVisible` — the window becomes its scene's
    ///   key window.
    /// - **Hydrolysis/winit:** `focus_window` — X11
    ///   `_NET_ACTIVE_WINDOW` and Windows `SetForegroundWindow` behind
    ///   it; the window manager may still decline, and on Wayland an
    ///   activation token is required, which only exists when the request
    ///   follows a user gesture — otherwise the request is best-effort.
    /// - **Headless/semantic runners:** unarmed; the call panics.
    ///
    /// A handle that outlived its window's teardown files into nothing:
    /// the request is a no-op.
    ///
    /// # Panics
    ///
    /// When the window has not been realized yet, and on the platforms
    /// with no mapped-window activation — headless and semantic runners.
    pub fn focus(&self) {
        self.focus_request.request();
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
        ResolvedWindowBackground, Window, WindowBackground, WindowIcon, WindowIconError,
        WindowManager, WindowState, resolve_background,
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

    /// `Window::new` defaults to `WindowState::Normal`, and `.state(&binding)`
    /// rebinds both the window's state and its close-request machine.
    #[test]
    fn window_new_defaults_to_normal_and_state_rebinds_close_requests() {
        let default_window = Window::new("Notes", || ());
        assert_eq!(default_window.state.snapshot(), WindowState::Normal);

        let external = Binding::new(WindowState::Normal);
        let window = Window::new("Settings", || ()).state(&external);
        assert_eq!(window.state.snapshot(), WindowState::Normal);
        window.request_close(&Environment::new());
        assert_eq!(external.snapshot(), WindowState::Closed);
    }

    /// `Window::background(Material)` stores the material as the window's
    /// background instead of wrapping the content.
    #[test]
    fn a_material_is_the_window_background() {
        let window = Window::new("", || ()).background(Material::Thin);
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
        let accent = Binding::new(green);
        let mut env = Environment::new();
        Theme::new()
            .colors(
                ColorSettings::new()
                    .background(from_linear_srgb([0.5, 0.5, 0.5], 1.0))
                    .accent(accent.clone()),
            )
            .install(&mut env);

        let background = Binding::new(WindowBackground::Color(Color::srgb(255, 0, 0)));
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

    /// Showing a window after its channel was dropped shows nothing.
    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "WindowChannel was dropped"))]
    fn showing_a_window_on_a_dropped_channel_is_dropped() {
        let state = Binding::new(WindowState::Closed);
        let (manager, channel) = WindowManager::new();
        drop(channel);
        Window::new("Orphan", || ()).state(&state).show(&manager);
        assert_eq!(state.snapshot(), WindowState::Closed);
    }

    /// `WindowManager` and `WindowChannel` carry the concrete content and
    /// toolbar types to the runner, preserving FIFO order across pre-connect
    /// queueing and re-entrant `show` calls.
    #[test]
    fn window_channel_delivers_typed_windows_in_order() {
        fn body(label: &'static str) -> impl super::ViewBuilder<Output = &'static str> {
            move || label
        }

        let (manager, channel) = WindowManager::new();
        Window::new("first", body("a")).show(&manager);
        assert_eq!(channel.queued_len(), 1);

        let received = Rc::new(RefCell::new(Vec::new()));
        let nested = std::cell::Cell::new(false);
        let queued = channel.connect({
            let received = Rc::clone(&received);
            let manager = manager.clone();
            move |window| {
                let built: &'static str = window.build_content().0;
                let title = window
                    .title
                    .resolve(&Environment::new())
                    .content()
                    .snapshot()
                    .to_plain();
                received.borrow_mut().push((title, built));
                if !nested.replace(true) {
                    Window::new("third", body("c")).show(&manager);
                }
            }
        });
        assert_eq!(queued.len(), 1);
        assert_eq!(
            queued[0]
                .title
                .resolve(&Environment::new())
                .content()
                .snapshot()
                .to_plain()
                .as_str(),
            "first"
        );
        assert_eq!(queued[0].build_content().0, "a");

        Window::new("second", body("b")).show(&manager);
        let log: Vec<_> = received
            .borrow()
            .iter()
            .map(|(title, body)| (title.as_str().to_owned(), *body))
            .collect();
        assert_eq!(
            log,
            vec![("second".to_owned(), "b"), ("third".to_owned(), "c")]
        );
    }

    #[test]
    fn window_builders_set_closable_default_size_and_inject_window_handle() {
        use super::{WindowHandle, WindowRoot};
        use waterui_core::extract::Extractor as _;
        use waterui_core::handler::ViewBuilder as _;
        use waterui_core::layout::{Point, Rect, Size};
        use waterui_core::metadata::Metadata;
        use waterui_core::{AnyView, View as _};

        let state = Binding::new(WindowState::Normal);
        let parts = Window::new("Controls", || ())
            .state(&state)
            .closable(false)
            .default_size(Size::new(1024.0, 768.0))
            .into_parts(&Environment::new());

        assert!(!parts.closable);
        assert_eq!(
            parts.frame.snapshot(),
            Rect::new(Point::zero(), Size::new(1024.0, 768.0))
        );

        let wrapped = parts.content.build();
        let metadata = AnyView::new(wrapped.body(&Environment::new()))
            .downcast::<Metadata<Environment, WindowRoot<()>>>()
            .expect("WindowHandleView installs WindowHandle via Metadata<Environment>");
        let handle = WindowHandle::extract(&metadata.value)
            .expect("Window::into_parts installs WindowHandle as handler state");
        handle.maximize();
        assert_eq!(state.snapshot(), WindowState::Maximized);
    }

    #[test]
    fn window_title_resolves_through_translation_catalog_and_a11y_metadata() {
        use waterui_core::metadata::Metadata;
        use waterui_locale::{TranslationCatalog, locales};

        use crate::ViewExt as _;

        let locale = Binding::new(locales::EN);
        let mut env = Environment::new();
        env.insert(locale.clone());
        env.insert(
            TranslationCatalog::new()
                .add_toml(
                    locales::FR,
                    "\"Settings\" = \"Réglages\"\n\"Close window\" = \"Fermer la fenêtre\"\n\"Ready\" = \"Prêt\"",
                )
                .expect("test catalog must parse"),
        );

        let parts = Window::new("Settings", || ()).into_parts(&env);
        assert_eq!(parts.title.snapshot().as_str(), "Settings");

        let Metadata {
            value: a11y_label, ..
        } = ().a11y_label("Close window");
        let Metadata {
            value: a11y_value, ..
        } = ().a11y_value("Ready");
        let label_signal = a11y_label.resolve(&env);
        let value_signal = a11y_value.resolve(&env);
        assert_eq!(label_signal.snapshot().as_str(), "Close window");
        assert_eq!(value_signal.snapshot().as_str(), "Ready");

        locale.set(locales::FR);
        assert_eq!(parts.title.snapshot().as_str(), "Réglages");
        assert_eq!(label_signal.snapshot().as_str(), "Fermer la fenêtre");
        assert_eq!(value_signal.snapshot().as_str(), "Prêt");
    }

    #[test]
    fn window_toolbar_keeps_concrete_type_without_erasing() {
        use super::WindowToolbar as _;

        let unset = Window::new("Plain", || ());
        let _: () = unset.toolbar;
        assert!(().into_view().is_none());
        assert!(().erase().is_none());

        let (manager, channel) = WindowManager::new();
        Window::new("With Toolbar", || ())
            .toolbar(crate::widget::Divider)
            .show(&manager);
        let mut queued = channel.drain();
        assert_eq!(queued.len(), 1);
        let delivered = queued.pop().expect("one queued window");
        let toolbar: super::ToolbarContent<_> = delivered.toolbar;
        assert!(toolbar.into_view().is_some());
    }

    /// The icon validates its pixel buffer against its size.
    #[test]
    fn window_icon_rejects_a_short_buffer() {
        let error = WindowIcon::new(2, 1, vec![0_u8; 4]).expect_err("4 bytes cannot fill 2x1");
        assert_eq!(error.to_string(), "a 2x1 icon takes 8 RGBA8 bytes, got 4");
    }

    /// An icon without pixels is rejected rather than handed to a platform.
    #[test]
    fn window_icon_rejects_an_empty_icon() {
        assert_eq!(
            WindowIcon::new(0, 4, Vec::<u8>::new()),
            Err(WindowIconError::Empty {
                width: 0,
                height: 4
            })
        );
    }

    /// Icons compare by their pixels, whether or not they share them.
    #[test]
    fn window_icons_compare_by_pixels() {
        let icon = WindowIcon::new(1, 1, vec![1_u8, 2, 3, 4]).expect("one pixel");
        assert_eq!(icon, icon.clone());
        assert_eq!(
            icon,
            WindowIcon::new(1, 1, vec![1_u8, 2, 3, 4]).expect("one pixel")
        );
        assert_ne!(
            icon,
            WindowIcon::new(1, 1, vec![1_u8, 2, 3, 5]).expect("one pixel")
        );
    }

    #[test]
    fn window_root_does_not_install_snackbar_manager() {
        use core::any::{Any, TypeId};

        use super::WindowRoot;
        use crate::View;
        #[cfg(feature = "snackbar")]
        use crate::ViewExt as _;
        #[cfg(feature = "snackbar")]
        use crate::snackbar::SnackbarManager;

        struct Probe;

        impl View for Probe {
            fn body(self, _env: &Environment) -> impl View {}
        }

        let env = Environment::new();
        let root_body = WindowRoot(Probe).body(&env);
        assert_eq!(Any::type_id(&root_body), TypeId::of::<Probe>());

        #[cfg(feature = "snackbar")]
        {
            let _snackbar_host = Probe.snackbar_host();
            assert!(env.get::<SnackbarManager>().is_none());
        }
    }
}
