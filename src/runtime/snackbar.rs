//! Snackbar notification system for `WaterUI`.
//!
//! Snackbars provide brief messages about app processes at the bottom of the screen.
//! They automatically disappear after a timeout and can optionally include an action button.
//!
//! # Examples
//!
//! Attach [`ViewExt::snackbar_host`] to a root view and extract
//! [`SnackbarManager`] in any descendant handler:
//!
//! ```rust
//! use core::time::Duration;
//!
//! use waterui::prelude::*;
//! use waterui::snackbar::{Snackbar, SnackbarManager, SnackbarPosition};
//!
//! fn view() -> impl View {
//!     vstack((
//!         // Show a simple message
//!         button("Save").action(|manager: SnackbarManager| {
//!             manager.show(Snackbar::new("File saved"));
//!         }),
//!         // With action and custom duration
//!         button("Delete").action(|manager: SnackbarManager| {
//!             manager.show(
//!                 Snackbar::new("Item deleted")
//!                     .action("Undo", || {})
//!                     .duration(Duration::from_secs(5)),
//!             );
//!         }),
//!         // Custom position
//!         button("Reconnect").action(|manager: SnackbarManager| {
//!             manager.show(
//!                 Snackbar::new("Connection restored")
//!                     .position(SnackbarPosition::TopCenter),
//!             );
//!         }),
//!     ))
//!     .snackbar_host()
//! }
//! ```

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use core::fmt::{self, Debug, Formatter};
use core::marker::PhantomData;
use core::time::Duration;

use executor_core::spawn_local;
use nami::Binding;
use nami::collection::StateList;
use waterui_controls::{Button, ButtonStyle, IntoLabel, Label, SemanticLabel, button, label};
use waterui_core::animation::Animation;
use waterui_core::extract::State;
use waterui_core::handler::{
    BoxedEventAction, EventHandler, Handler, ReportExtraction, SharedAction, ViewBuilder,
    boxed_event_handler, shared_action,
};
use waterui_core::plugin::Plugin;
use waterui_core::views::ForEach;
use waterui_core::{Environment, OneOf2, SignalExt, View};
use waterui_layout::container::{FixedContainer, LazyContainer};
use waterui_layout::frame::Frame;
use waterui_layout::padding::EdgeInsets;
use waterui_layout::spacer::spacer;
use waterui_layout::stack::{Alignment, hstack};
use waterui_layout::{
    AbsoluteLayout, Layout, LayoutError, PlacementInput, ProposalSize, Rect, Size, StretchAxis,
    SubView, SubviewPlacement, UnitPoint,
};
use waterui_macros::state;
#[cfg(feature = "flow-markdown")]
use waterui_markdown::CodeCopyFeedback;
#[cfg(feature = "flow-markdown")]
use waterui_text::code::OnCopied;
use waterui_text::{IntoText, Text, font::Font};

use crate::ViewExt;
use crate::shape::{FilledShape, Path, RoundedRectangle, ShapeExt};
use crate::style::Shadow;
use waterui_graphics::color::Color;

/// Maximum number of snackbars visible at once *per placement*. M3 keeps each
/// on-screen stack small; when a new one arrives over this limit at the same
/// placement, the oldest at that placement is dismissed.
const MAX_VISIBLE_SNACKBARS: usize = 3;

/// Vertical gap between stacked snackbars, in logical units.
const SNACKBAR_STACK_GAP: f32 = 8.0;

/// The grid the close-icon geometry below is authored on, matching the 24dp
/// viewport every Material icon is drawn in.
const CLOSE_ICON_GRID: f32 = 24.0;

/// Outline of Material's filled `close` icon on the [`CLOSE_ICON_GRID`], as the
/// twelve corners of the two crossed bars. This is the geometry Compose draws for
/// `Icons.Filled.Close`, which is what `androidx.compose.material3`'s snackbar
/// puts in its `dismissAction` slot.
///
/// Drawing the real icon rather than a text glyph is what makes the affordance
/// read at its intended size: a font's multiplication sign inks barely half its
/// point size, so the previous 22pt "×" rendered as a ~10pt mark.
const CLOSE_ICON_OUTLINE: [(f32, f32); 12] = [
    (19.0, 6.41),
    (17.59, 5.0),
    (12.0, 10.59),
    (6.41, 5.0),
    (5.0, 6.41),
    (10.59, 12.0),
    (5.0, 17.59),
    (6.41, 19.0),
    (12.0, 13.41),
    (17.59, 19.0),
    (19.0, 17.59),
    (13.41, 12.0),
];

/// Builds the close icon as a unit-square path, so it scales with whatever frame
/// the theme sizes it into.
fn close_icon_path() -> Path {
    let mut corners = CLOSE_ICON_OUTLINE
        .into_iter()
        .map(|(x, y)| (x / CLOSE_ICON_GRID, y / CLOSE_ICON_GRID));
    let (start_x, start_y) = corners
        .next()
        .expect("close icon outline is a non-empty constant");
    corners
        .fold(Path::new().move_to(start_x, start_y), |path, (x, y)| {
            path.line_to(x, y)
        })
        .close()
}

/// The close affordance's visual: the icon drawn at `icon_size`, centered in the
/// `box_size` square that carries the button's hit area.
#[derive(Debug, Clone)]
struct CloseIcon {
    color: Color,
    icon_size: f32,
    box_size: f32,
}

impl View for CloseIcon {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        FilledShape::new(close_icon_path(), self.color)
            .size(self.icon_size, self.icon_size)
            .size(self.box_size, self.box_size)
    }
}

/// Theme tokens used by the snackbar overlay.
#[derive(Debug, Clone)]
pub struct SnackbarTheme {
    /// Container fill color.
    pub container_color: Color,
    /// Supporting text and icon color.
    pub supporting_text_color: Color,
    /// Action label color.
    pub action_label_color: Color,
    /// Supporting text font.
    pub supporting_text_font: Font,
    /// Action label font.
    pub action_label_font: Font,
    /// Inner content padding.
    pub content_padding: EdgeInsets,
    /// Margin between the bar and the window's safe area.
    ///
    /// This is spacing only. The backend places the host clear of the
    /// hardware — notch, status bar, home indicator — so a theme never has to
    /// guess those insets.
    pub viewport_padding: EdgeInsets,
    /// Gap between message and action.
    pub content_spacing: f32,
    /// Minimum container width.
    pub min_width: f32,
    /// Maximum container width.
    pub max_width: f32,
    /// Trailing padding when an action button is present.
    pub action_trailing_padding: f32,
    /// Drawn size of the close icon (Material icons are drawn at 24dp).
    pub close_icon_size: f32,
    /// Box the close icon is centered in, which is also its hit area.
    ///
    /// This is Compose's `IconButtonTokens.StateLayerSize`, the size an
    /// `IconButton` actually *measures*. Compose then widens only the touch area
    /// to 48dp via `minimumInteractiveComponentSize()`, which does not affect
    /// layout; `WaterUI` has no equivalent yet, so hit area is layout size here.
    /// Sizing this box to 48dp instead would push a single-line snackbar to 72dp
    /// tall, well past Material's 48dp.
    pub close_state_layer_size: f32,
    /// Minimum height for single-line snackbars.
    pub single_line_min_height: f32,
    /// Container corner radius in logical units — the same length traces the
    /// fill and the shadows.
    pub corner_radius: f32,
    /// Shadow color.
    pub shadow_color: Color,
    /// Shadow blur radius.
    pub shadow_radius: f32,
    /// Shadow vertical offset.
    pub shadow_offset_y: f32,
    /// Ambient shadow color.
    pub ambient_shadow_color: Color,
    /// Ambient shadow blur radius.
    pub ambient_shadow_radius: f32,
    /// Ambient shadow vertical offset.
    pub ambient_shadow_offset_y: f32,
    /// Entrance and dismissal vertical travel.
    pub motion_offset_y: f32,
    /// Entrance animation.
    pub enter_animation: Animation,
    /// Dismissal animation.
    pub exit_animation: Animation,
}

