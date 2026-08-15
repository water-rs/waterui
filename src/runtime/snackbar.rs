//! Snackbar notification system for `WaterUI`.
//!
//! Snackbars provide brief messages about app processes at the bottom of the screen.
//! They automatically disappear after a timeout and can optionally include an action button.
//!
//! # Examples
//!
//! ```ignore
//! use waterui::prelude::*;
//! use waterui::snackbar::{Snackbar, SnackbarManager, SnackbarPosition};
//!
//! // Get the manager from environment (automatically installed in Window)
//! let manager = env.get::<SnackbarManager>().unwrap();
//!
//! // Show a simple message
//! manager.show(Snackbar::new("File saved"));
//!
//! // With icon and action (use a packaged icon crate for portable apps)
//! manager.show(
//!     Snackbar::new("Item deleted")
//!         .icon(mdi::delete())
//!         .action("Undo", || {})
//!         .duration(Duration::from_secs(5))
//! );
//!
//! // Custom position
//! manager.show(
//!     Snackbar::new("Connection restored")
//!         .position(SnackbarPosition::TopCenter)
//! );
//! ```

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use core::time::Duration;

use executor_core::spawn_local;
use nami::Binding;
use nami::collection::List;
use waterui_controls::{Button, ButtonStyle, button, label};
use waterui_core::animation::Animation;
use waterui_core::extract::State;
use waterui_core::handler::{AnyViewBuilder, Handler, SharedAction, shared_action};
use waterui_core::id::Identifiable;
use waterui_core::plugin::Plugin;
use waterui_core::views::ForEach;
use waterui_core::{AnimationExt, View};
use waterui_layout::AbsoluteLayout;
use waterui_layout::container::LazyContainer;
use waterui_layout::frame::Frame;
use waterui_layout::padding::EdgeInsets;
use waterui_layout::spacer::spacer;
use waterui_layout::stack::{Alignment, hstack};
use waterui_str::Str;
use waterui_text::{font::Font, text::text};

use crate::AnyView;
use crate::ViewExt;
use crate::shape::{FilledShape, Path, RoundedRectangle, ShapeExt};
use crate::style::{Shadow, Vector};
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
    /// Outer viewport padding.
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
    /// Container corner radius in logical units for shadows.
    pub corner_radius: f32,
    /// Normalized corner radius for clipping and filled shape rendering.
    pub clip_radius: f32,
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

impl Default for SnackbarTheme {
    fn default() -> Self {
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
            clip_radius: 0.08,
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
}

/// Position for snackbar display.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
    pub label: Str,
    /// The callback when action is pressed.
    handler: SharedAction<()>,
}

impl core::fmt::Debug for SnackbarAction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SnackbarAction")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// Configuration for a snackbar notification.
///
/// Use the builder pattern to configure the snackbar before showing it.
///
/// # Examples
///
/// ```ignore
/// Snackbar::new("Message sent")
///     .icon(SystemIcon::CHECKMARK)
///     .duration(Duration::from_secs(3))
/// ```
#[derive(Clone)]
pub struct Snackbar {
    /// The message to display.
    message: Str,
    /// Optional icon to display before the message.
    icon: Option<SnackbarIcon>,
    /// Optional action button.
    action: Option<SnackbarAction>,
    /// Display duration before auto-dismiss.
    duration: Duration,
    /// Position on screen.
    position: SnackbarPosition,
    /// Whether to show a trailing close button (M3 `closeable`).
    closeable: bool,
    /// State injected into the action button environment.
    captured_env: waterui_core::Environment,
}

impl core::fmt::Debug for Snackbar {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Snackbar")
            .field("message", &self.message)
            .field("icon", &self.icon)
            .field("action", &self.action)
            .field("duration", &self.duration)
            .field("position", &self.position)
            .field("closeable", &self.closeable)
            .field("captured_env", &self.captured_env)
            .finish()
    }
}

/// Cloneable leading-icon slot built lazily per presentation.
#[derive(Clone)]
struct SnackbarIcon(AnyViewBuilder<AnyView>);

impl core::fmt::Debug for SnackbarIcon {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SnackbarIcon").finish_non_exhaustive()
    }
}

impl View for SnackbarIcon {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        self.0.build()
    }
}

impl Snackbar {
    /// Creates a new snackbar with the specified message.
    #[must_use]
    pub fn new(message: impl Into<Str>) -> Self {
        Self {
            message: message.into(),
            icon: None,
            action: None,
            duration: Duration::from_secs(3),
            position: SnackbarPosition::default(),
            closeable: false,
            captured_env: waterui_core::Environment::new(),
        }
    }

    /// Adds a leading icon to the snackbar.
    ///
    /// Any view works; for portable apps prefer a packaged icon crate such as
    /// `waterui-icons-material-icon` (`SystemIcon` is only available on Apple
    /// backends).
    #[must_use]
    pub fn icon(mut self, icon: impl View + Clone + 'static) -> Self {
        self.icon = Some(SnackbarIcon(AnyViewBuilder::new(move || {
            AnyView::new(icon.clone())
        })));
        self
    }

