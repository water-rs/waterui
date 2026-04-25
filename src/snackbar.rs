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
use waterui_text::text::text;

use crate::AnyView;
use crate::ViewExt;
use crate::background::Material;
use crate::component::Label;
use crate::shape::RoundedRectangle;
use crate::style::Shadow;

/// Maximum number of snackbars that can be queued.
const MAX_QUEUE_SIZE: usize = 10;

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
    ///     .action("Undo")
    ///     .handler(|State(items): State<Items>| items.restore())
    ///     .state(&items)
    /// ```
    #[must_use]
    pub fn action(self, label: impl Into<Str>) -> SnackbarActionBuilder {
        SnackbarActionBuilder {
            snackbar: self,
            label: label.into(),
        }
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
            state.is_showing = true;
            drop(state);
            self.show_immediate(snackbar);
        }
    }

    /// Dismisses the current snackbar immediately.
    pub fn dismiss(&self) {
        self.dismiss_and_show_next();
    }

    fn show_immediate(&self, snackbar: Snackbar) {
        let duration = snackbar.duration;
        let position = snackbar.position;

        // Build and display the snackbar view
        let view = self.build_snackbar_view(snackbar, position);
        self.state.borrow().handler.set(view);

        // Schedule auto-dismiss
        let manager = self.clone();
        spawn_local(async move {
            native_executor::sleep(duration).await;
            manager.dismiss_and_show_next();
        })
        .detach();
    }

    fn dismiss_and_show_next(&self) {
        let next = {
            let mut state = self.state.borrow_mut();
            if !state.is_showing {
                return; // Already dismissed
            }
            state.is_showing = false;
            state.queue.pop_front()
        };

        // Clear the view
        self.state.borrow().handler.set(());

        // Show next if available (with small delay for visual separation)
        if let Some(next_snackbar) = next {
            let manager = self.clone();
            spawn_local(async move {
                native_executor::sleep(Duration::from_millis(200)).await;
                manager.state.borrow_mut().is_showing = true;
                manager.show_immediate(next_snackbar);
            })
            .detach();
        }
    }

    fn build_snackbar_view(&self, snackbar: Snackbar, position: SnackbarPosition) -> impl View {
        let manager = self.clone();

        // Build content: Label (icon + message) + optional action button
        let content = Self::build_content(snackbar, manager);

        // Animation bindings
        let opacity = Binding::f32(0.0);
        let offset_y = Binding::container(position.initial_offset_y());

        let opacity_clone = opacity.clone();
        let offset_clone = offset_y.clone();

        // Styled container with blur background and rounded corners
        Frame::new(
            content
                .padding_with(EdgeInsets::symmetric(12.0, 16.0))
                .background(Material::Regular)
                .clip(RoundedRectangle::new(0.1))
                .shadow(Shadow::default())
                .opacity(opacity.animated())
                .offset(0.0, offset_y.animated()),
        )
        .alignment(position.to_alignment())
        .padding_with(EdgeInsets::all(16.0)) // Safe area inset
        .on_appear(move || {
            // Trigger entrance animation
            opacity_clone.set(1.0);
            offset_clone.set(0.0);
        })
    }

    fn build_content(snackbar: Snackbar, manager: Self) -> AnyView {
        // Message with optional icon using Label component
        let label_view = if let Some(icon) = snackbar.icon {
            Label::new(snackbar.message).icon(icon)
        } else {
            Label::new(snackbar.message)
        };

        // Add action button if present
        if let Some(action) = snackbar.action {
            let action_label = action.label.clone();
            let captured_env = snackbar.captured_env.clone();

            AnyView::new(
                hstack((
                    label_view,
                    spacer(),
                    button(text(action_label).bold())
                        .style(ButtonStyle::Borderless)
                        .action(move |env: waterui_core::Environment| {
                            // Execute action handler with the live view environment.
                            let () = action.handler.call(&captured_env.layered_on(&env));
                            manager.dismiss();
                        }),
                ))
                .spacing(12.0),
            )
        } else {
            AnyView::new(label_view)
        }
    }
}

impl Default for SnackbarManager {
    fn default() -> Self {
        Self::new().0
    }
}

// ============================================================================
// Snackbar Action Builder
// ============================================================================

/// Builder for creating snackbar actions with captured state.
#[derive(Debug)]
pub struct SnackbarActionBuilder {
    snackbar: Snackbar,
    label: Str,
}

impl SnackbarActionBuilder {
    /// Sets the action handler (no state).
    #[must_use]
    pub fn handler<Args>(mut self, handler: impl Handler<Args, ()> + 'static) -> Snackbar {
        self.snackbar.action = Some(SnackbarAction {
            label: self.label,
            handler: shared_action(handler),
        });
        self.snackbar
    }
}