impl SnackbarTheme {
    /// The Material 3 snackbar: a full-width bar pinned near the bottom edge,
    /// leading-aligned text, small corner radius.
    #[must_use]
    pub fn material() -> Self {
        Self {
            container_color: Color::srgb(0x32, 0x2f, 0x35),
            supporting_text_color: Color::srgb(0xf5, 0xef, 0xf7),
            action_label_color: Color::srgb(0xd0, 0xbc, 0xff),
            supporting_text_font: Font::default(),
            action_label_font: Font::default(),
            content_padding: EdgeInsets::symmetric(12.0, 16.0),
            viewport_padding: EdgeInsets::all(16.0),
            content_spacing: 12.0,
            min_width: 288.0,
            max_width: 568.0,
            action_trailing_padding: 8.0,
            close_icon_size: 24.0,
            close_state_layer_size: 40.0,
            single_line_min_height: 48.0,
            corner_radius: 4.0,
            shadow_color: Color::srgb(0, 0, 0).with_opacity(0.2),
            shadow_radius: 3.0,
            shadow_offset_y: 3.0,
            ambient_shadow_color: Color::srgb(0, 0, 0).with_opacity(0.0),
            ambient_shadow_radius: 0.0,
            ambient_shadow_offset_y: 0.0,
            motion_offset_y: 20.0,
            enter_animation: Animation::bezier(Duration::from_millis(250), 0.0, 0.0, 0.0, 1.0),
            exit_animation: Animation::bezier(Duration::from_millis(200), 0.3, 0.0, 1.0, 1.0),
        }
    }

    /// The Apple confirmation banner: the compact rounded capsule Apple Music
    /// shows for "Added to Library" — content-sized, centered, generously
    /// rounded, floating above the bottom edge with a soft shadow.
    #[must_use]
    pub fn apple() -> Self {
        Self {
            container_color: Color::srgb(0x2c, 0x2c, 0x2e),
            supporting_text_color: Color::srgb(0xff, 0xff, 0xff),
            action_label_color: Color::srgb(0x0a, 0x84, 0xff),
            supporting_text_font: Font::default(),
            action_label_font: Font::default(),
            content_padding: EdgeInsets::symmetric(14.0, 24.0),
            viewport_padding: EdgeInsets::all(16.0),
            content_spacing: 10.0,
            // Content-sized: no minimum, so the banner hugs its text.
            min_width: 0.0,
            max_width: 480.0,
            action_trailing_padding: 12.0,
            close_icon_size: 18.0,
            close_state_layer_size: 32.0,
            single_line_min_height: 50.0,
            corner_radius: 15.0,
            shadow_color: Color::srgb(0, 0, 0).with_opacity(0.18),
            shadow_radius: 18.0,
            shadow_offset_y: 6.0,
            ambient_shadow_color: Color::srgb(0, 0, 0).with_opacity(0.06),
            ambient_shadow_radius: 2.0,
            ambient_shadow_offset_y: 1.0,
            motion_offset_y: 16.0,
            enter_animation: Animation::bezier(Duration::from_millis(280), 0.2, 0.0, 0.0, 1.0),
            exit_animation: Animation::bezier(Duration::from_millis(220), 0.4, 0.0, 1.0, 1.0),
        }
    }
}

/// The platform's own notification style: Apple platforms show the compact
/// Apple-style confirmation banner, everything else the Material 3 bar. Both
/// remain available by name — the default only picks which one the platform
/// itself would draw.
impl Default for SnackbarTheme {
    fn default() -> Self {
        #[cfg(any(target_os = "ios", target_os = "macos", target_os = "tvos"))]
        {
            Self::apple()
        }
        #[cfg(not(any(target_os = "ios", target_os = "macos", target_os = "tvos")))]
        {
            Self::material()
        }
    }
}

/// Position for snackbar display.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnackbarPosition {
    /// Bottom center of the screen (default, Material Design standard).
    #[default]
    BottomCenter,
    /// Top center of the screen (iOS-style notification banner).
    TopCenter,
    /// Bottom leading (left in LTR).
    BottomLeading,
    /// Bottom trailing (right in LTR).
    BottomTrailing,
}

impl SnackbarPosition {
    /// Converts position to layout alignment.
    const fn to_alignment(self) -> Alignment {
        match self {
            Self::BottomCenter => Alignment::Bottom,
            Self::TopCenter => Alignment::Top,
            Self::BottomLeading => Alignment::BottomLeading,
            Self::BottomTrailing => Alignment::BottomTrailing,
        }
    }

    /// Returns the initial Y offset for the entrance slide (hidden position):
    /// below the edge for bottom placements, above it for top placements.
    const fn initial_offset_y(self) -> f32 {
        if self.is_top() { -20.0 } else { 20.0 }
    }

    /// Whether this placement anchors to the top edge.
    const fn is_top(self) -> bool {
        matches!(self, Self::TopCenter)
    }

    /// The sign of the reflow stack offset: a bottom stack grows upward
    /// (negative y), a top stack grows downward (positive y).
    const fn stack_direction(self) -> f32 {
        if self.is_top() { 1.0 } else { -1.0 }
    }
}

/// An action button for the snackbar.
#[derive(Clone)]
pub struct SnackbarAction {
    /// The action button label.
    label: Label<SemanticLabel>,
    /// The callback when action is pressed.
    handler: SharedAction<()>,
}

impl SnackbarAction {
    /// The semantic action button label.
    #[must_use]
    pub const fn label(&self) -> &Label<SemanticLabel> {
        &self.label
    }
}