    /// Adds an action button to the snackbar.
    ///
    /// # Examples
    ///
    /// Simple action:
    /// ```rust,ignore
    /// Snackbar::new("Message sent")
    ///     .action("Undo").handler(|| {})
    /// ```
    ///
    /// With injected state:
    /// ```rust,ignore
    /// Snackbar::new("Item deleted")
    ///     .action("Undo", |State(items): State<Items>| items.restore())
    ///     .state(&items)
    /// ```
    #[must_use]
    pub fn action<Args>(
        mut self,
        label: impl Into<Str>,
        handler: impl Handler<Args, ()> + 'static,
    ) -> Self {
        self.action = Some(SnackbarAction {
            label: label.into(),
            handler: shared_action(handler),
        });
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

    /// Injects cloneable state into this snackbar's action environment.
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
/// stack as a [`ForEach`] over the manager's [`List`], so the hydrolysis
/// collection engine dispatches each presentation's subtree exactly once (keyed
/// by [`Self::id`]) and reconciles only the items that join or leave. A
/// presentation that stays on screen is never re-dispatched, so its `on_appear`
/// entrance fires once and its in-flight animations continue uninterrupted —
/// without any one-shot latch.
///
/// Cloning an `ActiveSnackbar` shares those bindings by reference count, so the
/// snapshots the manager reflows from all refer to the same animated state.
#[derive(Clone, crate::Identifiable)]
struct ActiveSnackbar {
    #[id]
    id: u64,
    snackbar: Snackbar,
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

/// Internal state for the snackbar manager.
struct SnackbarManagerState {
    /// The active on-screen snackbars, oldest first. A reactive [`List`] so the
    /// overlay's [`ForEach`] reconciles the stack item-by-item as snackbars are
    /// added and removed; per-row animation bindings live on each
    /// [`ActiveSnackbar`] and are untouched by membership changes.
    active: List<ActiveSnackbar>,
    /// Monotonic presentation identifier used to ignore stale timers.
    next_id: u64,
}

/// Manages snackbar notifications in a FIFO queue.
///
/// This plugin is automatically installed in every `Window`. Access it from the environment
/// to show snackbar notifications.
///
/// # Examples
///
/// ```ignore
/// use waterui::prelude::*;
/// use waterui::snackbar::{Snackbar, SnackbarManager};
///
/// fn my_view(env: &Environment) -> impl View {
///     let manager = env.get::<SnackbarManager>().unwrap();
///
///     button("Save").action(move || {
///         manager.show(Snackbar::new("File saved successfully"));
///     })
/// }
/// ```
#[derive(Clone)]
pub struct SnackbarManager {
    state: Rc<RefCell<SnackbarManagerState>>,
}

impl Plugin for SnackbarManager {}

impl core::fmt::Debug for SnackbarManager {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SnackbarManager").finish()
    }
}

impl SnackbarManager {
    /// Creates a new snackbar manager and returns the overlay view.
    ///
    /// The returned view should be placed in a window overlay above the main content.
    /// This is automatically done by `Window::new()`.
    ///
    /// # Returns
    ///
    /// A tuple containing the manager and the overlay view.
    pub fn new() -> (Self, impl View + Clone) {
        let active = List::new();
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

    /// Shows a snackbar. Snackbars stack on screen simultaneously, oldest nearest
    /// the anchor edge. Each placement is an independent stack: a snackbar only
    /// interacts with others sharing its [`SnackbarPosition`], and when a
    /// placement already holds [`MAX_VISIBLE_SNACKBARS`] the oldest *at that
    /// placement* is dismissed to make room — different placements never evict
    /// each other.
    ///
    /// # Panics
    ///
    /// Panics if the snackbar presentation id counter overflows.
    pub fn show(&self, snackbar: Snackbar) {
        let duration = snackbar.duration;
        let position = snackbar.position;
        let id = {
            let mut state = self.state.borrow_mut();
            state.next_id = state
                .next_id
                .checked_add(1)
                .expect("Snackbar presentation id overflow");
            state.next_id
        };
        let item = ActiveSnackbar {
            id,
            opacity: Binding::f32(0.0),
            entrance_offset: Binding::f32(snackbar.position.initial_offset_y()),
            stack_offset: Binding::f32(0.0),
            dismissing: Rc::new(Cell::new(false)),
            snackbar,
        };

        let active = self.state.borrow().active.clone();

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
                self.dismiss_presentation(oldest);
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
            manager.dismiss_presentation(id);
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
            self.dismiss_presentation(id);
        }
    }

    /// Begins the dismissal fade for one snackbar, then removes it from the
    /// stack and reflows the remainder once the fade completes.
    fn dismiss_presentation(&self, id: u64) {
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
            manager.finish_dismissal(id);
        })
        .detach();
    }

    fn finish_dismissal(&self, id: u64) {
        let active = self.state.borrow().active.clone();
        let index = active.snapshot().iter().position(|item| item.id == id);
        if let Some(index) = index {
            let _removed = active.remove(index);
            self.reflow();
        }
    }

