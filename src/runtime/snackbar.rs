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
//! // With icon and action
//! manager.show(
//!     Snackbar::new("Item deleted")
//!         .icon(SystemIcon::TRASH)
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

use alloc::collections::VecDeque;
use alloc::rc::Rc;
use core::cell::RefCell;
use core::time::Duration;

use executor_core::spawn_local;
use nami::Binding;
use waterui_controls::{ButtonStyle, button};
use waterui_core::animation::Animation;
use waterui_core::dynamic::{Dynamic, DynamicHandler};
use waterui_core::extract::State;
use waterui_core::handler::{Handler, SharedAction, shared_action};
use waterui_core::plugin::Plugin;
use waterui_core::{AnimationExt, View};
use waterui_icon::SystemIcon;
use waterui_layout::frame::Frame;
use waterui_layout::padding::EdgeInsets;
use waterui_layout::spacer::spacer;
use waterui_layout::stack::{Alignment, hstack};
use waterui_str::Str;
use waterui_text::{font::Font, text::text};

use crate::AnyView;
use crate::ViewExt;
use crate::component::Label;
use crate::shape::{RoundedRectangle, ShapeExt};
use crate::style::{Shadow, Vector};
use waterui_graphics::color::Color;

/// Maximum number of snackbars that can be queued.
const MAX_QUEUE_SIZE: usize = 10;

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

    /// Returns the initial Y offset for entrance animation.
    const fn initial_offset_y(self) -> f32 {
        match self {
            Self::BottomCenter | Self::BottomLeading | Self::BottomTrailing => 20.0,
            Self::TopCenter => -20.0,
        }
    }
}

/// An action button for the snackbar.
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
pub struct Snackbar {
    /// The message to display.
    message: Str,
    /// Optional icon to display before the message.
    icon: Option<SystemIcon>,
    /// Optional action button.
    action: Option<SnackbarAction>,
    /// Display duration before auto-dismiss.
    duration: Duration,
    /// Position on screen.
    position: SnackbarPosition,
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
            .field("captured_env", &self.captured_env)
            .finish()
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
            captured_env: waterui_core::Environment::new(),
        }
    }

    /// Adds an icon to the snackbar.
    #[must_use]
    pub fn icon(mut self, icon: SystemIcon) -> Self {
        self.icon = Some(icon);
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

    /// Injects cloneable state into this snackbar's action environment.
    #[must_use]
    pub fn state<T: Clone + 'static>(mut self, state: &T) -> Self {
        self.captured_env = self.captured_env.extending(State(state.clone()));
        self
    }
}

/// Internal state for the snackbar manager.
struct SnackbarManagerState {
    /// Queue of pending snackbars.
    queue: VecDeque<Snackbar>,
    /// Handler to update the view.
    handler: DynamicHandler,
    /// Whether a snackbar is currently displayed.
    is_showing: bool,
    /// Whether the current snackbar is running its dismissal animation.
    is_dismissing: bool,
    /// Current presentation animation handles.
    current: Option<SnackbarPresentation>,
    /// Monotonic presentation identifier used to ignore stale timers.
    next_presentation_id: u64,
}

#[derive(Debug, Clone)]
struct SnackbarPresentation {
    id: u64,
    opacity: Binding<f32>,
    offset_y: Binding<f32>,
    hidden_offset_y: f32,
    exit_animation: Animation,
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
        let (handler, dynamic) = Dynamic::new();
        handler.set(()); // Initially empty

        let state = Rc::new(RefCell::new(SnackbarManagerState {
            queue: VecDeque::new(),
            handler,
            is_showing: false,
            is_dismissing: false,
            current: None,
            next_presentation_id: 0,
        }));