impl core::fmt::Debug for SnackbarAction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SnackbarAction")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// Why a snackbar presentation was dismissed, reported to
/// [`Snackbar::on_dismiss`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnackbarDismissReason {
    /// The snackbar's action button was pressed.
    Action,
    /// The auto-dismiss duration elapsed.
    Timeout,
    /// The trailing close button was pressed.
    Closed,
    /// The presentation was evicted to make room for a newer snackbar at the
    /// same placement.
    Replaced,
    /// The presentation was dismissed programmatically through
    /// [`SnackbarManager::dismiss`].
    Programmatic,
}

/// The snackbar's dismissal callback, shared by reference across every
/// presentation of a cloned [`Snackbar`].
#[derive(Clone)]
struct DismissalCallback(Rc<RefCell<BoxedEventAction<SnackbarDismissReason>>>);

impl DismissalCallback {
    fn call(&self, reason: SnackbarDismissReason, env: &waterui_core::Environment) {
        (self.0.borrow_mut())(reason, env).report_extraction();
    }
}

impl core::fmt::Debug for DismissalCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DismissalCallback").finish_non_exhaustive()
    }
}

/// Cloneable factory that constructs a snackbar's leading icon view on demand.
#[doc(hidden)]
pub struct IconFactory<I>(Rc<dyn Fn() -> I>);

impl<I> Clone for IconFactory<I> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<I> Debug for IconFactory<I> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("IconFactory").finish_non_exhaustive()
    }
}

/// Cloneable view adapter backed by an [`IconFactory`], allowing non-`Clone`
/// views inside a snackbar's `Label::icon` slot.
#[doc(hidden)]
pub struct BuiltIcon<V>(IconFactory<V>);

impl<V> Clone for BuiltIcon<V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<V> Debug for BuiltIcon<V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("BuiltIcon").finish_non_exhaustive()
    }
}

impl<V: View> View for BuiltIcon<V> {
    fn body(self, _env: &Environment) -> impl View {
        (self.0.0)()
    }
}

/// Configuration for a snackbar notification.
///
/// Use the builder pattern to configure the snackbar before showing it.
///
/// # Examples
///
/// ```rust
/// use core::time::Duration;
///
/// use waterui::snackbar::Snackbar;
///
/// let snackbar = Snackbar::new("Message sent")
///     .duration(Duration::from_secs(3));
/// ```
pub struct Snackbar<I = NoIcon> {
    /// The message to display.
    message: Text,
    /// Optional icon builder to display before the message.
    icon: Option<IconFactory<I>>,
    /// Optional action button.
    action: Option<SnackbarAction>,
    /// Display duration before auto-dismiss.
    duration: Duration,
    /// Position on screen.
    position: SnackbarPosition,
    /// Whether to show a trailing close button (M3 `closeable`).
    closeable: bool,
    /// Callback reporting each presentation's dismissal and its reason.
    dismissal: Option<DismissalCallback>,
    /// State injected into the action button environment.
    captured_env: waterui_core::Environment,
}

impl<I> Clone for Snackbar<I> {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            icon: self.icon.clone(),
            action: self.action.clone(),
            duration: self.duration,
            position: self.position,
            closeable: self.closeable,
            dismissal: self.dismissal.clone(),
            captured_env: self.captured_env.clone(),
        }
    }
}

impl<I> core::fmt::Debug for Snackbar<I> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Snackbar")
            .field("message", &self.message)
            .field("has_icon", &self.icon.is_some())
            .field("action", &self.action)
            .field("duration", &self.duration)
            .field("position", &self.position)
            .field("closeable", &self.closeable)
            .field("dismissal", &self.dismissal)
            .field("captured_env", &self.captured_env)
            .finish()
    }
}

/// Marker type for a [`Snackbar`] with no leading icon.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoIcon;

mod icon_sealed {
    pub trait Sealed {}
}

/// Leading-icon type accepted by [`Snackbar`], [`SnackbarManager`], and [`SnackbarHost`]:
/// either [`NoIcon`] or any [`View`].
#[doc(hidden)]
pub trait SnackbarIconSlot: icon_sealed::Sealed + Sized + 'static {
    type IconView: View + Clone;

    fn into_icon_view(factory: IconFactory<Self>) -> Option<Self::IconView>;

    fn from_plain(snackbar: Snackbar<NoIcon>) -> Snackbar<Self>;
}

impl icon_sealed::Sealed for NoIcon {}

impl SnackbarIconSlot for NoIcon {
    type IconView = ();

    fn into_icon_view(_factory: IconFactory<Self>) -> Option<Self::IconView> {
        None
    }

    fn from_plain(snackbar: Snackbar<NoIcon>) -> Snackbar<Self> {
        snackbar
    }
}

impl<V: View + 'static> icon_sealed::Sealed for V {}

impl<V: View + 'static> SnackbarIconSlot for V {
    type IconView = BuiltIcon<Self>;

    fn into_icon_view(factory: IconFactory<Self>) -> Option<Self::IconView> {
        Some(BuiltIcon(factory))
    }

    fn from_plain(snackbar: Snackbar<NoIcon>) -> Snackbar<Self> {
        snackbar.into()
    }
}

impl<V: View> From<Snackbar<NoIcon>> for Snackbar<V> {
    fn from(snackbar: Snackbar<NoIcon>) -> Self {
        Self {
            message: snackbar.message,
            icon: None,
            action: snackbar.action,
            duration: snackbar.duration,
            position: snackbar.position,
            closeable: snackbar.closeable,
            dismissal: snackbar.dismissal,
            captured_env: snackbar.captured_env,
        }
    }
}

impl Snackbar<NoIcon> {
    /// Creates a new snackbar with the specified message.
    ///
    /// Accepts any [`IntoText`] value so string literals resolve through the
    /// active translation catalog in the surrounding [`Environment`].
    #[must_use]
    pub fn new(message: impl IntoText) -> Self {
        Self {
            message: message.into_text(),
            icon: None,
            action: None,
            duration: Duration::from_secs(3),
            position: SnackbarPosition::default(),
            closeable: false,
            dismissal: None,
            captured_env: waterui_core::Environment::new(),
        }
    }
}

impl<I> Snackbar<I> {
    /// Adds a leading icon builder to the snackbar.
    ///
    /// Any [`ViewBuilder`] works, including closures returning non-`Clone`
    /// modified views (for example `|| mdi::check_circle().padding()`). For
    /// portable apps prefer a packaged icon crate such as
    /// `waterui-icons-material-icon` (`waterui_apple::SfSymbol` is only
    /// available on Apple backends). Pair with
    /// [`ViewExt::snackbar_host_with_icons`] or [`SnackbarManager::host`] when
    /// showing snackbars that carry an icon of type `B::Output`.
    #[must_use]
    pub fn icon<B: ViewBuilder>(self, icon: B) -> Snackbar<B::Output> {
        Snackbar {
            message: self.message,
            icon: Some(IconFactory(Rc::new(move || icon.build()))),
            action: self.action,
            duration: self.duration,
            position: self.position,
            closeable: self.closeable,
            dismissal: self.dismissal,
            captured_env: self.captured_env,
        }
    }

