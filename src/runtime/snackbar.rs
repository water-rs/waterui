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
//!         .action("Undo", || println!("Undo clicked"))
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
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::time::Duration;

use executor_core::spawn_local;
use nami::Binding;
use waterui_controls::{ButtonStyle, button};
use waterui_core::animation::Animation;
use waterui_core::dynamic::watch;
use waterui_core::extract::State;
use waterui_core::handler::{AnyViewBuilder, Handler, SharedAction, shared_action};
use waterui_core::plugin::Plugin;
use waterui_core::{AnimationExt, View};
use waterui_layout::frame::Frame;
use waterui_layout::padding::EdgeInsets;
use waterui_layout::spacer::spacer;
use waterui_layout::stack::{Alignment, ZStack, hstack};
use waterui_str::Str;
use waterui_text::{font::Font, text::text};

use crate::AnyView;
use crate::ViewExt;
use crate::component::Label;
use crate::shape::{RoundedRectangle, ShapeExt};
use crate::style::{Shadow, Vector};
use waterui_graphics::color::Color;

/// Maximum number of snackbars visible at once *per placement*. M3 keeps each
/// on-screen stack small; when a new one arrives over this limit at the same
/// placement, the oldest at that placement is dismissed.
const MAX_VISIBLE_SNACKBARS: usize = 3;

/// Vertical gap between stacked snackbars, in logical units.
const SNACKBAR_STACK_GAP: f32 = 8.0;

/// Glyph for the optional close button (Material `closeable`). The multiplication
/// sign (U+00D7) reads as a close "×" and, unlike the Dingbats heavy cross
/// (U+2715), is present in essentially every system font, so it never renders as
/// a missing-glyph box.
const SNACKBAR_CLOSE_GLYPH: &str = "\u{00d7}";

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
            single_line_min_height: 48.0,
            corner_radius: 4.0,
            clip_radius: 0.08,
            shadow_color: Color::srgb(0, 0, 0).with_opacity(0.2),
            shadow_radius: 3.0,
            shadow_offset_y: 3.0,
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
    ///     .action("Undo").handler(|| println!("Undo!"))
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
/// [`Self::entrance_offset`], [`Self::stack_offset`]). Those bindings live in
/// the manager, not inside the overlay subtree, so when the overlay's `watch`
/// rebuilds the stack on a membership change every presentation re-binds to the
/// same handles and its in-flight animation continues uninterrupted (animation
/// state is keyed by signal identity, not view-tree position).
///
/// Cloning an `ActiveSnackbar` shares those bindings by reference count, so the
/// `Vec` snapshots the overlay rebuilds from all refer to the same animated
/// state.
#[derive(Clone)]
struct ActiveSnackbar {
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
    /// Latches once the entrance animation has run. The overlay rebuilds the
    /// whole stack whenever membership changes, which re-fires every row's
    /// `on_appear`; this flag keeps the entrance a one-shot so a rebuild cannot
    /// restart a settled row's fade-in or reverse a row that is dismissing.
    has_entered: Rc<Cell<bool>>,
    /// Whether this presentation is running its dismissal fade.
    dismissing: Rc<Cell<bool>>,
}

/// Internal state for the snackbar manager.
struct SnackbarManagerState {
    /// The active on-screen snackbars, oldest first. A single reactive binding
    /// so the overlay's `watch` rebuilds the stack as snackbars are added and
    /// removed; per-row animation bindings live on each [`ActiveSnackbar`] and
    /// survive those rebuilds.
    active: Binding<Vec<ActiveSnackbar>>,
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
    /// The returned view should be placed in a `ZStack` above the main content.
    /// This is automatically done by `Window::new()`.
    ///
    /// # Returns
    ///
    /// A tuple containing the manager and the overlay view.
    pub fn new() -> (Self, impl View + Clone) {
        let active = Binding::container(Vec::new());
        let state = Rc::new(RefCell::new(SnackbarManagerState {
            active: active.clone(),
            next_id: 0,
        }));
        let manager = Self { state };

        // The overlay watches the active stack and rebuilds it on every
        // membership change. Each snackbar is a full-bleed Frame self-anchored
        // to its own placement, so mixed placements coexist; per-row animation
        // bindings persist across rebuilds (see [`ActiveSnackbar`]).
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
            has_entered: Rc::new(Cell::new(false)),
            dismissing: Rc::new(Cell::new(false)),
            snackbar,
        };