        let manager = Self { state };
        (manager, dynamic)
    }

    /// Shows a snackbar. If one is already showing, queues this one.
    ///
    /// Snackbars are displayed in FIFO order. If the queue is full (10 items),
    /// the oldest pending snackbar is dropped.
    pub fn show(&self, snackbar: Snackbar) {
        let mut state = self.state.borrow_mut();

        if state.is_showing {
            // Queue for later, dropping oldest if full
            if state.queue.len() >= MAX_QUEUE_SIZE {
                state.queue.pop_front();
            }
            state.queue.push_back(snackbar);
        } else {
            // Show immediately
            drop(state);
            self.begin_show(snackbar);
        }
    }

    /// Dismisses the current snackbar immediately.
    pub fn dismiss(&self) {
        let id = self
            .state
            .borrow()
            .current
            .as_ref()
            .map(|presentation| presentation.id);
        if let Some(id) = id {
            self.dismiss_presentation(id);
        }
    }

    fn begin_show(&self, snackbar: Snackbar) {
        let duration = snackbar.duration;
        let position = snackbar.position;
        let hidden_offset_y = position.initial_offset_y();
        let opacity = Binding::f32(0.0);
        let offset_y = Binding::container(hidden_offset_y);
        let presentation = {
            let mut state = self.state.borrow_mut();
            state.next_presentation_id = state
                .next_presentation_id
                .checked_add(1)
                .expect("Snackbar presentation id overflow");
            let presentation = SnackbarPresentation {
                id: state.next_presentation_id,
                opacity,
                offset_y,
                hidden_offset_y,
                exit_animation: SnackbarTheme::default().exit_animation,
            };
            state.is_showing = true;
            state.is_dismissing = false;
            state.current = Some(presentation.clone());
            presentation
        };

        // Build and display the snackbar view
        let view = self.build_snackbar_view(snackbar, position, presentation.clone());
        self.state.borrow().handler.set(view);

        let opacity = presentation.opacity.clone();
        let offset_y = presentation.offset_y.clone();
        spawn_local(async move {
            opacity.set(1.0);
            offset_y.set(0.0);
        })
        .detach();

        // Schedule auto-dismiss
        let manager = self.clone();
        spawn_local(async move {
            native_executor::sleep(duration).await;
            manager.dismiss_presentation(presentation.id);
        })
        .detach();
    }

    fn dismiss_presentation(&self, id: u64) {
        let presentation = {
            let mut state = self.state.borrow_mut();
            let Some(presentation) = state.current.clone() else {
                return;
            };
            if !state.is_showing || state.is_dismissing || presentation.id != id {
                return;
            }
            state.is_dismissing = true;
            presentation
        };

        presentation.opacity.set(0.0);
        presentation.offset_y.set(presentation.hidden_offset_y);

        let manager = self.clone();
        spawn_local(async move {
            native_executor::sleep(presentation.exit_animation.duration()).await;
            manager.finish_dismissal(id);
        })
        .detach();
    }

    fn finish_dismissal(&self, id: u64) {
        let next = {
            let mut state = self.state.borrow_mut();
            let Some(current) = state.current.as_ref() else {
                return;
            };
            if current.id != id {
                return;
            }
            state.is_showing = false;
            state.is_dismissing = false;
            state.current = None;
            state.queue.pop_front()
        };

        self.state.borrow().handler.set(());

        if let Some(next_snackbar) = next {
            self.begin_show(next_snackbar);
        }
    }

    fn build_snackbar_view(
        &self,
        snackbar: Snackbar,
        position: SnackbarPosition,
        presentation: SnackbarPresentation,
    ) -> impl View {
        SnackbarView {
            snackbar,
            manager: self.clone(),
            position,
            presentation,
        }
    }
}

struct SnackbarView {
    snackbar: Snackbar,
    manager: SnackbarManager,
    position: SnackbarPosition,
    presentation: SnackbarPresentation,
}

impl SnackbarView {
    fn build_content(
        snackbar: Snackbar,
        manager: SnackbarManager,
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

        // Add action button if present
        if let Some(action) = snackbar.action {
            let action_label = action.label.clone();
            let captured_env = snackbar.captured_env.clone();

            AnyView::new(
                hstack((
                    label_view,
                    spacer(),
                    button(
                        text(action_label)
                            .font(theme.action_label_font.clone())
                            .color(theme.action_label_color.clone()),
                    )
                    .style(ButtonStyle::Borderless)
                    .action(move |env: waterui_core::Environment| {
                        // Execute action handler with the live view environment.
                        let () = action.handler.call(&captured_env.layered_on(&env));
                        manager.dismiss();
                    }),
                ))
                .spacing(theme.content_spacing),
            )
        } else {
            AnyView::new(label_view)
        }
    }
}

impl View for SnackbarView {
    fn body(self, env: &waterui_core::Environment) -> impl View {
        let theme = env.get::<SnackbarTheme>().cloned().unwrap_or_default();
        let Self {
            snackbar,
            manager,
            position,
            presentation,
        } = self;

        // Build content: Label (icon + message) + optional action button
        let content = Self::build_content(snackbar, manager, &theme);

        let enter_animation = theme.enter_animation.clone();
        let shadow = Shadow::new(
            theme.shadow_color.clone(),
            Vector::new(0.0, theme.shadow_offset_y),
            theme.shadow_radius,
        );

        // Styled container with blur background and rounded corners
        Frame::new(
            content
                .padding_with(theme.content_padding)
                .height(theme.single_line_min_height)
                .background(
                    RoundedRectangle::new(theme.clip_radius).fill(theme.container_color.clone()),
                )
                .border_with(
                    crate::border::Border::new(
                        theme.container_color.clone().with_opacity(0.0),
                        0.0,
                    )
                    .corner_radius(theme.corner_radius),
                )
                .shadow(shadow)
                .opacity(
                    presentation
                        .opacity
                        .clone()
                        .with_animation(enter_animation.clone()),
                )
                .offset(0.0, presentation.offset_y.with_animation(enter_animation)),
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