    /// Adds an action button to the snackbar.
    ///
    /// # Examples
    ///
    /// Simple action:
    /// ```rust
    /// use waterui::snackbar::Snackbar;
    ///
    /// let snackbar = Snackbar::new("Message sent").action("Undo", || {});
    /// ```
    ///
    /// With injected state:
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::snackbar::Snackbar;
    ///
    /// let items = binding::<Vec<String>>(Vec::new());
    /// let snackbar = Snackbar::new("Item deleted")
    ///     .state(&items)
    ///     .action("Undo", |State(items): State<Binding<Vec<String>>>| {
    ///         items.get_mut().clear();
    ///     });
    /// ```
    #[must_use]
    pub fn action<Args>(
        mut self,
        label: impl IntoLabel,
        handler: impl Handler<Args, ()> + 'static,
    ) -> Self {
        self.action = Some(SnackbarAction {
            label: label.into_label().into_semantic(),
            handler: shared_action(handler),
        });
        self
    }

    /// Registers a handler called when a presentation of this snackbar is
    /// dismissed, with the [`SnackbarDismissReason`] it went away.
    ///
    /// The handler runs exactly once per presentation, after the exit
    /// animation completes and the snackbar has been removed from the stack.
    /// Every dismissal reports through it: `Action` when the action button is
    /// pressed, `Timeout` when the duration elapses, `Closed` from the close
    /// button, `Replaced` when a newer snackbar at the same placement evicts
    /// it, and `Programmatic` for [`SnackbarManager::dismiss`]. Each `show` of
    /// a cloned snackbar is an independent presentation and reports its own
    /// dismissal.
    ///
    /// The reason arrives as the handler's first parameter; values injected
    /// with `.state()` extract as further parameters, exactly like `action`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::snackbar::{Snackbar, SnackbarDismissReason};
    ///
    /// let items = binding::<Vec<String>>(Vec::new());
    /// let snackbar = Snackbar::new("Item deleted")
    ///     .state(&items)
    ///     .on_dismiss(
    ///         |reason: SnackbarDismissReason, State(items): State<Binding<Vec<String>>>| {
    ///             if reason != SnackbarDismissReason::Action {
    ///                 items.get_mut().clear();
    ///             }
    ///         },
    ///     );
    /// ```
    #[must_use]
    pub fn on_dismiss<Args>(
        mut self,
        handler: impl EventHandler<SnackbarDismissReason, Args> + 'static,
    ) -> Self {
        self.dismissal = Some(DismissalCallback(Rc::new(RefCell::new(
            boxed_event_handler(handler),
        ))));
        self
    }

    /// Sets the auto-dismiss duration.
    ///
    /// Default is 3 seconds.
    #[must_use]
    pub const fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Sets the position on screen.
    ///
    /// Default is `SnackbarPosition::BottomCenter`.
    #[must_use]
    pub const fn position(mut self, position: SnackbarPosition) -> Self {
        self.position = position;
        self
    }

    /// Shows a trailing close button that dismisses this snackbar (Material 3
    /// `closeable`). Pair it with `.duration(Duration::ZERO)` for a snackbar that
    /// stays until the user closes it.
    #[must_use]
    pub const fn closeable(mut self) -> Self {
        self.closeable = true;
        self
    }

    /// Injects cloneable state into this snackbar's handler environment,
    /// where both `action` and `on_dismiss` handlers can extract it.
    #[must_use]
    pub fn state<T: Clone + 'static>(mut self, state: &T) -> Self {
        self.captured_env = self.captured_env.extending(State(state.clone()));
        self
    }
}

/// One on-screen snackbar in the reactive stack.
///
/// Every presentation owns its own animation bindings ([`Self::opacity`],
/// [`Self::entrance_offset`], [`Self::stack_offset`]). The overlay renders the
/// stack as a [`ForEach`] over the manager's [`StateList`], so the hydrolysis
/// collection engine dispatches each presentation's subtree exactly once (keyed
/// by [`Self::id`]) and reconciles only the items that join or leave. A
/// presentation that stays on screen is never re-dispatched, so its `on_appear`
/// entrance fires once and its in-flight animations continue uninterrupted —
/// without any one-shot latch.
///
/// Cloning an `ActiveSnackbar` shares those bindings by reference count, so the
/// snapshots the manager reflows from all refer to the same animated state.
#[derive(crate::Identifiable)]
struct ActiveSnackbar<I: 'static = NoIcon> {
    #[id]
    id: u64,
    snackbar: Snackbar<I>,
    /// 0 hidden → 1 shown; drives the fade.
    opacity: Binding<f32>,
    /// Entrance slide offset: starts at the position's hidden offset, animates
    /// to 0 on appear.
    entrance_offset: Binding<f32>,
    /// Reflow offset from the anchor edge (signed: negative pushes a bottom
    /// snackbar up, positive pushes a top snackbar down), animated when the
    /// stack reorders.
    stack_offset: Binding<f32>,
    /// Whether this presentation is running its dismissal fade.
    dismissing: Rc<Cell<bool>>,
}

impl<I: 'static> Clone for ActiveSnackbar<I> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            snackbar: self.snackbar.clone(),
            opacity: self.opacity.clone(),
            entrance_offset: self.entrance_offset.clone(),
            stack_offset: self.stack_offset.clone(),
            dismissing: Rc::clone(&self.dismissing),
        }
    }
}

/// Internal state for the snackbar manager.
struct SnackbarManagerState<I: 'static = NoIcon> {
    /// The active on-screen snackbars, oldest first. A reactive [`StateList`] so the
    /// overlay's [`ForEach`] reconciles the stack item-by-item as snackbars are
    /// added and removed; per-row animation bindings live on each
    /// [`ActiveSnackbar`] and are untouched by membership changes.
    active: StateList<ActiveSnackbar<I>>,
    /// Monotonic presentation identifier used to ignore stale timers.
    next_id: u64,
}

/// Manages snackbar notifications in a FIFO queue.
///
/// Attach [`ViewExt::snackbar_host`] (or [`ViewExt::snackbar_host_with_icons`]
/// / [`SnackbarManager::host`]) to an ancestor view to install the manager and
/// its overlay, then extract `SnackbarManager` in descendant handlers.
///
/// # Examples
///
/// ```rust
/// use waterui::prelude::*;
/// use waterui::snackbar::{Snackbar, SnackbarManager};
///
/// fn my_view() -> impl View {
///     button("Save")
///         .action(|manager: SnackbarManager| {
///             manager.show(Snackbar::new("File saved successfully"));
///         })
///         .snackbar_host()
/// }
/// ```
#[state]
pub struct SnackbarManager<I: 'static = NoIcon> {
    state: Rc<RefCell<SnackbarManagerState<I>>>,
}

impl<I: 'static> Clone for SnackbarManager<I> {
    fn clone(&self) -> Self {
        Self {
            state: Rc::clone(&self.state),
        }
    }
}