        let active = self.state.borrow().active.clone();

        // Evict the oldest live snackbar AT THIS PLACEMENT if its stack is
        // already full, so the new one slides in as the oldest fades out.
        // Snackbars at other placements are untouched.
        let snapshot = active.get();
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

        active.with_mut(|stack| stack.push(item));
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
            .get()
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
            .get()
            .into_iter()
            .find(|item| item.id == id);
        let Some(item) = item else {
            return;
        };
        if item.dismissing.replace(true) {
            return;
        }

        item.opacity.set(0.0);
        item.entrance_offset.set(item.snackbar.position.initial_offset_y());

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
        let index = active.get().iter().position(|item| item.id == id);
        if let Some(index) = index {
            active.with_mut(|stack| {
                stack.remove(index);
            });
            self.reflow();
        }
    }

    /// Repositions the stack: within each placement, snackbars are offset from
    /// the anchor edge by the cumulative height of the earlier snackbars at
    /// that placement (oldest nearest the edge). Mirrors mdui's `reorderStack`.
    fn reflow(&self) {
        // The container is a fixed single-line height; the default theme value
        // matches the active (M3) theme, so it is a stable stacking increment
        // without resolving the environment theme here.
        let row = SnackbarTheme::default().single_line_min_height + SNACKBAR_STACK_GAP;
        let items = self.state.borrow().active.get();
        for (index, item) in items.iter().enumerate() {
            let earlier = items[..index]
                .iter()
                .filter(|other| other.snackbar.position == item.snackbar.position)
                .count();
            let offset = item.snackbar.position.stack_direction() * (earlier as f32) * row;
            item.stack_offset.set(offset);
        }
    }
}

/// Window-level overlay that renders the manager's active snackbar stack.
///
/// `watch`es the active binding and rebuilds a [`ZStack`] of full-bleed,
/// self-anchored rows whenever membership changes. Cloneable so [`Window`] can
/// place one instance per window above the main content.
///
/// [`Window`]: crate::runtime::window::Window
#[derive(Clone)]
struct SnackbarOverlay {
    active: Binding<Vec<ActiveSnackbar>>,
    manager: SnackbarManager,
}

impl View for SnackbarOverlay {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        let Self { active, manager } = self;
        watch(active, move |items: Vec<ActiveSnackbar>| {
            let manager = manager.clone();
            items
                .into_iter()
                .map(|item| StackedSnackbarView {
                    item,
                    manager: manager.clone(),
                })
                .collect::<ZStack<(Vec<AnyView>,)>>()
        })
    }
}

/// One snackbar in the reactive stack overlay.
#[derive(Clone)]
struct StackedSnackbarView {
    item: ActiveSnackbar,
    manager: SnackbarManager,
}

impl StackedSnackbarView {
    fn build_content(
        snackbar: Snackbar,
        manager: SnackbarManager,
        id: u64,
        theme: &SnackbarTheme,
    ) -> AnyView {
        // Message with optional icon using Label component
        let label_view = if let Some(icon) = snackbar.icon {
            Label::new(snackbar.message)
                .icon(icon)
                .font(theme.supporting_text_font.clone())
        } else {
            Label::new(snackbar.message).font(theme.supporting_text_font.clone())
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
        let close_button = snackbar.closeable.then(|| {
            let manager = manager.clone();
            button(
                text(SNACKBAR_CLOSE_GLYPH)
                    .font(theme.supporting_text_font.clone())
                    .color(theme.supporting_text_color.clone()),
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

        // The appear hook runs while this subtree is dispatched — after the
        // animated opacity/offset handles bind at their hidden initial values —
        // so the entrance transitions from hidden to shown. The overlay rebuilds
        // the whole stack on membership changes, which re-fires this hook for
        // every row; `has_entered` latches it to a one-shot so a rebuild cannot
        // restart a settled row's fade-in or reverse a row mid-dismissal.
        let entrance_opacity = item.opacity.clone();
        let entrance_offset = item.entrance_offset.clone();
        let has_entered = item.has_entered.clone();

        Frame::new(
            content
                .on_appear(move || {
                    if has_entered.replace(true) {
                        return;
                    }
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
                .shadow(shadow)
                .opacity(item.opacity.with_animation(enter_animation.clone()))
                // Entrance slide and reflow shift are independent animated
                // bindings (each has a stable identity, so each animates).
                .offset(0.0, item.entrance_offset.with_animation(enter_animation.clone()))
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