    /// Repositions the stack: within each placement, each live snackbar is offset
    /// from the anchor edge by the cumulative height of the earlier *live*
    /// snackbars at that placement (oldest nearest the edge). Mirrors mdui's
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
#[derive(Clone)]
struct SnackbarOverlay {
    active: List<ActiveSnackbar>,
    manager: SnackbarManager,
}

impl View for SnackbarOverlay {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        let Self { active, manager } = self;
        // Full-window absolute layer; the engine reconciles the ForEach
        // membership item-by-item. Every snackbar gets the full window bounds
        // and self-anchors to its own placement within them.
        LazyContainer::new(
            AbsoluteLayout,
            ForEach::new(active, move |item: ActiveSnackbar| StackedSnackbarView {
                item,
                manager: manager.clone(),
            }),
        )
    }
}

/// One snackbar in the reactive stack overlay.
#[derive(Clone)]
struct StackedSnackbarView {
    item: ActiveSnackbar,
    manager: SnackbarManager,
}

impl StackedSnackbarView {
    #[allow(
        clippy::needless_pass_by_value,
        reason = "consumes the snackbar to build its view"
    )]
    fn build_content(
        snackbar: Snackbar,
        manager: SnackbarManager,
        id: u64,
        theme: &SnackbarTheme,
    ) -> AnyView {
        // Message with optional icon using Label component
        let label_view = if let Some(icon) = snackbar.icon {
            label(snackbar.message)
                .icon(icon)
                .font(theme.supporting_text_font.clone())
        } else {
            label(snackbar.message).font(theme.supporting_text_font.clone())
        }
        .foreground(theme.supporting_text_color.clone());

        // Optional trailing action: runs the handler with the live environment,
        // then dismisses this specific snackbar.
        let action_button = snackbar.action.map(|action| {
            let captured_env = snackbar.captured_env.clone();
            let manager = manager.clone();
            button(
                text(action.label.clone())
                    .font(theme.action_label_font.clone())
                    .color(theme.action_label_color.clone()),
            )
            .style(ButtonStyle::Borderless)
            .action(move |env: waterui_core::Environment| {
                let () = action.handler.call(&captured_env.layered_on(&env));
                manager.dismiss_presentation(id);
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
                manager.dismiss_presentation(id);
            })
        });

        // Supporting text is leading-aligned; trailing controls (if any) follow
        // the spacer. `Option<View>` renders nothing when absent.
        AnyView::new(
            hstack((label_view, spacer(), action_button, close_button))
                .spacing(theme.content_spacing),
        )
    }
}

impl View for StackedSnackbarView {
    fn body(self, env: &waterui_core::Environment) -> impl View {
        let theme = env.get::<SnackbarTheme>().cloned().unwrap_or_default();
        let Self { item, manager } = self;
        let position = item.snackbar.position;

        // M3 trailing inset shrinks to clear a trailing button's own padding.
        let content_padding = if item.snackbar.action.is_some() || item.snackbar.closeable {
            EdgeInsets::new(
                theme.content_padding.top(),
                theme.content_padding.bottom(),
                theme.content_padding.leading(),
                theme.action_trailing_padding,
            )
        } else {
            theme.content_padding.clone()
        };

        let content = Self::build_content(item.snackbar.clone(), manager, item.id, &theme);

        let enter_animation = theme.enter_animation.clone();
        let shadow = Shadow::new(
            theme.shadow_color.clone(),
            Vector::new(0.0, theme.shadow_offset_y),
            theme.shadow_radius,
        );
        let ambient_shadow = Shadow::new(
            theme.ambient_shadow_color.clone(),
            Vector::new(0.0, theme.ambient_shadow_offset_y),
            theme.ambient_shadow_radius,
        );

        // The appear hook runs after this subtree's first flush, once the animated
        // opacity/offset handles have bound their hidden initial values, so the
        // entrance transitions from hidden to shown. The collection engine
        // dispatches each snackbar exactly once (keyed by id) and never
        // re-dispatches a surviving one, so this hook fires once naturally: no
        // one-shot latch is needed.
        let entrance_opacity = item.opacity.clone();
        let entrance_offset = item.entrance_offset.clone();

        Frame::new(
            content
                .on_appear(move || {
                    entrance_opacity.set(1.0);
                    entrance_offset.set(0.0);
                })
                .padding_with(content_padding)
                .height(theme.single_line_min_height)
                .min_width(theme.min_width)
                .max_width(theme.max_width)
                .background(
                    RoundedRectangle::new(theme.clip_radius).fill(theme.container_color.clone()),
                )
                .shadow(ambient_shadow)
                .shadow(shadow)
                .opacity(item.opacity.with_animation(enter_animation.clone()))
                // Entrance slide and reflow shift are independent animated
                // bindings (each has a stable identity, so each animates).
                .offset(
                    0.0,
                    item.entrance_offset.with_animation(enter_animation.clone()),
                )
                .offset(0.0, item.stack_offset.with_animation(enter_animation)),
        )
        .alignment(position.to_alignment())
        .padding_with(theme.viewport_padding) // Safe area inset
    }
}

impl Default for SnackbarManager {
    fn default() -> Self {
        Self::new().0
    }
}