impl<I: 'static> Plugin for SnackbarManager<I> {}

impl<I: 'static> core::fmt::Debug for SnackbarManager<I> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SnackbarManager").finish()
    }
}

impl SnackbarManager<NoIcon> {
    /// Creates a new snackbar manager and returns the overlay view.
    #[cfg(test)]
    pub(crate) fn new() -> (Self, impl View + Clone) {
        Self::typed()
    }
}

impl<I: SnackbarIconSlot> SnackbarManager<I> {
    /// Creates a new typed snackbar manager and returns the overlay view.
    pub(crate) fn typed() -> (Self, impl View + Clone) {
        let active = StateList::new();
        let state = Rc::new(RefCell::new(SnackbarManagerState {
            active: active.clone(),
            next_id: 0,
        }));
        let manager = Self { state };

        // The overlay renders the active stack as a ForEach collection; the
        // engine dispatches each snackbar once and reconciles membership
        // incrementally. Each snackbar is a full-bleed Frame self-anchored to
        // its own placement, so mixed placements coexist (see [`ActiveSnackbar`]).
        let overlay = SnackbarOverlay {
            active,
            manager: manager.clone(),
        };
        (manager, overlay)
    }

    /// Builds a view subtree paired with a snackbar overlay layer, inferring `I`
    /// from how `build` uses the handed manager and installing it for descendants.
    pub fn host<R: View>(build: impl FnOnce(Self) -> R) -> impl View {
        let (manager, overlay) = Self::typed();
        let content = build(manager.clone()).overlay(overlay).state(&manager);
        #[cfg(feature = "flow-markdown")]
        let content = content.with(CodeCopyFeedback(OnCopied::new({
            let snackbar = manager;
            move |_env| {
                snackbar.show(I::from_plain(Snackbar::new("Copied to clipboard")));
            }
        })));
        content
    }

    /// Shows a snackbar. Snackbars stack on screen simultaneously, oldest nearest
    /// the anchor edge. Each placement is an independent stack: a snackbar only
    /// interacts with others sharing its [`SnackbarPosition`], and when a
    /// placement already holds `MAX_VISIBLE_SNACKBARS` the oldest *at that
    /// placement* is dismissed to make room — different placements never evict
    /// each other.
    ///
    /// # Panics
    ///
    /// Panics if the snackbar presentation id counter overflows, which takes
    /// more than `u64::MAX` snackbars.
    pub fn show(&self, snackbar: impl Into<Snackbar<I>>) {
        let snackbar = snackbar.into();
        let duration = snackbar.duration;
        let position = snackbar.position;
        let id = {
            let mut state = self.state.borrow_mut();
            state.next_id = state
                .next_id
                .checked_add(1)
                .expect("a u64 id advancing once per shown snackbar does not overflow");
            state.next_id
        };
        let item = ActiveSnackbar {
            id,
            opacity: Binding::new(0.0_f32),
            entrance_offset: Binding::<f32>::new(snackbar.position.initial_offset_y()),
            stack_offset: Binding::new(0.0_f32),
            dismissing: Rc::new(Cell::new(false)),
            snackbar,
        };

        let mut active = self.state.borrow().active.clone();

        // Evict the oldest live snackbar AT THIS PLACEMENT if its stack is
        // already full, so the new one slides in as the oldest fades out.
        // Snackbars at other placements are untouched.
        let snapshot = active.snapshot();
        let live_here = snapshot
            .iter()
            .filter(|item| !item.dismissing.get() && item.snackbar.position == position)
            .count();
        if live_here >= MAX_VISIBLE_SNACKBARS {
            let oldest = snapshot
                .iter()
                .find(|item| !item.dismissing.get() && item.snackbar.position == position)
                .map(|item| item.id);
            if let Some(oldest) = oldest {
                self.dismiss_presentation(oldest, SnackbarDismissReason::Replaced);
            }
        }
        drop(snapshot);

        active.push(item);
        self.reflow();

        // A zero duration disables auto-dismiss (M3 `auto-close-delay = 0`): the
        // snackbar stays until dismissed by its action or close button.
        if duration.is_zero() {
            return;
        }
        let manager = self.clone();
        spawn_local(async move {
            native_executor::sleep(duration).await;
            tracing::debug!("snackbar auto-dismiss timer fired for {id}");
            manager.dismiss_presentation(id, SnackbarDismissReason::Timeout);
        })
        .detach();
    }

    /// Dismisses the most recently shown snackbar.
    pub fn dismiss(&self) {
        let newest = self
            .state
            .borrow()
            .active
            .snapshot()
            .into_iter()
            .rev()
            .find(|item| !item.dismissing.get())
            .map(|item| item.id);
        if let Some(id) = newest {
            self.dismiss_presentation(id, SnackbarDismissReason::Programmatic);
        }
    }

    /// Begins the dismissal fade for one snackbar, then removes it from the
    /// stack and reflows the remainder once the fade completes. The `reason`
    /// of the first dismissal wins: later requests for a presentation already
    /// fading out return here without recording anything.
    fn dismiss_presentation(&self, id: u64, reason: SnackbarDismissReason) {
        let item = self
            .state
            .borrow()
            .active
            .snapshot()
            .into_iter()
            .find(|item| item.id == id);
        let Some(item) = item else {
            return;
        };
        if item.dismissing.replace(true) {
            return;
        }

        item.opacity.set(0.0);
        item.entrance_offset
            .set(item.snackbar.position.initial_offset_y());
        // The dismissing row no longer occupies a slot: reflow now so the
        // survivors animate down to close the gap while it fades out.
        self.reflow();

        let manager = self.clone();
        let exit = SnackbarTheme::default().exit_animation.duration();
        spawn_local(async move {
            native_executor::sleep(exit).await;
            manager.finish_dismissal(id, reason);
        })
        .detach();
    }

    fn finish_dismissal(&self, id: u64, reason: SnackbarDismissReason) {
        let mut active = self.state.borrow().active.clone();
        let index = active.snapshot().iter().position(|item| item.id == id);
        let Some(index) = index else {
            return;
        };
        let removed = active.remove(index);
        self.reflow();

        // The presentation is off the stack and no manager state is borrowed:
        // the callback may itself show or dismiss snackbars.
        if let Some(dismissal) = &removed.snackbar.dismissal {
            dismissal.call(reason, &removed.snackbar.captured_env);
        }
    }

    /// Repositions the stack: within each placement, each live snackbar is offset
    /// from the anchor edge by the cumulative height of the earlier *live*
    /// snackbars at that placement (oldest nearest the edge). Mirrors the Material
    /// `reorderStack`.
    ///
    /// Dismissing snackbars are excluded from the count and keep their current
    /// offset (they fade out in place). Counting them would leave their slot
    /// occupied for the exit duration, so under rapid show/dismiss new snackbars
    /// would pile up onto ever-higher offsets instead of the survivors closing
    /// the gap.
    #[expect(
        clippy::cast_precision_loss,
        reason = "the number of simultaneously visible snackbars is exactly representable as f32"
    )]
    fn reflow(&self) {
        // The container is a fixed single-line height; the default theme value
        // matches the active (M3) theme, so it is a stable stacking increment
        // without resolving the environment theme here.
        let row = SnackbarTheme::default().single_line_min_height + SNACKBAR_STACK_GAP;
        let items = self.state.borrow().active.snapshot();
        for (index, item) in items.iter().enumerate() {
            if item.dismissing.get() {
                continue;
            }
            let earlier = items[..index]
                .iter()
                .filter(|other| {
                    !other.dismissing.get() && other.snackbar.position == item.snackbar.position
                })
                .count();
            let offset = item.snackbar.position.stack_direction() * (earlier as f32) * row;
            item.stack_offset.set(offset);
        }
    }
}

/// Window-level overlay that renders the manager's active snackbar stack.
///
/// This is a **full-window layer**, not a content-sized stack: it renders the
/// reactive snackbar set as a [`ForEach`] in an [`AbsoluteLayout`], which fills
/// the window ([`StretchAxis::Both`]) and hands every child the full window
/// bounds. That gives each snackbar a stable, window-sized frame to anchor in,
/// so bottom/top placement and `max_width` centering stay correct at any window
/// size — a plain `ZStack` is content-sized and only anchored correctly by
/// accident in a small window.
///
/// The collection engine renders the `ForEach` incrementally: each snackbar is
/// dispatched once (keyed by id) and a membership change reconciles only the
/// items that joined or left, re-compositing the window frame in isolation. The
/// layer's size is constant (full window) regardless of item count, so a change
/// never reflows the surrounding layout — which is what made rapid show/dismiss
/// and window resize flicker.
///
/// Cloneable so [`Window`] can place one instance per window above the main
/// content.
///
/// [`StretchAxis::Both`]: waterui_layout::StretchAxis
/// [`Window`]: crate::runtime::window::Window
struct SnackbarOverlay<I: 'static = NoIcon> {
    active: StateList<ActiveSnackbar<I>>,
    manager: SnackbarManager<I>,
}

impl<I: 'static> Clone for SnackbarOverlay<I> {
    fn clone(&self) -> Self {
        Self {
            active: self.active.clone(),
            manager: self.manager.clone(),
        }
    }
}

impl<I: SnackbarIconSlot> View for SnackbarOverlay<I> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        let Self { active, manager } = self;
        // Full-window absolute layer; the engine reconciles the ForEach
        // membership item-by-item. Every snackbar gets the full window bounds
        // and self-anchors to its own placement within them.
        LazyContainer::new(
            AbsoluteLayout,
            ForEach::new(active, move |item: ActiveSnackbar<I>| StackedSnackbarView {
                item,
                manager: manager.clone(),
            }),
        )
    }
}

/// One snackbar in the reactive stack overlay.
struct StackedSnackbarView<I: 'static = NoIcon> {
    item: ActiveSnackbar<I>,
    manager: SnackbarManager<I>,
}

impl<I: 'static> Clone for StackedSnackbarView<I> {
    fn clone(&self) -> Self {
        Self {
            item: self.item.clone(),
            manager: self.manager.clone(),
        }
    }
}

impl<I: SnackbarIconSlot> StackedSnackbarView<I> {
    #[allow(
        clippy::needless_pass_by_value,
        reason = "consumes the snackbar to build its view"
    )]
    fn build_content(
        snackbar: Snackbar<I>,
        manager: SnackbarManager<I>,
        id: u64,
        theme: &SnackbarTheme,
    ) -> impl View + use<I> {
        let has_trailing_controls = snackbar.action.is_some() || snackbar.closeable;
        // Message with optional icon using Label component
        let label_view =
            if let Some(icon) = snackbar.icon.and_then(SnackbarIconSlot::into_icon_view) {
                OneOf2::A(
                    label(snackbar.message)
                        .icon(icon)
                        .font(theme.supporting_text_font.clone()),
                )
            } else {
                OneOf2::B(label(snackbar.message).font(theme.supporting_text_font.clone()))
            }
            .foreground(theme.supporting_text_color.clone());

        // Optional trailing action: runs the handler with the live environment,
        // then dismisses this specific snackbar.
        let action_button = snackbar.action.map(|action| {
            let captured_env = snackbar.captured_env.clone();
            let manager = manager.clone();
            let action_label = action.label.clone();
            let semantic_text = action_label.semantic_text().clone();
            let action_font = theme.action_label_font.clone();
            let action_color = theme.action_label_color.clone();
            button(Label::new(semantic_text, move || {
                action_label
                    .clone()
                    .font(action_font.clone())
                    .foreground(action_color.clone())
            }))
            .style(ButtonStyle::Borderless)
            .action(move |env: waterui_core::Environment| {
                action.handler.run(&captured_env.layered_on(&env));
                manager.dismiss_presentation(id, SnackbarDismissReason::Action);
            })
        });

        // Optional trailing close button (M3 `closeable`): dismisses on tap.
        // The icon is drawn at `close_icon_size`, centered in the larger
        // `close_state_layer_size` box that carries the hit area.
        let close_button = snackbar.closeable.then(|| {
            let manager = manager.clone();
            Button::new(
                label("Close")
                    .icon(CloseIcon {
                        color: theme.supporting_text_color.clone(),
                        icon_size: theme.close_icon_size,
                        box_size: theme.close_state_layer_size,
                    })
                    .icon_only(),
            )
            .style(ButtonStyle::Borderless)
            .action(move |_env: waterui_core::Environment| {
                manager.dismiss_presentation(id, SnackbarDismissReason::Closed);
            })
        });

        // With trailing controls the text leads and the controls follow the
        // spacer. Without them the row is just the text, so the banner can hug
        // its content — the Apple confirmation banner is content-sized, and
        // the Material bar keeps its width through `min_width` instead of a
        // spacer. `Option<View>` renders nothing when absent.
        if !has_trailing_controls {
            return OneOf2::A(hstack((label_view,)).spacing(theme.content_spacing));
        }
        OneOf2::B(
            hstack((label_view, spacer(), action_button, close_button))
                .spacing(theme.content_spacing),
        )
    }
}

/// Sizes the bar to its content, floored and capped by the theme's width bounds.
///
/// A `Frame` with a `max_width` answers a concrete proposal with the proposal
/// itself clamped — `SwiftUI`'s expansion semantics — so a bar composed that
/// way always stretched to its cap, and a closeable snackbar spanned the whole
/// cap no matter how short its message was. The Material and Apple banners
/// hug instead: the child row is measured under a proposal capped at `max`, so
/// text still wraps there, and the bar's own width is the measured content
/// clamped into `[min, max]`. During placement the row is laid out at the
/// bar's full width, so the spacer between the message and its trailing
/// controls still pins those controls to the trailing edge whenever the `min`
/// floor leaves slack.
#[derive(Debug, Clone, Copy)]
struct HugWidth {
    min: f32,
    max: f32,
}

impl Layout for HugWidth {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        // The row is measured *unconstrained* on purpose: it contains a spacer,
        // and a stack with a spacer answers any concrete width proposal with
        // that proposal — which is exactly the expansion this layout exists to
        // stop. The unconstrained answer is the content's own size; the bar is
        // a fixed-height single line, so nothing needs the cap to wrap at.
        let child = children.first().map_or_else(Size::zero, |child| {
            child.measure(ProposalSize::new(None, proposal.height)).size
        });
        Size::new(child.width.clamp(self.min, self.max), child.height)
    }

    /// Anchored top-leading: `place` hands the row `bounds` from its origin.
    fn resolve_extent(
        &self,
        input: PlacementInput,
        _children: &[&dyn SubView],
    ) -> Result<Rect, LayoutError> {
        Ok(input.bounds.align(input.extent()?, UnitPoint::TOP_LEADING))
    }

    fn place(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
        children: &[&dyn SubView],
    ) -> Vec<SubviewPlacement> {
        // The row fills the resolved bar width; height keeps the incoming proposal.
        let child_proposal = ProposalSize::new(Some(bounds.width()), proposal.height);
        children
            .iter()
            .map(|_| SubviewPlacement::new(bounds, child_proposal))
            .collect()
    }

    fn stretch_axis(&self, children: &[StretchAxis]) -> StretchAxis {
        // Never horizontally greedy — hugging is the point. Vertical follows
        // the content, which here is a fixed-height row.
        match children.first() {
            Some(StretchAxis::Vertical | StretchAxis::Both) => StretchAxis::Vertical,
            _ => StretchAxis::None,
        }
    }
}

impl<I: SnackbarIconSlot> View for StackedSnackbarView<I> {
    fn body(self, env: &waterui_core::Environment) -> impl View {
        let theme = env.get::<SnackbarTheme>().cloned().unwrap_or_default();
        let Self { item, manager } = self;
        let position = item.snackbar.position;

        // M3 trailing inset shrinks to clear a trailing button's own padding.
        let content_padding = if item.snackbar.action.is_some() || item.snackbar.closeable {
            EdgeInsets::new(
                theme.content_padding.top(),
                theme.content_padding.leading(),
                theme.content_padding.bottom(),
                theme.action_trailing_padding,
            )
        } else {
            theme.content_padding.clone()
        };

        let content = Self::build_content(item.snackbar.clone(), manager, item.id, &theme);

        let enter_animation = theme.enter_animation.clone();
        let shape = RoundedRectangle::new(theme.corner_radius);
        let shadow = Shadow::splat(theme.shadow_radius)
            .color(theme.shadow_color.clone())
            .offset(0.0, theme.shadow_offset_y)
            .silhouette(shape);
        let ambient_shadow = Shadow::splat(theme.ambient_shadow_radius)
            .color(theme.ambient_shadow_color.clone())
            .offset(0.0, theme.ambient_shadow_offset_y)
            .silhouette(shape);

        // The appear hook runs after this subtree's first flush, once the animated
        // opacity/offset handles have bound their hidden initial values, so the
        // entrance transitions from hidden to shown. The collection engine
        // dispatches each snackbar exactly once (keyed by id) and never
        // re-dispatches a surviving one, so this hook fires once naturally: no
        // one-shot latch is needed.
        let entrance_opacity = item.opacity.clone();
        let entrance_offset = item.entrance_offset.clone();

        Frame::new(
            FixedContainer::new(
                HugWidth {
                    min: theme.min_width,
                    max: theme.max_width,
                },
                (content
                    .on_appear(move || {
                        entrance_opacity.set(1.0);
                        entrance_offset.set(0.0);
                    })
                    .padding_with(content_padding)
                    .height(theme.single_line_min_height),),
            )
            .background(shape.fill(theme.container_color.clone()))
            .shadow(ambient_shadow)
            .shadow(shadow)
            .opacity(item.opacity.with(enter_animation.clone()))
            // Entrance slide and reflow shift are independent animated
            // bindings (each has a stable identity, so each animates).
            .offset(0.0, item.entrance_offset.with(enter_animation.clone()))
            .offset(0.0, item.stack_offset.with(enter_animation)),
        )
        .alignment(position.to_alignment())
        // The backend places this host clear of the hardware; the theme's own
        // margin keeps the bar off the window edge.
        .padding_with(theme.viewport_padding)
    }
}

/// Hosts a [`SnackbarManager`] and its overlay for a view subtree.
///
/// Attach with [`ViewExt::snackbar_host`] (plain snackbars) or
/// [`ViewExt::snackbar_host_with_icons`] (snackbars carrying a typed icon view).
/// Each built instance creates its own manager and overlay stack, so multiple
/// windows or scenes stay isolated. When `flow-markdown` is enabled, this host
/// also installs `CodeCopyFeedback` so copying a markdown code block presents
/// `"Copied to clipboard"` through the hosted manager.
#[derive(Debug)]
pub struct SnackbarHost<V, I = NoIcon> {
    content: V,
    _icon: PhantomData<fn() -> I>,
}

impl<V, I> SnackbarHost<V, I> {
    /// Wraps `content` in a snackbar host.
    #[must_use]
    pub const fn new(content: V) -> Self {
        Self {
            content,
            _icon: PhantomData,
        }
    }
}

impl<V: View, I: SnackbarIconSlot> View for SnackbarHost<V, I> {
    fn body(self, _env: &Environment) -> impl View {
        let (manager, overlay) = SnackbarManager::<I>::typed();
        let content = self.content.overlay(overlay).state(&manager);
        #[cfg(feature = "flow-markdown")]
        let content = content.with(CodeCopyFeedback(OnCopied::new({
            let snackbar = manager;
            move |_env| {
                snackbar.show(I::from_plain(Snackbar::new("Copied to clipboard")));
            }
        })));
        content
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use nami::Signal as _;
    use waterui_core::binding;

    use super::super::parked_executor::install as install_executor;
    use super::*;

    /// A presentation row built directly, so tests drive `finish_dismissal`
    /// without a show timer or a running executor.
    fn presentation(id: u64, snackbar: Snackbar) -> ActiveSnackbar {
        ActiveSnackbar {
            id,
            opacity: Binding::new(0.0_f32),
            entrance_offset: Binding::<f32>::new(snackbar.position.initial_offset_y()),
            stack_offset: Binding::new(0.0_f32),
            dismissing: Rc::new(Cell::new(false)),
            snackbar,
        }
    }

    fn present(manager: &SnackbarManager, item: ActiveSnackbar) {
        manager.state.borrow().active.clone().push(item);
    }

    fn live_ids(manager: &SnackbarManager) -> Vec<u64> {
        manager
            .state
            .borrow()
            .active
            .snapshot()
            .iter()
            .map(|item| item.id)
            .collect()
    }

    /// A snackbar recording every reported reason.
    fn recording(snackbar: Snackbar) -> (Snackbar, Rc<RefCell<Vec<SnackbarDismissReason>>>) {
        let reasons = Rc::new(RefCell::new(Vec::new()));
        let log = Rc::clone(&reasons);
        (
            snackbar.on_dismiss(move |reason: SnackbarDismissReason| {
                log.borrow_mut().push(reason);
            }),
            reasons,
        )
    }

    #[test]
    fn building_or_showing_a_snackbar_reports_nothing() {
        let (manager, _overlay) = SnackbarManager::new();
        let (snackbar, reasons) = recording(Snackbar::new("hello"));
        present(&manager, presentation(1, snackbar.clone()));
        present(&manager, presentation(2, snackbar));
        assert!(reasons.borrow().is_empty());

        // A finish for an id that was never presented reports nothing either.
        manager.finish_dismissal(42, SnackbarDismissReason::Programmatic);
        assert!(reasons.borrow().is_empty());
    }

    #[test]
    fn removal_reports_the_reason_once() {
        let (manager, _overlay) = SnackbarManager::new();
        let (snackbar, reasons) = recording(Snackbar::new("bye"));
        present(&manager, presentation(1, snackbar));

        manager.finish_dismissal(1, SnackbarDismissReason::Timeout);
        assert_eq!(live_ids(&manager), Vec::<u64>::new());
        assert_eq!(*reasons.borrow(), [SnackbarDismissReason::Timeout]);

        // A stale finish for the same presentation reports nothing more.
        manager.finish_dismissal(1, SnackbarDismissReason::Programmatic);
        assert_eq!(*reasons.borrow(), [SnackbarDismissReason::Timeout]);
    }

    #[test]
    fn removal_without_a_callback_is_harmless() {
        let (manager, _overlay) = SnackbarManager::new();
        present(&manager, presentation(1, Snackbar::new("plain")));
        manager.finish_dismissal(1, SnackbarDismissReason::Timeout);
        assert_eq!(live_ids(&manager), Vec::<u64>::new());
    }

    #[test]
    fn the_callback_observes_removal_and_may_reenter_the_manager() {
        let (manager, _overlay) = SnackbarManager::new();
        let seen = Rc::new(Cell::new(false));
        let snackbar = Snackbar::new("going").on_dismiss({
            let manager = manager.clone();
            let seen = Rc::clone(&seen);
            move |_reason: SnackbarDismissReason| {
                // The presentation is already off the stack, and reentering
                // the manager does not panic on a held borrow.
                assert!(manager.state.borrow().active.snapshot().is_empty());
                manager.dismiss();
                seen.set(true);
            }
        });
        present(&manager, presentation(1, snackbar));

        manager.finish_dismissal(1, SnackbarDismissReason::Programmatic);
        assert!(seen.get());
    }

    #[test]
    fn each_presentation_of_a_cloned_snackbar_reports_its_own_reason() {
        let (manager, _overlay) = SnackbarManager::new();
        let (snackbar, reasons) = recording(Snackbar::new("twice"));
        present(&manager, presentation(1, snackbar.clone()));
        present(&manager, presentation(2, snackbar));

        manager.finish_dismissal(1, SnackbarDismissReason::Replaced);
        manager.finish_dismissal(2, SnackbarDismissReason::Closed);
        assert_eq!(
            *reasons.borrow(),
            [
                SnackbarDismissReason::Replaced,
                SnackbarDismissReason::Closed
            ]
        );
    }

    #[test]
    fn the_first_dismissal_reason_wins() {
        install_executor();
        let (manager, _overlay) = SnackbarManager::new();
        let (snackbar, reasons) = recording(Snackbar::new("raced"));
        present(&manager, presentation(1, snackbar));

        // A timeout already started the fade; the later programmatic dismiss
        // must not overwrite the reason the exit timer carries.
        manager.dismiss_presentation(1, SnackbarDismissReason::Timeout);
        manager.dismiss_presentation(1, SnackbarDismissReason::Programmatic);
        manager.finish_dismissal(1, SnackbarDismissReason::Timeout);
        assert_eq!(*reasons.borrow(), [SnackbarDismissReason::Timeout]);
    }

    #[test]
    fn the_handler_extracts_injected_state() {
        let (manager, _overlay) = SnackbarManager::new();
        let count = binding::<i32>(0);
        let snackbar = Snackbar::new("stateful").state(&count).on_dismiss(
            |reason: SnackbarDismissReason, State(count): State<Binding<i32>>| {
                assert_eq!(reason, SnackbarDismissReason::Action);
                count.set(count.snapshot() + 1);
            },
        );
        present(&manager, presentation(1, snackbar));

        manager.finish_dismissal(1, SnackbarDismissReason::Action);
        assert_eq!(count.snapshot(), 1);
    }

    #[test]
    fn typed_icon_host_accepts_plain_and_icon_snackbars() {
        install_executor();
        let view = SnackbarManager::host(|manager| {
            manager.show(Snackbar::new("Plain"));
            manager.show(Snackbar::new("Icon").icon(|| "★"));
            "Content"
        });
        let _ = view.body(&Environment::new());
    }

    #[test]
    fn snackbar_icon_accepts_non_clone_modified_view_builder() {
        use waterui_layout::padding::Padding;
        use waterui_text::{Text, text};

        install_executor();
        let snackbar = Snackbar::new("Padded").icon(|| text("★").padding());
        let cloned = snackbar.clone();
        let host = text("Content").snackbar_host_with_icons::<Padding<Text>>();
        let _ = host.body(&Environment::new());

        let view = SnackbarManager::host(move |manager| {
            manager.show(snackbar);
            manager.show(cloned);
            "Content"
        });
        let _ = view.body(&Environment::new());
    }

    #[test]
    fn snackbar_message_and_action_resolve_through_translation_catalog() {
        use waterui_locale::{TranslationCatalog, locales};

        let mut env = Environment::new();
        env.insert(locales::FR);
        env.insert(
            TranslationCatalog::new()
                .add_toml(
                    locales::FR,
                    "\"File saved\" = \"Fichier enregistré\"\n\"Undo\" = \"Annuler\"",
                )
                .expect("test catalog must parse"),
        );

        let snackbar = Snackbar::new("File saved").action("Undo", || {});
        assert_eq!(
            snackbar
                .message
                .resolve(&env)
                .content()
                .snapshot()
                .to_plain()
                .as_str(),
            "Fichier enregistré"
        );
        let action = snackbar.action.as_ref().expect("action must be present");
        assert_eq!(
            action
                .label()
                .semantic_text()
                .resolve(&env)
                .content()
                .snapshot()
                .to_plain()
                .as_str(),
            "Annuler"
        );
    }
}
